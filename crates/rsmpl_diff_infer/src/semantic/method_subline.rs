//! Sub-line method-to-method precision single (WI-7 slice-15, split
//! from `methods` per the D5 hygiene gate): N == 1 lifting of the
//! `try_method_to_method_sweep` WI-4 sub-line fallback under
//! `PrecisionUnique`.

use super::methods::{m2f_args, parse_method_to_method_pair};
use super::unique_rule;

/// WI-7 slice-15: sub-line method-to-method rename-only single (N=1)
/// under `PrecisionUnique`. Fires iff the pair does NOT parse
/// whole-line as a method-to-method rename (slice-2 owns that shape),
/// but `subline::extract_method_sites` yields exactly one site with a
/// real method delta and the frame rule holds (e.g.
/// `val.old(alpha) + 1` → `val.new(alpha) + 1`, `let`-init included
/// via the shared `side_expr` entry). Uniqueness is proved by
/// `template_expr_holds` at n=1 over the emitted minus template
/// (same probe as the multi — one site file-wide, zero opaque hits
/// via the engine count); the T361 byte-compare pre-reservation
/// (`single_reproduces`, slices-4–14 posture) vetoes any TokenStream
/// spacing drift on the sub-expression rewrite path. Whole-line pairs
/// decline here (slice-2 owns them, even when it vetoed — no bypass
/// around its twin veto via a second counter). Multi-subterm lines
/// decline (`sites.len() == 1` only — uniform two-site lines are multi
/// territory by site count, even at one pair). Emission is the slice-2
/// allow-one spelling (tight range, directive-free, default
/// `allow=one`). Detector name `method_to_method` (same family,
/// provenance unchanged).
pub(crate) fn try_subline_method_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let pre = file_old?;
    // Whole-line pairs belong to the slice-2 single — decline here even
    // when it vetoed.
    if parse_method_to_method_pair(minuses[0], pluses[0]).is_some() {
        return None;
    }
    let sites = super::subline::extract_method_sites(minuses[0], pluses[0])?;
    if sites.len() != 1 {
        return None;
    }
    let (old_method, new_method, arity) = (
        sites[0].old_method.clone(),
        sites[0].new_method.clone(),
        sites[0].arity,
    );
    if old_method == new_method {
        return None;
    }
    if !crate::totality::template_expr_holds(
        path,
        file_old,
        &format!("Receiver.{old_method}({})", m2f_args(arity)),
        1,
    ) {
        return None;
    }
    let mut vars = Vec::with_capacity(arity);
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let old_lit = super::shape::literal_callee(&old_method);
    let new_lit = super::shape::literal_callee(&new_method);
    let minus_tpl = format!("Receiver.{old_lit}({args})");
    let plus_tpl = format!("Receiver.{new_lit}({args})");
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    // The check runs on the raw method: backticks are RSmPL-level
    // literal spelling (SPEC 15.4), not Rust syntax (TOOL-5).
    let raw_minus = format!("Receiver.{old_method}({args})");
    let raw_plus = format!("Receiver.{new_method}({args})");
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return None;
    }
    // T361 byte gate: the sub-expression rewrite must reproduce the
    // native line swap byte-identical.
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_tpl, &plus_tpl,
    ) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_method}"));
    // Allow-one: directive-free ranged hunk (slice-2 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn wi15_subline_method_single_fires_and_round_trips() {
        // WI-7 slice-15: unique sub-line single
        // `val.old_method(a) + 1` → `val.new_method(a) + 1` raises to
        // an allow-one hunk and real-applies byte-identical (fixed
        // point `S″ == S′` checked below).
        let pre = "fn m() {\n    val.old_method(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    val.new_method(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique sub-line method single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_method(E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    val.new_method(alpha_argument) + 1;\n}\n"
        );
        // Fixed point: re-diffing the applied tree re-emits the same
        // single (canonical `S″ == S′`).
        let re = super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    val.new_method(alpha_argument) + 1;\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi15_subline_method_single_let_init_fires() {
        // `let`-init framing shares the `side_expr` entry.
        let pre = "fn m() {\n    let x = val.old_method(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = val.old_method(alpha_argument) + 1;\n+    let x = val.new_method(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init sub-line method single must fire");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_method(E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi15_subline_method_single_uppercase_fires_literal() {
        let pre = "fn m() {\n    val.OldMethod(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.OldMethod(alpha_argument) + 1;\n+    val.NewMethod(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("uppercase sub-line method single must fire");
        assert!(out.contains("Receiver.`OldMethod`(E0)"), "got: {out}");
        assert!(out.contains("Receiver.`NewMethod`(E0)"), "got: {out}");
    }

    #[test]
    fn wi15_subline_method_single_twin_vetoes() {
        // An untouched same-method same-arity twin anywhere vetoes.
        let pre = "fn m() {\n    val.old_method(alpha_argument) + 1;\n    other.old_method(different_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    val.new_method(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi15_subline_method_single_different_arity_twin_keeps_firing() {
        let pre =
            "fn m() {\n    val.old_method(alpha_argument) + 1;\n    val.old_method(alpha_argument, beta_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    val.new_method(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-arity twin must not veto");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi15_subline_method_single_whole_line_declines() {
        // Whole-line pairs belong to the slice-2 arm, never this one.
        assert!(
            super::try_subline_method_single(
                &["    val.old_method(alpha_argument);"],
                &["    val.new_method(alpha_argument);"],
                "f.rs",
                Some("fn m() {\n    val.old_method(alpha_argument);\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi15_subline_method_single_two_site_line_declines() {
        // Uniform two-site lines are multi territory by site count, even
        // at one pair (the multi needs N>=2 pairs).
        let pre = "fn m() {\n    val.old_method(alpha_argument) + val.old_method(beta_value);\n}\n";
        assert!(
            super::try_subline_method_single(
                &["    val.old_method(alpha_argument) + val.old_method(beta_value);"],
                &["    val.new_method(alpha_argument) + val.new_method(beta_value);"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "two-site line must decline"
        );
    }

    #[test]
    fn wi15_subline_method_single_no_pre_image_vetoes() {
        assert!(
            super::try_subline_method_single(
                &["    val.old_method(alpha_argument) + 1;"],
                &["    val.new_method(alpha_argument) + 1;"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "None pre-image must veto"
        );
    }
}
