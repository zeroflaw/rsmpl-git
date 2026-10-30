//! Nested same-callee fold sweep (WI-1c): `Box::new(Box::new(x))` → `x`.
//!
//! N ≥ 2 pairs where every minus is the same 1-arg callee applied ≥ 1
//! times around a plus side that is exactly the full-strip fixpoint
//! (peeling that callee repeatedly from the minus reaches the plus).
//! Emits one `# allow=many` hunk (`- C(E0)` / `+ E0`, inferred hole,
//! SPEC 15.8) with `# repeat` when some pair peeled ≥ 2 layers — the
//! engine folds nested matches bottom-up (T20/T126), so one hunk strips
//! every depth; flat single-unwrap sweeps emit the plain hunk (repeat
//! would be a no-op there). `let`-init sites unwrap to their init call
//! (T449 precedent — the emission stays a bare pattern and the engine
//! rewrites the call sub-expression in place).
//!
//! Soundness gates (all conservative, all veto to ordinary):
//! same rendered callee every pair (bare `foo` or qualified `Box::new`;
//! leading `::`, qself, and generic args decline — no engine spelling
//! or unexpressible template); full-strip per pair (a partial strip
//! under `# repeat` would over-strip the untouched layers); the plus
//! side contains no same-callee call (repeat would rewrite inside the
//! replacement); `&`-led sides decline (general-expr territory);
//! all-byte-identical classes stay for the tighter verbatim sweep (T422
//! precedent). Totality is the generic engine-count check
//! (`template_repeat_holds` on the minus template — the `# repeat`
//! probe counts outermost sites exactly like the real sweep; nested
//! matches are folds, not separate sites).

use super::{parse_call_expr, unique_rule};
use quote::ToTokens;
use syn::visit::Visit;

/// N-site (N ≥ 2) nested same-callee full-strip fold: every pair peels
/// through the shared 1-arg callee to its plus side. Emits one
/// `# allow=many` hunk with `# repeat` iff some pair nested ≥ 2 deep.
pub(crate) fn try_repeat_fold_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    // All-byte-identical classes stay for the tighter verbatim sweep
    // (T422 precedent — verbatim wins identical runs byte-for-byte).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut callee: Option<String> = None;
    let mut nested = false;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        if m.trim_start().starts_with('&') || p.trim_start().starts_with('&') {
            return None;
        }
        let (c, layers) = peel_layers(m, p)?;
        if callee.as_ref().is_some_and(|have| *have != c) {
            return None;
        }
        nested |= layers >= 2;
        callee = Some(c);
    }
    let callee = callee?;
    let lit = super::shape::literal_path_last(&callee);
    let minus_template = format!("{lit}(E0)");
    if !crate::totality::template_repeat_holds(path, file_old, &minus_template, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("repeat_{callee}_fold"));
    let mut hunk = crate::emission_header(path, &rule, changed);
    if nested {
        hunk.push_str("# repeat\n");
    }
    hunk.push_str(&format!("- {minus_template}\n+ E0\n"));
    Some(hunk)
}

/// Peel the 1-arg `callee` spine off `minus_line` (T449 `let`-unwrap)
/// to its fixpoint; return (rendered callee, layers) when the fixpoint
/// token-equals `plus_line` (full strip, ≥ 1 layer) and the fixpoint
/// holds no same-callee call (repeat would keep folding past the
/// diff's post-state). Partial strips (an intermediate layer equals the
/// plus) stay ordinary — a `# repeat` emission would over-strip them,
/// and mixing plain/repeat emissions across depths is unsound.
/// `None` on any shape divergence.
fn peel_layers(minus_line: &str, plus_line: &str) -> Option<(String, usize)> {
    let mut current = parse_call_expr(minus_line)?;
    let want_expr = parse_value_expr(plus_line)?;
    let callee = callee_text(&current)?;
    let mut layers = 0usize;
    loop {
        let inner = match &current {
            syn::Expr::Call(call) if call.args.len() == 1 => match callee_text(&current) {
                Some(c) if c == callee => call.args[0].clone(),
                _ => break,
            },
            _ => break,
        };
        current = inner;
        layers += 1;
    }
    if layers == 0 {
        return None;
    }
    if current.to_token_stream().to_string() != want_expr.to_token_stream().to_string() {
        return None;
    }
    if contains_callee(&current, &callee) {
        return None;
    }
    Some((callee, layers))
}

