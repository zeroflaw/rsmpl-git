//! Sub-line general-expression precision single (WI-7 slice-19, split
//! from `general_expr` per the D5 hygiene gate): N == 1 lifting of the
//! whole-line/let-init single to call-wrapped (and otherwise framed)
//! inner general deltas under `PrecisionUnique`.

/// WI-7 slice-19: sub-line general-expression single (N=1) under
/// `PrecisionUnique`. Fires iff the pair does NOT form a whole-line
/// general single (slices 4–5 own that shape — the outer parses as a
/// `Call`/`MethodCall` whose `variant()` is `None`, or any other
/// non-anti-unifiable whole-line pair), but
/// `subline::extract_general_sites` yields exactly one inner general
/// site with a real single-hole delta and the frame rule holds (e.g.
/// `foo(alpha + 1);` → `foo(alpha + 2);`, `let`-init included via the
/// shared `side_expr` entry). Uniqueness is proved by
/// `template_pair_holds` at n=1 over the emitted pair (one site
/// file-wide, zero opaque hits); the T361 byte-compare
/// pre-reservation (`single_reproduces`, slices-4–18 posture) vetoes
/// any TokenStream spacing drift on the sub-expression rewrite path.
/// Whole-line-owned pairs decline here (slices 4–5 own them, even when
/// they vetoed — no bypass around their twin veto via a second
/// counter). Multi-subterm lines decline (`sites.len() == 1` only —
/// uniform two-site lines are multi territory by site count, even at
/// one pair). Emission is the slice-4 allow-one spelling (tight
/// range, directive-free, default `allow=one`). Detector name
/// `general_expr` (same family, provenance unchanged).
pub(crate) fn try_subline_general_single(
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
    // Whole-line-owned pairs belong to the slices-4–5 singles — decline
    // here even when they vetoed (whole-line and `let`-init entries).
    if let (Some(m), Some(p)) = (
        super::general_expr::parse_line_expr(minuses[0]),
        super::general_expr::parse_line_expr(pluses[0]),
    ) {
        if super::general_expr::antiunify_general_expr_single(&m, &p).is_some() {
            return None;
        }
    }
    if let (Some(m), Some(p)) = (
        super::general_expr::parse_let_init(minuses[0]),
        super::general_expr::parse_let_init(pluses[0]),
    ) {
        if super::general_expr::antiunify_general_expr_single(&m, &p).is_some() {
            return None;
        }
    }
    let sites = super::subline::extract_general_sites(minuses[0], pluses[0])?;
    if sites.len() != 1 {
        return None;
    }
    // Zero-hole/two-delta/identical/variant-mismatch/`Await` vetoes
    // ride free via the shared single-pair core.
    let (minus_tpl, plus_tpl) =
        super::general_expr::antiunify_general_expr_single(&sites[0].minus, &sites[0].plus)?;
    // Anchored gate, file-wide uniqueness at n=1, byte-compare
    // pre-reservation, and the allow-one emission share the slice-4
    // tail (no mirrored emission).
    super::general_expr::emit_general_expr_single(
        path, pre, minuses[0], pluses[0], changed, &minus_tpl, &plus_tpl, used_rules,
    )
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
    fn wi19_subline_general_single_fires_and_round_trips() {
        // WI-7 slice-19: unique sub-line single `foo(alpha + 1);` →
        // `foo(alpha + 2);` raises to an allow-one hunk and
        // real-applies byte-identical (fixed point `S″ == S′` below).
        let pre = "fn m() {\n    foo(alpha + 1);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(alpha + 1);\n+    foo(alpha + 2);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique sub-line general single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- E0 + 1\n"), "got: {out}");
        assert!(out.contains("+ E0 + 2\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    foo(alpha + 2);\n}\n");
        // Fixed point: re-diffing the applied tree re-emits the same
        // single (canonical `S″ == S′`).
        let re = super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    foo(alpha + 1);\n+    foo(alpha + 2);\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi19_subline_general_single_nested_call_fires() {
        // A method-wrapped delta is owned by the earlier method-wrap
        // arm (`Receiver.run(alpha + 1)` concrete), so the sub-line
        // general probe uses a nested bare call the wrap arms decline:
        // no side token-equals, no structural wrap, arity 1.
        let pre = "fn m() {\n    foo(bar(alpha + 1));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha + 1));\n+    foo(bar(alpha + 2));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("nested-call sub-line general single must fire");
        assert!(out.contains("- E0 + 1\n"), "got: {out}");
        assert!(out.contains("+ E0 + 2\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi19_subline_general_single_nested_base_fires() {
        // An unchanged nested base (`self.alpha` inside the binary site)
        // is skipped, never vetoed: it cannot match the emitted pair.
        let pre = "fn m() {\n    foo(self.alpha + 1);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(self.alpha + 1);\n+    foo(self.alpha + 2);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("nested-base sub-line general single must fire");
        assert!(out.contains("- E0 + 1\n"), "got: {out}");
        assert!(out.contains("+ E0 + 2\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi19_subline_general_single_let_init_fires() {
        // `let`-init framing shares the `side_expr` entry; the init is a
        // call, so slices 4–5 decline and the inner site fires.
        let pre = "fn m() {\n    let y = foo(alpha + 1);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let y = foo(alpha + 1);\n+    let y = foo(alpha + 2);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init sub-line general single must fire");
        assert!(out.contains("- E0 + 1\n"), "got: {out}");
        assert!(out.contains("+ E0 + 2\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi19_subline_general_single_field_variant_fires() {
        // Non-binary variants ride the same core: `s.len` → `s.size`
        // anti-unifies to `E0.len` / `E0.size`.
        let pre = "fn m() {\n    foo(item.len);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(item.len);\n+    foo(item.size);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("field sub-line general single must fire");
        assert!(out.contains("- E0.len\n"), "got: {out}");
        assert!(out.contains("+ E0.size\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi19_subline_general_single_twin_vetoes() {
        // An untouched same-template twin anywhere vetoes.
        let pre = "fn m() {\n    foo(alpha + 1);\n    bar(alpha + 1);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(alpha + 1);\n+    foo(alpha + 2);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi19_subline_general_single_different_template_twin_keeps_firing() {
        // A twin under a different template never counts.
        let pre = "fn m() {\n    foo(alpha + 1);\n    bar(alpha + 9);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(alpha + 1);\n+    foo(alpha + 2);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-template twin must not veto");
        assert!(out.contains("- E0 + 1\n"), "got: {out}");
    }

    #[test]
    fn wi19_subline_general_single_whole_line_declines() {
        // Whole-line pairs belong to the slice-4 arm, never this one.
        assert!(
            super::try_subline_general_single(
                &["    alpha + 1;"],
                &["    alpha + 2;"],
                "f.rs",
                Some("fn m() {\n    alpha + 1;\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi19_subline_general_single_let_init_whole_line_declines() {
        // `let`-init pairs whose init is itself a general single belong
        // to the slice-5 arm, never this one.
        assert!(
            super::try_subline_general_single(
                &["    let y = alpha + 1;"],
                &["    let y = alpha + 2;"],
                "f.rs",
                Some("fn m() {\n    let y = alpha + 1;\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "let-init whole-line pair must decline"
        );
    }

    #[test]
    fn wi19_subline_general_single_two_site_line_declines() {
        // Uniform two-site lines are multi territory by site count, even
        // at one pair (no general sub-line multi exists).
        let pre = "fn m() {\n    foo(alpha + 1) + bar(beta + 1);\n}\n";
        assert!(
            super::try_subline_general_single(
                &["    foo(alpha + 1) + bar(beta + 1);"],
                &["    foo(alpha + 2) + bar(beta + 2);"],
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
    fn wi19_subline_general_single_no_pre_image_vetoes() {
        assert!(
            super::try_subline_general_single(
                &["    foo(alpha + 1);"],
                &["    foo(alpha + 2);"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi19_subline_general_single_callee_delta_declines() {
        // A callee delta is call-family territory: no general subterm
        // pairs with a delta, so the extractor finds nothing.
        let pre = "fn m() {\n    foo(alpha);\n}\n";
        assert!(
            super::try_subline_general_single(
                &["    foo(alpha);"],
                &["    bar(alpha);"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "non-general delta must decline"
        );
    }

    #[test]
    fn wi19_subline_general_single_macro_vetoes() {
        // Macro bodies never bind: the extractor vetoes.
        let pre = "fn m() {\n    foo(m!(alpha) + 1);\n}\n";
        assert!(
            super::try_subline_general_single(
                &["    foo(m!(alpha) + 1);"],
                &["    foo(m!(alpha) + 2);"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "macro subterm must veto"
        );
    }
}
