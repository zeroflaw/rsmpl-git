//! Whole-line bare-call arg-permutation single (WI-7 slice-10).
//!
//! Split from `arg_template.rs` under the 2000-line hygiene gate (D5):
//! pure move of the slice-10 `try_call_perm_single` production plus its
//! inline pins, no behavior change. The wrap singles and their pins
//! stay in `arg_template.rs`; this module owns only the perm single.
//! Unit tests stay inline (`mod tests` below, WI-19).

use super::arg_template::{parse_bare_pair, tok};
use super::unique_rule;

/// WI-7 slice-10: whole-line bare-call arg-permutation single (N=1)
/// under `PrecisionUnique`. Fires iff the pair parses (whole-line or
/// the shared T449 `let`-init entry — the emission stays the bare-call
/// pattern and the engine rewrites the call sub-expression in place)
/// as one same-callee call whose plus args are a non-trivial
/// permutation of the minus args (pure bijection over token text, no
/// wraps, no concrete deltas — those stay with the wrap/literal
/// singles), a pre-image is present (`None` vetoes — uniqueness needs
/// the whole file, D3), and `template_pair_holds` proves exactly one
/// site file-wide with zero opaque hits (conditions (a)+(b); (c) is
/// structural: the callee token anchors the `E{i}` holes). The T361
/// byte-compare pre-reservation (`single_reproduces`, shared with the
/// wrap singles) vetoes any TokenStream spacing drift. Emission is
/// the WI-19b allow-one spelling (tight range, directive-free,
/// default `allow=one`). Detector name `call_arg_template` (same
/// family, provenance unchanged). The N ≥ 2 perm sweep
/// (`try_call_perm_sweep`) keeps its sub-line-only classes untouched.
pub(crate) fn try_call_perm_single(
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
    let (o_cal, n_cal, o_args, n_args) = parse_bare_pair(minuses[0], pluses[0])?;
    // Callee deltas stay with the slice-1 single.
    if o_cal != n_cal {
        return None;
    }
    let k = o_args.len();
    // Arity 0/1 cannot permute; `parse_bare_pair` already enforces
    // uniform arity across the pair.
    if k < 2 {
        return None;
    }
    for arg in o_args.iter().chain(n_args.iter()) {
        let probe = super::shape::probe_expr(arg);
        if probe.macros > 0 || probe.unsupported > 0 {
            return None;
        }
    }
    let minus_toks: Vec<String> = o_args.iter().map(tok).collect();
    let plus_toks: Vec<String> = n_args.iter().map(tok).collect();
    // Pure bijection over token text: same multiset, non-trivial order.
    let mut sorted_minus = minus_toks.clone();
    let mut sorted_plus = plus_toks.clone();
    sorted_minus.sort();
    sorted_plus.sort();
    if sorted_minus != sorted_plus {
        return None;
    }
    if minus_toks == plus_toks {
        return None;
    }
    // Deterministic hole mapping: each plus position binds the first
    // still-unused minus position with equal token text. Multiset
    // equality above guarantees every position binds exactly once.
    let mut used = vec![false; k];
    let mut plus_tpl = Vec::with_capacity(k);
    for pt in &plus_toks {
        let mut found = None;
        for (i, mt) in minus_toks.iter().enumerate() {
            if !used[i] && mt == pt {
                found = Some(i);
                break;
            }
        }
        let i = found?;
        used[i] = true;
        plus_tpl.push(format!("E{i}"));
    }
    let minus_tpl: Vec<String> = (0..k).map(|i| format!("E{i}")).collect();
    // WI-7b: uppercase same callee rides backticks (literal), else it
    // infers as an `expr` hole and over-matches.
    let lit = super::shape::literal_callee(&o_cal);
    let minus_call = format!("{lit}({})", minus_tpl.join(", "));
    let plus_call = format!("{lit}({})", plus_tpl.join(", "));
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    // The check runs on the raw callee: backticks are RSmPL-level
    // literal spelling (SPEC 15.4), not Rust syntax, so the backticked
    // emission would never parse as an `Expr`.
    let raw_minus = format!("{o_cal}({})", minus_tpl.join(", "));
    let raw_plus = format!("{o_cal}({})", plus_tpl.join(", "));
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return None;
    }
    if !crate::totality::template_pair_holds(path, Some(pre), &minus_call, &plus_call, 1) {
        return None;
    }
    if !super::general_expr::single_reproduces(
        path,
        pre,
        changed[0],
        minuses[0],
        pluses[0],
        &minus_call,
        &plus_call,
    ) {
        return None;
    }
    let rule = unique_rule(used_rules, "call_arg_template");
    // Allow-one: directive-free ranged hunk (slices-1-9 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
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

    fn psingle(minus: &str, plus: &str, pre: Option<&str>, changed: &[usize]) -> Option<String> {
        super::try_call_perm_single(&[minus], &[plus], "f.rs", pre, changed, &mut Vec::new())
    }

    #[test]
    fn wi10_perm_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-10: unique single `swap(x, y)` -> `swap(y, x)`
        // raises to an allow-one hunk and real-applies byte-identical.
        let pre =
            "fn swap(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn f(x: i32, y: i32) -> i32 {\n    swap(x, y)\n}\n";
        let raw = section("f.rs", "@@ -5 +5 @@\n-    swap(x, y)\n+    swap(y, x)\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique perm single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- swap(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ swap(E1, E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn swap(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn f(x: i32, y: i32) -> i32 {\n    swap(y, x)\n}\n"
        );
    }

    #[test]
    fn wi10_perm_single_let_init_fires_and_round_trips() {
        // The shared T449 entry unwraps `let` inits: the emission stays
        // the bare-call pattern and the engine rewrites the call
        // sub-expression in place.
        let pre = "fn m() {\n    let v = swap(x, y);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let v = swap(x, y);\n+    let v = swap(y, x);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique let-init perm single must fire");
        assert!(out.contains("- swap(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ swap(E1, E0)\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    let v = swap(y, x);\n}\n");
    }

    #[test]
    fn wi10_perm_single_three_arg_fires() {
        // Non-trivial 3-cycles prove the same way (first-unused mapping).
        let out = psingle(
            "    f(a, b, c);",
            "    f(c, a, b);",
            Some("fn m() {\n    f(a, b, c);\n}\n"),
            &[2],
        )
        .expect("3-cycle perm single must fire");
        assert!(out.contains("- f(E0, E1, E2)\n"), "got: {out}");
        assert!(out.contains("+ f(E2, E0, E1)\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
    }

    #[test]
    fn wi10_perm_single_twin_vetoes() {
        // An untouched same-callee same-arity twin anywhere vetoes.
        assert!(
            psingle(
                "    swap(x, y);",
                "    swap(y, x);",
                Some("fn m() {\n    swap(x, y);\n    swap(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi10_perm_single_different_arity_twin_keeps_firing() {
        // Totality keys on the template: a different-arity twin never
        // matches the emitted pattern and never vetoes.
        let out = psingle(
            "    swap(x, y);",
            "    swap(y, x);",
            Some("fn m() {\n    swap(x, y);\n    swap(x, y, z);\n}\n"),
            &[2],
        )
        .expect("different-arity twin must not veto");
        assert!(out.contains("+ swap(E1, E0)\n"), "got: {out}");
    }

    #[test]
    fn wi10_perm_single_none_vetoes() {
        // No whole-file evidence → no guessing (D3).
        assert!(
            psingle("    swap(x, y);", "    swap(y, x);", None, &[2]).is_none(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi10_perm_single_identity_vetoes() {
        // No delta is nobody's class.
        assert!(
            psingle(
                "    swap(x, y);",
                "    swap(x, y);",
                Some("fn m() {\n    swap(x, y);\n}\n"),
                &[2]
            )
            .is_none(),
            "identity must veto"
        );
    }

    #[test]
    fn wi10_perm_single_non_bijection_vetoes() {
        // A real arg delta is not a permutation (wrap/literal territory).
        assert!(
            psingle(
                "    swap(a, b);",
                "    swap(b, c);",
                Some("fn m() {\n    swap(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "non-bijective delta must veto"
        );
    }

    #[test]
    fn wi10_perm_single_unary_vetoes() {
        // Arity 0/1 cannot permute.
        assert!(
            psingle(
                "    f(x);",
                "    f(x);",
                Some("fn m() {\n    f(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "unary must veto"
        );
        assert!(
            psingle(
                "    f();",
                "    f();",
                Some("fn m() {\n    f();\n}\n"),
                &[2]
            )
            .is_none(),
            "nullary must veto"
        );
    }

    #[test]
    fn wi10_perm_single_callee_delta_declines() {
        // Callee deltas stay with the slice-1 single.
        assert!(
            psingle(
                "    foo(a, b);",
                "    bar(b, a);",
                Some("fn m() {\n    foo(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "callee delta must decline here"
        );
    }

    #[test]
    fn wi10_perm_single_macro_arg_vetoes() {
        // Macro-hiding args never raise: the engine cannot match
        // TokenStream-normalized macro text.
        assert!(
            psingle(
                "    swap(m!(x), y);",
                "    swap(y, m!(x));",
                Some("fn m() {\n    swap(m!(x), y);\n}\n"),
                &[2]
            )
            .is_none(),
            "macro arg must veto"
        );
    }

    #[test]
    fn wi10_perm_single_uppercase_fires_literal() {
        // Uppercase callees ride backticks (literal), else the engine
        // would infer them as `expr` holes.
        let out = psingle(
            "    Swap(a, b);",
            "    Swap(b, a);",
            Some("fn m() {\n    Swap(a, b);\n}\n"),
            &[2],
        )
        .expect("uppercase perm single must fire");
        assert!(out.contains("`Swap`(E0, E1)"), "got: {out}");
        assert!(out.contains("`Swap`(E1, E0)"), "got: {out}");
    }
}