/// Value side of a fold pair: a `let`-statement side contributes its
/// init (T449 precedent — the emission rewrites the call
/// sub-expression in place, bindings preserved), anything else parses
/// as a plain expression. `let-else` and init-less `let` decline.
fn parse_value_expr(line: &str) -> Option<syn::Expr> {
    let text = line.trim();
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(text) {
        if let Some(init) = local.init {
            if init.diverge.is_none() {
                return Some(*init.expr);
            }
        }
    }
    let t = text.strip_suffix(';').map(str::trim_end).unwrap_or(text);
    syn::parse_str::<syn::Expr>(t).ok()
}

/// Rendered callee path (`foo` or `Box::new`); `None` for method calls,
/// qself, leading-`::`, or any generic segment (no engine spelling or
/// unexpressible template — same gates as the qualified sweep).
fn callee_text(expr: &syn::Expr) -> Option<String> {
    if let syn::Expr::Call(call) = expr {
        if call.args.len() != 1 {
            return None;
        }
        if let syn::Expr::Path(p) = call.func.as_ref() {
            if p.qself.is_some() || p.path.leading_colon.is_some() {
                return None;
            }
            if !p
                .path
                .segments
                .iter()
                .all(|s| matches!(s.arguments, syn::PathArguments::None))
            {
                return None;
            }
            let segs: Vec<String> = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if segs.is_empty() || segs.iter().any(|s| super::ident_len(s) != Some(s.len())) {
                return None;
            }
            return Some(segs.join("::"));
        }
    }
    None
}

