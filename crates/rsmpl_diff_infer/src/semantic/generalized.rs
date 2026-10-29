//! Generalized anti-unification sweeps (WI-4a first slice, WI-4b type slice).
//!
//! Thin detectors over [`super::antiunify`]: `try_let_sweep` for let
//! type-addition (`let PAT = INIT;` → `let PAT: T = INIT;` with
//! per-site preserved `PAT`/`INIT` bound as inferred `P0`/`E0` and a
//! uniform concrete `T`), plus `try_type_sweep` for type rewrites
//! (`Vec<T>` → `Box<[T]>` with hetero `T` bound as inferred `T0…`).
//! Signature evolution (WI-4c) lands here next.

use super::antiunify::{antiunify_let_type_add, antiunify_types, TypedSite};
use super::unique_rule;
use quote::ToTokens;
use syn::spanned::Spanned;

/// N-site (N ≥ 2) let type-addition sweep: every pair adds the same
/// concrete type ascription to a preserved pattern and initializer,
/// both varying across sites (hence the `P0`/`E0` holes). Emits one
/// `# allow=many` hunk (tight original range, SPEC D.5) with diet
/// inferred holes — no `# pat`/`# expr` declarations (SPEC 15.8).
pub(crate) fn try_let_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let ty = antiunify_let_type_add(minuses, pluses)?;
    if !crate::totality::let_untyped_holds(file_old, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_type_add");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- let P0 = E0;\n+ let P0: {ty} = E0;\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) type rewrite sweep: every pair carries the same type
/// delta with per-site varying subtrees bound as inferred `T0…` holes
/// (e.g. `Vec<T>` → `Box<[T]>`). Line shapes: bare `syn::Type` lines
/// (one `;`/`,` terminator tolerated), or `Stmt::Local` pairs with a
/// type ascription on both sides and per-site token-equal patterns and
/// initializers (a per-site init/pattern delta is a let-statement
/// template, not a type template — that is WI-4's `try_let_sweep`
/// generalization territory, not this detector). All-identical line
/// classes decline (verbatim stays the tighter winner, T422).
/// Totality is the generic engine-count check
/// (`template_type_holds`): the emitted minus template must match
/// exactly N type subterms in the pre-image — macro/string twins never
/// count (the engine never enters them), nested matches count
/// outer-only (contained skips), exactly like the real sweep rewrites.
pub(crate) fn try_type_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut minus_sites = Vec::with_capacity(minuses.len());
    let mut plus_sites = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (mt, pt, m_text, p_text) = extract_type_pair(m, p)?;
        minus_sites.push(TypedSite {
            ty: mt,
            text: m_text,
        });
        plus_sites.push(TypedSite {
            ty: pt,
            text: p_text,
        });
    }
    let (minus_tpl, plus_tpl) = antiunify_types(&minus_sites, &plus_sites)?;
    if !crate::totality::template_type_holds(path, file_old, &minus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "type_sweep");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// One pair's `(minus, plus)` types plus their original spellings: the
/// ascribed types of a `let` pair whose pattern and initializer are
/// token-equal (plus `let-else` diverge agreement — the engine never
/// moves into diverge targets). Bare-type full lines never occur in
/// `-U0` diffs of valid Rust (every type sits inside a larger line),
/// so there is deliberately no bare-`syn::Type` path: every pair site
/// is provably a type position, and the expression-domain hazard class
/// cannot arise. The texts are line-sliced via the types' own spans
/// and re-parsed, so every span inside them slices the text — template
/// rendering inherits the first site's exact spacing.
fn extract_type_pair(m: &str, p: &str) -> Option<(syn::Type, syn::Type, String, String)> {
    let (minus, plus) = match (
        syn::parse_str::<syn::Stmt>(m.trim()),
        syn::parse_str::<syn::Stmt>(p.trim()),
    ) {
        (Ok(syn::Stmt::Local(a)), Ok(syn::Stmt::Local(b))) => (a, b),
        _ => return None,
    };
    let (syn::Pat::Type(minus_pt), syn::Pat::Type(plus_pt)) = (&minus.pat, &plus.pat) else {
        return None;
    };
    if minus_pt.pat.to_token_stream().to_string() != plus_pt.pat.to_token_stream().to_string() {
        return None;
    }
    match (&minus.init, &plus.init) {
        (Some(a), Some(b)) => {
            if a.diverge.is_some() || b.diverge.is_some() {
                return None;
            }
            if a.expr.to_token_stream().to_string() != b.expr.to_token_stream().to_string() {
                return None;
            }
        }
        (None, None) => {}
        _ => return None,
    }
    let m_text = slice_type_text(m.trim(), &minus_pt.ty)?;
    let p_text = slice_type_text(p.trim(), &plus_pt.ty)?;
    let mt = syn::parse_str::<syn::Type>(&m_text).ok()?;
    let pt = syn::parse_str::<syn::Type>(&p_text).ok()?;
    Some((mt, pt, m_text, p_text))
}

