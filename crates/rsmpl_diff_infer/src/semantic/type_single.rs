//! N=1 bare-type literal single (WI-2 slice 1, type arm).
//!
//! A single line pair differing in exactly one type subterm (fn-param
//! types like `117-type-rewrite-single`, `let` ascriptions, or bare
//! type lines) compresses to the literal bare-type pair
//! (`- Vec<i32>` / `+ Box<[i32]>`, no holes — fewest-holes-wins with
//! zero holes). The engine `match_type` already matches type subterms
//! anywhere, so the literal form applies to the same site the authored
//! holed sweep (`- Vec<T>` / `+ Box<[T]>`) claims; D17 single-site
//! equality unifies them exactly as it did for the verbatim family.
//! Totality is the engine-count check (`template_type_holds` at n=1 —
//! an untouched twin vetoes) plus the T361 scratch-apply byte-compare
//! (`single_reproduces`). Multi-type deltas, macro interiors (the
//! engine never matches inside them, so the count probe vetoes), and
//! unparseable pairs decline. Detector name `type_sweep` (same family
//! as the N>=2 sweep; `guard_for("type_sweep")` stays `Strict` for
//! multis — the singles block prices its own precision allowance).

use super::unique_rule;

/// N=1 bare-type literal single: exactly one pair/site, pre-image
/// required (same wide-minus doctrine as the slot-sweep N=1). The
/// delta region is the longest-common-prefix/suffix middle of the two
/// trimmed lines; both middles must re-parse as `syn::Type`.
pub(crate) fn try_type_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return super::prov::Counted::Miss;
    }
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    let m = minuses[0].trim();
    let p = pluses[0].trim();
    if m.is_empty() || p.is_empty() || m == p {
        return super::prov::Counted::Miss;
    }
    if m.contains('\n') || p.contains('\n') {
        return super::prov::Counted::Miss;
    }
    let Some((minus_ty, plus_ty)) = single_type_delta(m, p) else {
        return super::prov::Counted::Miss;
    };
    // Type-position honesty (WI-3 fixture-64 precedent): the delta must
    // be a `syn::Type` node *in its own line*, not merely text that
    // parses as a type in isolation. A call-callee path with turbofish
    // (`old_deserializer_name::<u8>`) parses as `Type::Path`, but the
    // line parses as a call whose callee is an `ExprPath` — no `Type`
    // node carries that spelling, so the call arms (verbatim and
    // friends, which keep the args in the pattern) stay owners and this
    // arm declines instead of emitting an arg-dropping "type" rewrite.
    if !line_type_spellings(m).contains(&minus_ty) || !line_type_spellings(p).contains(&plus_ty) {
        return super::prov::Counted::Miss;
    }
    // The emitted texts must re-parse as types (the extractor already
    // checked, but the emission is what the engine reads — never emit
    // a guess).
    if syn::parse_str::<syn::Type>(&minus_ty).is_err()
        || syn::parse_str::<syn::Type>(&plus_ty).is_err()
    {
        return super::prov::Counted::Miss;
    }
    // Exactly one type subterm in the whole file (an untouched twin
    // vetoes — same posture as the N>=2 sweep).
    match crate::totality::template_type_counts(path, Some(pre), &minus_ty) {
        Some(1) => {}
        Some(found) => return super::prov::Counted::Totality { found, want: 1 },
        None => return super::prov::Counted::Miss,
    }
    // T361: the ranged allow-one emission must real-apply byte-identical
    // to the native single-line swap.
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_ty, &plus_ty,
    ) {
        return super::prov::Counted::Miss;
    }
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, "type_sweep");
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_ty}\n+ {plus_ty}\n"));
    // Defensive: the emission must parse (the probes above already
    // applied equivalent spellings, but the hunk header is new).
    if rsmpl_patch::parse_patch(&hunk).is_err() {
        used_rules.truncate(checkpoint);
        return super::prov::Counted::Miss;
    }
    super::prov::Counted::Fire(hunk)
}