/// True when `expr` contains a call headed by `callee` anywhere inside
/// (a repeat rewrite would keep folding the replacement — veto).
fn contains_callee(expr: &syn::Expr, callee: &str) -> bool {
    struct F<'a> {
        callee: &'a str,
        hit: bool,
    }
    impl<'ast> Visit<'ast> for F<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = call.func.as_ref() {
                    let segs: Vec<String> = p
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect();
                    if segs.join("::") == self.callee {
                        self.hit = true;
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut f = F { callee, hit: false };
    f.visit_expr(expr);
    f.hit
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    fn wir(body: &str) -> (String, HashMap<String, String>) {
        let pre = "fn a(x: i32) -> i32 {\n    let v = Box::new(Box::new(x));\n    v\n}\nfn b(y: i32) -> i32 {\n    let w = Box::new(Box::new(Box::new(y)));\n    w\n}\n";
        let raw = section("f.rs", body);
        (raw, pre_map(&[("f.rs", pre)]))
    }

    #[test]
    fn repeat_fold_mixed_depths_fires() {
        // Fixture-16 shape: depth-2 and depth-3 sites, `let`-wrapped,
        // full strip both — one `# repeat` hunk.
        let (raw, pre) = wir(
            "@@ -1,8 +1,8 @@\n fn a(x: i32) -> i32 {\n-    let v = Box::new(Box::new(x));\n+    let v = x;\n fn b(y: i32) -> i32 {\n-    let w = Box::new(Box::new(Box::new(y)));\n+    let w = y;\n }\n",
        );
        let out = infer_semantic_with_pre(&raw, &pre);
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("# repeat\n"), "got: {out}");
        assert!(out.contains("- Box::new(E0)\n"), "got: {out}");
        assert!(out.contains("+ E0\n"), "got: {out}");
    }

    #[test]
    fn repeat_fold_bare_callee_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(foo(first_argument));\n-    foo(foo(foo(second_argument)));\n+    first_argument;\n+    second_argument;\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    foo(foo(first_argument));\n    foo(foo(foo(second_argument)));\n}\n",
        )]);
        let out = infer_semantic_with_pre(&raw, &pre);
        validated_live(&out, &pre);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ E0\n"), "got: {out}");
        assert!(out.contains("# repeat\n"), "got: {out}");
    }

    #[test]
    fn repeat_fold_flat_unwraps_emit_plain_hunk() {
        // No nesting anywhere: the plain hunk is exact, `# repeat`
        // would be a no-op — still sound, still one hunk.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let av = Box::new(first_argument);\n-    let bv = Box::new(second_argument);\n+    let av = first_argument;\n+    let bv = second_argument;\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let av = Box::new(first_argument);\n    let bv = Box::new(second_argument);\n}\n",
        )]);
        let out = infer_semantic_with_pre(&raw, &pre);
        validated_live(&out, &pre);
        assert!(out.contains("- Box::new(E0)\n"), "got: {out}");
        assert!(!out.contains("# repeat\n"), "got: {out}");
    }

    #[test]
    fn repeat_fold_single_site_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let v = Box::new(Box::new(first_argument));\n+    let v = first_argument;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn repeat_fold_divergent_callee_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let av = Box::new(Box::new(first_argument));\n-    let bv = Rc::new(Rc::new(second_argument));\n+    let av = first_argument;\n+    let bv = second_argument;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn repeat_fold_partial_strip_stays_passthrough() {
        // `Box::new(Box::new(x))` → `Box::new(x)` under `# repeat`
        // would over-strip to `x` — decline (no partial-strip shape).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let av = Box::new(Box::new(first_argument));\n-    let bv = Box::new(Box::new(second_argument));\n+    let av = Box::new(first_argument);\n+    let bv = Box::new(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn repeat_fold_plus_holds_callee_stays_passthrough() {
        // The plus side still contains the callee — repeat would keep
        // folding past the diff's post-state.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let av = Box::new(wrap(Box::new(first_argument)));\n-    let bv = Box::new(wrap(Box::new(second_argument)));\n+    let av = wrap(Box::new(first_argument));\n+    let bv = wrap(Box::new(second_argument));\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn repeat_fold_extra_site_vetoes() {
        let (raw, _) = wir(
            "@@ -1,8 +1,8 @@\n fn a(x: i32) -> i32 {\n-    let v = Box::new(Box::new(x));\n+    let v = x;\n fn b(y: i32) -> i32 {\n-    let w = Box::new(Box::new(Box::new(y)));\n+    let w = y;\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn a(x: i32) -> i32 {\n    let v = Box::new(Box::new(x));\n    v\n}\nfn b(y: i32) -> i32 {\n    let w = Box::new(Box::new(Box::new(y)));\n    w\n}\nfn c(z: i32) -> i32 {\n    Box::new(z)\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn repeat_fold_amp_led_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &Box::new(Box::new(first_argument));\n-    &Box::new(Box::new(second_argument));\n+    &first_argument;\n+    &second_argument;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn repeat_fold_turbofish_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let av = Box::<u8>::new(Box::<u8>::new(first_argument));\n-    let bv = Box::<u8>::new(Box::<u8>::new(second_argument));\n+    let av = first_argument;\n+    let bv = second_argument;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn repeat_fold_leading_colon_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let av = ::Box::new(::Box::new(first_argument));\n-    let bv = ::Box::new(::Box::new(second_argument));\n+    let av = first_argument;\n+    let bv = second_argument;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn repeat_fold_all_identical_stays_for_verbatim() {
        // Two byte-identical pairs: verbatim owns them (T422 precedent) —
        // repeat must not claim the run even though the spine qualifies.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    sink(Box::new(Box::new(shared_argument)));\n-    sink(Box::new(Box::new(shared_argument)));\n+    sink(shared_argument);\n+    sink(shared_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    sink(Box::new(Box::new(shared_argument)));\n    sink(Box::new(Box::new(shared_argument)));\n}\n",
        )]);
        let out = infer_semantic_with_pre(&raw, &pre);
        assert!(
            !out.contains("(E0)"),
            "repeat must not claim identical runs, got: {out}"
        );
    }

    #[test]
    fn repeat_fold_uppercase_callee_backticked() {
        // WI-7-upper: an uppercase last segment infers as an `expr`
        // hole — backtick it (probed literal spelling).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let av = Crate::Wrap(Crate::Wrap(first_argument));\n-    let bv = Crate::Wrap(Crate::Wrap(second_argument));\n+    let av = first_argument;\n+    let bv = second_argument;\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let av = Crate::Wrap(Crate::Wrap(first_argument));\n    let bv = Crate::Wrap(Crate::Wrap(second_argument));\n}\n",
        )]);
        let out = infer_semantic_with_pre(&raw, &pre);
        validated_live(&out, &pre);
        assert!(out.contains("- Crate::`Wrap`(E0)\n"), "got: {out}");
    }

    fn infer_semantic_with_pre(raw: &str, pre: &HashMap<String, String>) -> String {
        let owned = raw.to_string();
        super::super::infer_semantic_with_pre_images(std::slice::from_ref(&owned), pre)
            .expect("repeat fold must compress")
    }
}
