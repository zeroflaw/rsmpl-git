//! Generalized anti-unification sweeps (WI-4a first slice).
//!
//! Thin detectors over [`super::antiunify`]: this slice ships
//! `try_let_sweep` for let type-addition (`let PAT = INIT;` →
//! `let PAT: T = INIT;` with per-site preserved `PAT`/`INIT` bound as
//! inferred `P0`/`E0` and a uniform concrete `T`). The `syn::Type`
//! sweep (WI-4b) and signature evolution (WI-4c) land here.

use super::antiunify::antiunify_let_type_add;
use super::unique_rule;

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
}