/// The single differing middle of two trimmed lines when both middles
/// parse as `syn::Type`. Longest common prefix/suffix on chars (column
/// safety for non-ASCII); the frame around the middle is byte-identical
/// by construction. Multi-delta lines yield a middle that spans both
/// deltas plus the equal text between them, which never parses as one
/// `Type` — so exactly-one-type-delta holds whenever this returns
/// `Some`.
fn single_type_delta(minus: &str, plus: &str) -> Option<(String, String)> {
    let m: Vec<char> = minus.chars().collect();
    let p: Vec<char> = plus.chars().collect();
    let mut pre = 0usize;
    while pre < m.len() && pre < p.len() && m[pre] == p[pre] {
        pre += 1;
    }
    let mut max_suf = 0usize;
    while max_suf < m.len() - pre
        && max_suf < p.len() - pre
        && m[m.len() - 1 - max_suf] == p[p.len() - 1 - max_suf]
    {
        max_suf += 1;
    }
    // Bracket backtrack: the shared suffix may start inside the delta
    // (117: both sides end `>` before `) -> i32 {`, so the maximal
    // suffix leaves `Vec<i32` / `Box<[i32]` which never parse). Scan
    // from the longest shared suffix down and take the first split
    // whose middles both parse as one `Type` — the maximal shared
    // frame with a type delta.
    let mut suf = max_suf;
    loop {
        let m_mid: String = m[pre..m.len() - suf].iter().collect();
        let p_mid: String = p[pre..p.len() - suf].iter().collect();
        let m_mid = m_mid.trim().to_string();
        let p_mid = p_mid.trim().to_string();
        if !m_mid.is_empty()
            && !p_mid.is_empty()
            && m_mid != p_mid
            && !m_mid.contains('\n')
            && !p_mid.contains('\n')
            && !(is_bare_ident(&m_mid) && is_bare_ident(&p_mid))
            && syn::parse_str::<syn::Type>(&m_mid).is_ok()
            && syn::parse_str::<syn::Type>(&p_mid).is_ok()
        {
            return Some((m_mid, p_mid));
        }
        if suf == 0 {
            return None;
        }
        suf -= 1;
    }
}