/// Slice a type's original spelling from its single-line parent snippet
/// via the type's own span. Multi-line or out-of-bounds spans veto.
fn slice_type_text(line: &str, ty: &syn::Type) -> Option<String> {
    let span = ty.span();
    if span.start().line != 1 || span.end().line != 1 {
        return None;
    }
    let (s, e) = (span.start().column, span.end().column);
    if s >= e || e > line.len() || !line.is_char_boundary(s) || !line.is_char_boundary(e) {
        return None;
    }
    Some(line[s..e].to_string())
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic, infer_semantic_with_pre_images};
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

    const PRE_THREE: &str = "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n}\n";

    fn three_pair_body() -> String {
        "@@ -1,5 +1,5 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n-    let gamma_value = compute_third(extra);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u32 = compute_second(other);\n+    let gamma_value: u32 = compute_third(extra);\n }\n"
            .to_string()
    }

    #[test]
    fn let_type_add_fires() {
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[("f.rs", PRE_THREE)]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        // Consumed: no native let lines survive.
        assert!(!out.contains("compute_first(input);"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("# pat"), "got: {out}");
    }

    #[test]
    fn let_type_add_extra_untyped_let_vetoes() {
        // Totality: an untouched fourth untyped `let` in the pre-image
        // would be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n    let untouched_guard = compute_fourth(more);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn let_type_add_typed_twin_does_not_veto() {
        // An already-typed `let` never matches `let P0 = E0;` (ascription
        // agreement is strict), so it is not counted and never vetoes.
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n    let already: u32 = 1;\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn let_type_add_varying_type_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u64 = compute_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn let_type_add_init_delta_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u32 = changed_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn let_type_add_single_vetoes() {
        // Singles never reach the pair chain (whole-section singles try
        // item rename only; per-hunk singles likewise).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n+    let alpha_value: u32 = compute_first(input);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn let_type_add_unparseable_pre_vetoes() {
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    const PRE_TYPE: &str = "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let gamma_count: u32 = compute_third(extra);\n}\n";

    fn three_type_body() -> String {
        "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n-    let beta_value: Vec<String> = compute_second(other);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n+    let beta_value: Box<[String]> = compute_second(other);\n }\n"
            .to_string()
    }

    #[test]
    fn type_sweep_fires() {
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[("f.rs", PRE_TYPE)]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        // Templates inherit the sites' exact spacing (span-sliced).
        assert!(out.contains("- Vec<T0>\n"), "got: {out}");
        assert!(out.contains("+ Box<[T0]>\n"), "got: {out}");
        // Inferred holes need no declarations (SPEC 15.8).
        assert!(!out.contains("# type"), "got: {out}");
        // Consumed: no native type lines survive.
        assert!(!out.contains("Vec<u32>"), "got: {out}");
        assert!(
            out.len() < three_type_body().len() + 100,
            "type sweep must beat native bytes, got: {out}"
        );
    }

    #[test]
    fn type_sweep_extra_type_site_vetoes() {
        // Totality: a third `Vec<..>` ascription in the pre-image would
        // be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let gamma_extra: Vec<u8> = compute_third(extra);\n    let gamma_count: u32 = done();\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn type_sweep_untyped_let_does_not_veto() {
        // An untyped `let` holds no type subterm, so the template never
        // matches it — unlike `let P0 = E0;`, it is invisible here.
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let plain = compute_third(extra);\n    let gamma_count: u32 = done();\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn type_sweep_init_delta_vetoes() {
        // A per-site initializer delta is a let-statement template, not
        // a type template.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n-    let beta_value: Vec<String> = compute_second(other);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n+    let beta_value: Box<[String]> = changed_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn type_sweep_single_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn type_sweep_unparseable_pre_vetoes() {
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }
}