/// Every `syn::Type` node's original spelling in one trimmed line:
/// the line parsed as `Stmt`, as `ItemFn`/`ImplItemFn` (via an appended
/// `}` for header lines), and as `Expr`, with each node's span sliced
/// back out of the line (single-line spans only — the probes that need
/// multi-line never reach this single-line detector). A callee path
/// with turbofish contributes only its type *arguments* (`u8`), never
/// the whole callee spelling — which is exactly the type-vs-expression
/// position distinction the honesty gate needs.
fn line_type_spellings(line: &str) -> Vec<String> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;
    struct Collector<'a> {
        tys: Vec<&'a syn::Type>,
    }
    impl<'ast> Visit<'ast> for Collector<'ast> {
        fn visit_type(&mut self, t: &'ast syn::Type) {
            self.tys.push(t);
            syn::visit::visit_type(self, t);
        }
    }
    fn slice(line: &str, span: proc_macro2::Span) -> Option<String> {
        if span.start().line != 1 || span.end().line != 1 {
            return None;
        }
        let (s, e) = (
            rsmpl_pattern::char_col_to_byte(line, span.start().column),
            rsmpl_pattern::char_col_to_byte(line, span.end().column),
        );
        if s >= e || e > line.len() || !line.is_char_boundary(s) || !line.is_char_boundary(e) {
            return None;
        }
        Some(line[s..e].to_string())
    }
    let mut out = Vec::new();
    let mut harvest = |tys: Vec<&syn::Type>| {
        for t in tys {
            if let Some(s) = slice(line, t.span()) {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    };
    if let Ok(stmt) = syn::parse_str::<syn::Stmt>(line) {
        let mut c = Collector { tys: Vec::new() };
        c.visit_stmt(&stmt);
        harvest(c.tys);
    }
    let headed = format!("{line} }}");
    if let Ok(item) = syn::parse_str::<syn::ItemFn>(&headed) {
        let mut c = Collector { tys: Vec::new() };
        c.visit_item_fn(&item);
        harvest(c.tys);
    }
    if let Ok(item) = syn::parse_str::<syn::ImplItemFn>(&headed) {
        let mut c = Collector { tys: Vec::new() };
        c.visit_impl_item_fn(&item);
        harvest(c.tys);
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(line) {
        let mut c = Collector { tys: Vec::new() };
        c.visit_expr(&expr);
        harvest(c.tys);
    }
    out
}

/// A plain ident with no generics, path segments, or reference wrappers
/// (`foo`, `Old`) — an item-name delta, never a type delta.
fn is_bare_ident(s: &str) -> bool {
    let mut chars = s.char_indices();
    let Some((_, first)) = chars.next() else {
        return false;
    };
    if first != '_' && !first.is_ascii_alphabetic() {
        return false;
    }
    let mut len = first.len_utf8();
    for (i, c) in chars {
        if c == '_' || c.is_ascii_alphanumeric() {
            len = i + c.len_utf8();
        } else {
            return false;
        }
    }
    len == s.len()
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn single(
        minus: &str,
        plus: &str,
        pre: &str,
        changed: &[usize],
    ) -> crate::semantic::prov::Counted<String> {
        let mut used = Vec::new();
        super::try_type_single(&[minus], &[plus], "f.rs", Some(pre), changed, &mut used)
    }

    const PRE_117: &str = "fn sum(values: Vec<i32>) -> i32 {\n    values.iter().sum()\n}\n\nfn count(items: &[u8]) -> usize {\n    items.len()\n}\n";

    #[test]
    fn fn_param_type_single_fires_and_round_trips() {
        // The 117 shape: one fn-param type delta inside a header line.
        let out = single(
            "fn sum(values: Vec<i32>) -> i32 {",
            "fn sum(values: Box<[i32]>) -> i32 {",
            PRE_117,
            &[1],
        )
        .unwrap_fire("117-shaped single must fire");
        assert!(out.contains("- Vec<i32>\n"), "got:\n{out}");
        assert!(out.contains("+ Box<[i32]>\n"), "got:\n{out}");
        assert!(!out.contains("# allow=many"), "ranged single: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", PRE_117)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("fn sum(values: Box<[i32]>) -> i32 {"));
        assert!(files["f.rs"].contains("fn count(items: &[u8]) -> usize {"));
    }

    #[test]
    fn return_type_single_fires() {
        // Return-position delta: the middle is the return type.
        let pre = "fn f() -> Vec<u32> {\n    vec![]\n}\nfn g() -> u32 {\n    0\n}\n";
        let out = single("fn f() -> Vec<u32> {", "fn f() -> Box<[u32]> {", pre, &[1])
            .unwrap_fire("return-type single must fire");
        assert!(out.contains("- Vec<u32>\n"), "got:\n{out}");
        assert!(out.contains("+ Box<[u32]>\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", pre)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("fn f() -> Box<[u32]> {"));
    }

    #[test]
    fn let_ascription_type_single_fires() {
        let pre = "fn m() {\n    let alpha: Vec<u32> = compute_first(input);\n    let beta: u32 = done();\n}\n";
        let out = single(
            "    let alpha: Vec<u32> = compute_first(input);",
            "    let alpha: Box<[u32]> = compute_first(input);",
            pre,
            &[2],
        )
        .unwrap_fire("let-ascription single must fire");
        assert!(out.contains("- Vec<u32>\n"), "got:\n{out}");
        assert!(out.contains("+ Box<[u32]>\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", pre)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("let alpha: Box<[u32]> = compute_first(input);"));
    }

    #[test]
    fn twin_type_elsewhere_vetoes() {
        // Totality: a second `Vec<i32>` param elsewhere would be
        // silently rewritten — the single vetoes.
        let pre = "fn sum(values: Vec<i32>) -> i32 {\n    values.iter().sum()\n}\nfn other(v: Vec<i32>) -> usize {\n    v.len()\n}\n";
        assert!(matches!(
            single(
                "fn sum(values: Vec<i32>) -> i32 {",
                "fn sum(values: Box<[i32]>) -> i32 {",
                pre,
                &[1],
            ),
            crate::semantic::prov::Counted::Totality { found: 2, want: 1 }
        ));
    }

    #[test]
    fn two_type_delta_declines() {
        // Two differing types in one line never yield one `Type` middle.
        let pre = "fn m(a: Vec<u32>, b: Vec<String>) -> i32 {\n    0\n}\n";
        assert!(single(
            "fn m(a: Vec<u32>, b: Vec<String>) -> i32 {",
            "fn m(a: Box<[u32]>, b: Box<[String]>) -> i32 {",
            pre,
            &[1],
        )
        .is_miss());
    }

    #[test]
    fn bare_ident_rename_declines() {
        // Item-name deltas belong to the rename arms, never here.
        let pre = "fn foo(x: i32) {\n    x\n}\n";
        assert!(single("fn foo(x: i32) {", "fn bar(x: i32) {", pre, &[1]).is_miss());
    }

    #[test]
    fn non_type_delta_declines() {
        // A body-token delta is not a type.
        let pre = "fn m() {\n    foo(1);\n}\n";
        assert!(single("    foo(1);", "    foo(2);", pre, &[2]).is_miss());
    }

    #[test]
    fn no_pre_image_vetoes() {
        let mut used = Vec::new();
        assert!(super::try_type_single(
            &["fn sum(values: Vec<i32>) -> i32 {"],
            &["fn sum(values: Box<[i32]>) -> i32 {"],
            "f.rs",
            None,
            &[1],
            &mut used,
        )
        .is_miss());
        assert!(used.is_empty());
    }

    #[test]
    fn misaligned_site_vetoes() {
        // The claimed line must equal the pre-image line (the T361 probe
        // compares against it).
        assert!(single(
            "fn sum(values: Vec<i32>) -> i32 {",
            "fn sum(values: Box<[i32]>) -> i32 {",
            PRE_117,
            &[5],
        )
        .is_miss());
    }
    #[test]
    fn turbofish_callee_delta_declines() {
        // WI-3 fixture-64 precedent: a call-callee path with turbofish
        // parses as `Type::Path` in isolation, but the line is a call —
        // no `Type` node carries the callee spelling, so the call arms
        // (which keep the args in the pattern) stay owners.
        let pre = "fn m() {\n    old_deserializer_name::<u8>(first_argument);\n}\n";
        assert!(single(
            "    old_deserializer_name::<u8>(first_argument);",
            "    new_deserializer_name::<u8>(first_argument);",
            pre,
            &[2],
        )
        .is_miss());
    }

    #[test]
    fn string_param_single_fires() {
        // Fixture-72 beta shape: `String` as the holed position must
        // behave exactly like the `i32` shape (no special-casing of
        // path-like inner types).
        let pre = "fn m() {\n    let beta_value: Vec<String> = compute_second(other);\n    let gamma_count: u32 = done();\n}\n";
        let out = single(
            "    let beta_value: Vec<String> = compute_second(other);",
            "    let beta_value: Box<[String]> = compute_second(other);",
            pre,
            &[2],
        )
        .unwrap_fire("Vec<String> single must fire");
        assert!(out.contains("- Vec<String>\n"), "got:\n{out}");
        assert!(out.contains("+ Box<[String]>\n"), "got:\n{out}");
    }

    #[test]
    fn string_inner_with_extra_vec_vetoes() {
        // Fixture-72 beta posture: `String` infers as a type hole, so
        // `Vec<String>` matches every `Vec<..>` site (here `Vec<u8>`
        // too) and the count probe vetoes — while the `u32` twin below
        // stays a literal and fires. Same shape, different verdicts,
        // pinned.
        let pre = "fn m() {\n    let beta_value: Vec<String> = compute_second(other);\n    let untouched_guard: Vec<u8> = compute_fourth(more);\n}\n";
        assert!(matches!(
            single(
                "    let beta_value: Vec<String> = compute_second(other);",
                "    let beta_value: Box<[String]> = compute_second(other);",
                pre,
                &[2],
            ),
            crate::semantic::prov::Counted::Totality { found: 2, want: 1 }
        ));
        let pre2 = "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let untouched_guard: Vec<u8> = compute_fourth(more);\n}\n";
        single(
            "    let alpha_value: Vec<u32> = compute_first(input);",
            "    let alpha_value: Box<[u32]> = compute_first(input);",
            pre2,
            &[2],
        )
        .unwrap_fire("primitive-inner literal still fires beside an extra Vec");
    }

    #[test]
    fn whole_pipeline_117_shape_compresses() {
        // End to end: the native 117 hunk compresses through the shared
        // chain (the new arm runs before the literal verbatim floor).
        let raw = "diff --git a/f.rs b/f.rs\n@@ -1,1 +1,1 @@\n-fn sum(values: Vec<i32>) -> i32 {\n+fn sum(values: Box<[i32]>) -> i32 {\n";
        let pre = pre_map(&[("f.rs", PRE_117)]);
        let out = infer_semantic_with_pre_images(&[raw.to_string()], &pre)
            .expect("117 single must compress");
        assert!(out.contains("- Vec<i32>\n"), "got:\n{out}");
        assert!(out.contains("+ Box<[i32]>\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre;
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("Box<[i32]>"));
    }
}
