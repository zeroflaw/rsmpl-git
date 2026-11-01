//! Whole-line bare-call arg-permutation single (WI-7 slice-10).
//!
//! Split from `arg_template.rs` under the 2000-line hygiene gate (D5):
//! pure move of the slice-10 `try_call_perm_single` production plus its
//! inline pins, no behavior change. The wrap singles and their pins
//! stay in `arg_template.rs`; this module owns only the perm single.
//! Unit tests stay inline (`mod tests` below, WI-19).

use super::arg_template::{parse_bare_pair, parse_method_line, tok};
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

/// WI-7 slice-17: sub-line bare perm single (N=1) under
/// `PrecisionUnique`. Fires iff the pair does NOT parse whole-line as a
/// same-callee perm pair (slice-10 owns that shape, including the T449
/// `let`-init entry), but `subline::extract_arg_permutation_sites`
/// yields exactly one site with a real non-trivial permutation and the
/// frame rule holds (e.g. `swap(aa, bb) + 1` -> `swap(bb, aa) + 1`,
/// `let`-init included via the shared `side_expr` entry). Uniqueness is
/// proved by `template_pair_holds` at n=1 over the emitted pair (same
/// probe as the multi - one site file-wide, zero opaque hits via the
/// engine count); the T361 byte-compare pre-reservation
/// (`single_reproduces`, slices-4-16 posture) vetoes any TokenStream
/// spacing drift on the sub-expression rewrite path. Whole-line pairs
/// decline here (slice-10 owns them, even when it vetoed - no bypass
/// around its twin veto via a second counter). Multi-subterm lines
/// decline (`sites.len() == 1` only - uniform two-site lines are multi
/// territory by site count, even at one pair). Emission is the slice-10
/// allow-one spelling (tight range, directive-free, default
/// `allow=one`). Detector name `call_arg_template` (same family,
/// provenance unchanged).
pub(crate) fn try_subline_perm_single(
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
    let pre = file_old?;
    if super::arg_template::parse_bare_pair(minuses[0], pluses[0]).is_some() {
        return None;
    }
    let sites = super::subline::extract_arg_permutation_sites(minuses[0], pluses[0])?;
    if sites.len() != 1 {
        return None;
    }
    let (cal, arity, perm) = (sites[0].cal.clone(), sites[0].arity, sites[0].perm.clone());
    if arity < 2 {
        return None;
    }
    let minus_tpl: Vec<String> = (0..arity).map(|i| format!("E{i}")).collect();
    let mut plus_tpl = Vec::with_capacity(arity);
    for p in perm.iter().take(arity) {
        plus_tpl.push(format!("E{p}"));
    }
    let lit = super::shape::literal_callee(&cal);
    let minus_call = format!("{lit}({})", minus_tpl.join(", "));
    let plus_call = format!("{lit}({})", plus_tpl.join(", "));
    let raw_minus = format!("{cal}({})", minus_tpl.join(", "));
    let raw_plus = format!("{cal}({})", plus_tpl.join(", "));
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
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    Some(hunk)
}

/// WI-7 slice-18: sub-line method perm single (N=1) under
/// `PrecisionUnique`. Fires iff the pair does NOT parse whole-line as a
/// method call on both sides (slice-11 owns that shape, including the
/// T449 `let`-init entry), but `subline::extract_method_perm_sites`
/// yields exactly one site with a real non-trivial permutation and the
/// frame rule holds (e.g. `svc.configure(aa, bb) + 1` ->
/// `svc.configure(bb, aa) + 1`, `let`-init included via the shared
/// `side_expr` entry). Receiver deltas stay with the m2f single,
/// method deltas with the slice-15 single — both veto in the
/// extractor. Uniqueness is proved by `template_pair_holds` at n=1 over
/// the emitted pair (one site file-wide, zero opaque hits via the
/// engine count); the T361 byte-compare pre-reservation
/// (`single_reproduces`, slices-4-17 posture) vetoes any TokenStream
/// spacing drift on the sub-expression rewrite path. Whole-line pairs
/// decline here (slice-11 owns them, even when it vetoed - no bypass
/// around its twin veto via a second counter). Multi-subterm lines
/// decline (`sites.len() == 1` only - uniform two-site lines are multi
/// territory by site count, even at one pair). Emission is the slice-11
/// allow-one spelling (tight range, directive-free, default
/// `allow=one`). Detector name `method_arg_template` (same family,
/// provenance unchanged).
pub(crate) fn try_subline_method_perm_single(
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
    let pre = file_old?;
    if super::arg_template::parse_method_line(minuses[0]).is_some()
        && super::arg_template::parse_method_line(pluses[0]).is_some()
    {
        return None;
    }
    let sites = super::subline::extract_method_perm_sites(minuses[0], pluses[0])?;
    if sites.len() != 1 {
        return None;
    }
    let (method, arity, perm) = (
        sites[0].method.clone(),
        sites[0].arity,
        sites[0].perm.clone(),
    );
    if arity < 2 {
        return None;
    }
    let minus_tpl: Vec<String> = (0..arity).map(|i| format!("E{i}")).collect();
    let mut plus_tpl = Vec::with_capacity(arity);
    for p in perm.iter().take(arity) {
        plus_tpl.push(format!("E{p}"));
    }
    let lit = super::shape::literal_callee(&method);
    let minus_call = format!("Receiver.{lit}({})", minus_tpl.join(", "));
    let plus_call = format!("Receiver.{lit}({})", plus_tpl.join(", "));
    let raw_minus = format!("Receiver.{method}({})", minus_tpl.join(", "));
    let raw_plus = format!("Receiver.{method}({})", plus_tpl.join(", "));
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
    let rule = unique_rule(used_rules, "method_arg_template");
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    Some(hunk)
}

/// WI-7 slice-11: whole-line method-call arg-permutation single (N=1)
/// under `PrecisionUnique`. Twin of the slice-10 bare-call entry:
/// same receiver and same method (receiver deltas stay with the m2f
/// single, method deltas stay with the slice-2 single), plus args a
/// non-trivial permutation of the minus args (pure bijection over
/// token text). Whole-line or the shared T449 `let`-init entry — the
/// emission stays the `Receiver.m(…)` pattern and the engine rewrites
/// the call sub-expression in place. Detector name
/// `method_arg_template` (same family, provenance unchanged).
pub(crate) fn try_method_perm_single(
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
    let pre = file_old?;
    let (o_recv, o_method, o_args) = parse_method_line(minuses[0])?;
    let (n_recv, n_method, n_args) = parse_method_line(pluses[0])?;
    if o_recv != n_recv || o_method != n_method {
        return None;
    }
    let k = o_args.len();
    if k < 2 || n_args.len() != k {
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
    let lit = super::shape::literal_callee(&o_method);
    let minus_call = format!("Receiver.{lit}({})", minus_tpl.join(", "));
    let plus_call = format!("Receiver.{lit}({})", plus_tpl.join(", "));
    // Raw method for the re-parse check (TOOL-5: backticks never parse
    // as `syn::Expr`); the emission keeps the backticked literal.
    let raw_minus = format!("Receiver.{o_method}({})", minus_tpl.join(", "));
    let raw_plus = format!("Receiver.{o_method}({})", plus_tpl.join(", "));
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
    let rule = unique_rule(used_rules, "method_arg_template");
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

    fn msingle(minus: &str, plus: &str, pre: Option<&str>, changed: &[usize]) -> Option<String> {
        super::try_method_perm_single(&[minus], &[plus], "f.rs", pre, changed, &mut Vec::new())
    }

    #[test]
    fn wi11_method_perm_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-11: unique `service.configure(alpha, beta)` ->
        // `service.configure(beta, alpha)` raises to an allow-one hunk
        // and real-applies byte-identical. Long arg names clear the
        // precision guard (`body <= hunk + 16`); short `s.foo(a, b)`
        // shapes stay ordinary by design (pinned by the direct-core
        // twin test below, which bypasses the guard).
        let pre = "struct S;\nimpl S {\n    fn configure(&self, a: i32, b: i32) {}\n}\nfn m(service: S) {\n    service.configure(alpha, beta);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -6 +6 @@\n-    service.configure(alpha, beta);\n+    service.configure(beta, alpha);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique method perm single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        assert!(!out.contains("# sweep"), "ranged single: {out}");
        assert!(out.contains("- Receiver.configure(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.configure(E1, E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("service.configure(beta, alpha);"),
            "got: {}",
            files["f.rs"]
        );
        // Fixed point: re-diff of the applied tree is itself.
        let out2 =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("must fire");
        assert_eq!(out, out2, "fixed point");
    }

    #[test]
    fn wi11_method_perm_single_let_init_fires() {
        let pre = "fn m(service: S) {\n    let v = service.configure(alpha, beta);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let v = service.configure(alpha, beta);\n+    let v = service.configure(beta, alpha);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init method perm must fire");
        assert!(out.contains("- Receiver.configure(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.configure(E1, E0)\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi11_method_perm_single_three_arg_fires() {
        let out = msingle(
            "    s.foo(a, b, c);",
            "    s.foo(c, a, b);",
            Some("fn m(s: S) {\n    s.foo(a, b, c);\n}\n"),
            &[2],
        )
        .expect("3-cycle method perm must fire");
        assert!(out.contains("- Receiver.foo(E0, E1, E2)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.foo(E2, E0, E1)\n"), "got: {out}");
    }

    #[test]
    fn wi11_method_perm_single_twin_vetoes() {
        assert!(
            msingle(
                "    s.foo(a, b);",
                "    s.foo(b, a);",
                Some("fn m(s: S) {\n    s.foo(a, b);\n    s.foo(c, d);\n}\n"),
                &[2]
            )
            .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi11_method_perm_single_different_arity_twin_fires() {
        let out = msingle(
            "    s.foo(a, b);",
            "    s.foo(b, a);",
            Some("fn m(s: S) {\n    s.foo(a, b);\n    s.foo(a, b, c);\n}\n"),
            &[2],
        )
        .expect("different-arity twin must not veto");
        assert!(out.contains("+ Receiver.foo(E1, E0)\n"), "got: {out}");
    }

    #[test]
    fn wi11_method_perm_single_receiver_delta_declines() {
        assert!(
            msingle(
                "    s.foo(a, b);",
                "    t.foo(b, a);",
                Some("fn m(s: S, t: S) {\n    s.foo(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "receiver delta must decline here"
        );
    }

    #[test]
    fn wi11_method_perm_single_method_delta_declines() {
        assert!(
            msingle(
                "    s.foo(a, b);",
                "    s.bar(b, a);",
                Some("fn m(s: S) {\n    s.foo(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "method delta must decline here"
        );
    }

    #[test]
    fn wi11_method_perm_single_none_identity_nonbijection_veto() {
        assert!(msingle("    s.foo(a, b);", "    s.foo(b, a);", None, &[2]).is_none());
        assert!(
            msingle(
                "    s.foo(a, b);",
                "    s.foo(a, b);",
                Some("fn m(s: S) {\n    s.foo(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "identity must veto"
        );
        assert!(
            msingle(
                "    s.foo(a, b);",
                "    s.foo(b, c);",
                Some("fn m(s: S) {\n    s.foo(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "non-bijective delta must veto"
        );
        assert!(
            msingle(
                "    s.foo(m!(x), y);",
                "    s.foo(y, m!(x));",
                Some("fn m(s: S) {\n    s.foo(m!(x), y);\n}\n"),
                &[2]
            )
            .is_none(),
            "macro arg must veto"
        );
    }

    #[test]
    fn wi11_method_perm_single_uppercase_method_fires_literal() {
        let out = msingle(
            "    s.Foo(a, b);",
            "    s.Foo(b, a);",
            Some("fn m(s: S) {\n    s.Foo(a, b);\n}\n"),
            &[2],
        )
        .expect("uppercase method perm must fire");
        assert!(out.contains("Receiver.`Foo`(E0, E1)"), "got: {out}");
        assert!(out.contains("Receiver.`Foo`(E1, E0)"), "got: {out}");
    }

    #[test]
    fn wi11_tool5_uppercase_wrap_singles_fire_literal() {
        // TOOL-5: the slice-6/7 defensive re-parse used to veto
        // uppercase same-callee wraps (backticks never parse as
        // `syn::Expr`); the check now runs on the raw callee/method.
        let out = super::super::arg_template::try_call_arg_template_single(
            &["    Foo(x);"],
            &["    Foo(x.clone());"],
            "f.rs",
            Some("fn m() {\n    Foo(x);\n}\n"),
            &[2],
            &mut Vec::new(),
        )
        .expect("uppercase bare-call wrap single must fire");
        assert!(out.contains("`Foo`(E0)"), "got: {out}");
        assert!(out.contains("`Foo`(E0.clone())"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        let out = super::super::arg_template::try_method_arg_template_single(
            &["    s.Foo(x);"],
            &["    s.Foo(x.clone());"],
            "f.rs",
            Some("fn m(s: S) {\n    s.Foo(x);\n}\n"),
            &[2],
            &mut Vec::new(),
        )
        .expect("uppercase method wrap single must fire");
        assert!(out.contains("Receiver.`Foo`(E0)"), "got: {out}");
        assert!(out.contains("Receiver.`Foo`(E0.clone())"), "got: {out}");
    }

    #[test]
    fn wi17_subline_perm_single_fires_and_round_trips() {
        let pre = "fn m() {\n    swap(alpha_arg, beta_arg) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    swap(alpha_arg, beta_arg) + 1;\n+    swap(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique sub-line perm single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- swap(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ swap(E1, E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    swap(beta_arg, alpha_arg) + 1;\n}\n"
        );
        let re = super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    swap(alpha_arg, beta_arg) + 1;\n+    swap(beta_arg, alpha_arg) + 1;\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi17_subline_perm_single_let_init_fires() {
        let pre = "fn m() {\n    let x = swap(alpha_arg, beta_arg) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = swap(alpha_arg, beta_arg) + 1;\n+    let x = swap(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init sub-line perm single must fire");
        assert!(out.contains("- swap(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ swap(E1, E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi17_subline_perm_single_uppercase_fires_literal() {
        let pre = "fn m() {\n    Swap(alpha_arg, beta_arg) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    Swap(alpha_arg, beta_arg) + 1;\n+    Swap(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("uppercase sub-line perm single must fire");
        assert!(out.contains("`Swap`(E0, E1)"), "got: {out}");
        assert!(out.contains("`Swap`(E1, E0)"), "got: {out}");
    }

    #[test]
    fn wi17_subline_perm_single_twin_vetoes() {
        let pre = "fn m() {\n    swap(alpha_arg, beta_arg) + 1;\n    swap(first_value, second_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    swap(alpha_arg, beta_arg) + 1;\n+    swap(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi17_subline_perm_single_different_arity_twin_keeps_firing() {
        let pre = "fn m() {\n    swap(alpha_arg, beta_arg) + 1;\n    swap(alpha_arg, beta_arg, gamma_arg);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    swap(alpha_arg, beta_arg) + 1;\n+    swap(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-arity twin must not veto");
        assert!(out.contains("- swap(E0, E1)\n"), "got: {out}");
    }

    #[test]
    fn wi17_subline_perm_single_whole_line_declines() {
        assert!(
            super::try_subline_perm_single(
                &["    swap(alpha_arg, beta_arg);"],
                &["    swap(beta_arg, alpha_arg);"],
                "f.rs",
                Some("fn m() {\n    swap(alpha_arg, beta_arg);\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi17_subline_perm_single_two_site_line_declines() {
        let pre = "fn m() {\n    swap(alpha_arg, beta_arg) + swap(gamma_arg, delta_arg);\n}\n";
        assert!(
            super::try_subline_perm_single(
                &["    swap(alpha_arg, beta_arg) + swap(gamma_arg, delta_arg);"],
                &["    swap(beta_arg, alpha_arg) + swap(delta_arg, gamma_arg);"],
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
    fn wi17_subline_perm_single_none_and_nonperm_veto() {
        assert!(
            super::try_subline_perm_single(
                &["    swap(alpha_arg, beta_arg) + 1;"],
                &["    swap(beta_arg, alpha_arg) + 1;"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "None pre-image must veto"
        );
        assert!(
            super::try_subline_perm_single(
                &["    swap(alpha_arg, beta_arg) + 1;"],
                &["    swap(beta_arg, gamma_arg) + 1;"],
                "f.rs",
                Some("fn m() {\n    swap(alpha_arg, beta_arg) + 1;\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "non-bijective delta must decline"
        );
    }

    #[test]
    fn wi18_subline_method_perm_single_fires_and_round_trips() {
        let pre = "fn m(svc: Svc) {\n    svc.configure(alpha_arg, beta_arg) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    svc.configure(alpha_arg, beta_arg) + 1;\n+    svc.configure(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique sub-line method perm single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- Receiver.configure(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.configure(E1, E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m(svc: Svc) {\n    svc.configure(beta_arg, alpha_arg) + 1;\n}\n"
        );
        let re = super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    svc.configure(alpha_arg, beta_arg) + 1;\n+    svc.configure(beta_arg, alpha_arg) + 1;\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi18_subline_method_perm_single_let_init_fires() {
        let pre = "fn m(svc: Svc) {\n    let x = svc.configure(alpha_arg, beta_arg) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = svc.configure(alpha_arg, beta_arg) + 1;\n+    let x = svc.configure(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init sub-line method perm single must fire");
        assert!(out.contains("- Receiver.configure(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.configure(E1, E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi18_subline_method_perm_single_uppercase_fires_literal() {
        let pre = "fn m(svc: Svc) {\n    svc.Configure(alpha_arg, beta_arg) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    svc.Configure(alpha_arg, beta_arg) + 1;\n+    svc.Configure(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("uppercase sub-line method perm single must fire");
        assert!(out.contains("Receiver.`Configure`(E0, E1)"), "got: {out}");
        assert!(out.contains("Receiver.`Configure`(E1, E0)"), "got: {out}");
    }

    #[test]
    fn wi18_subline_method_perm_single_twin_vetoes() {
        let pre = "fn m(svc: Svc) {\n    svc.configure(alpha_arg, beta_arg) + 1;\n    svc.configure(first_value, second_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    svc.configure(alpha_arg, beta_arg) + 1;\n+    svc.configure(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi18_subline_method_perm_single_different_arity_twin_keeps_firing() {
        let pre = "fn m(svc: Svc) {\n    svc.configure(alpha_arg, beta_arg) + 1;\n    svc.configure(alpha_arg, beta_arg, gamma_arg);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    svc.configure(alpha_arg, beta_arg) + 1;\n+    svc.configure(beta_arg, alpha_arg) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-arity twin must not veto");
        assert!(out.contains("- Receiver.configure(E0, E1)\n"), "got: {out}");
    }

    #[test]
    fn wi18_subline_method_perm_single_whole_line_declines() {
        assert!(
            super::try_subline_method_perm_single(
                &["    svc.configure(alpha_arg, beta_arg);"],
                &["    svc.configure(beta_arg, alpha_arg);"],
                "f.rs",
                Some("fn m(svc: Svc) {\n    svc.configure(alpha_arg, beta_arg);\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi18_subline_method_perm_single_two_site_line_declines() {
        let pre = "fn m(svc: Svc) {\n    svc.configure(alpha_arg, beta_arg) + svc.configure(gamma_arg, delta_arg);\n}\n";
        assert!(
            super::try_subline_method_perm_single(
                &["    svc.configure(alpha_arg, beta_arg) + svc.configure(gamma_arg, delta_arg);"],
                &["    svc.configure(beta_arg, alpha_arg) + svc.configure(delta_arg, gamma_arg);"],
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
    fn wi18_subline_method_perm_single_vetoes() {
        assert!(
            super::try_subline_method_perm_single(
                &["    svc.configure(alpha_arg, beta_arg) + 1;"],
                &["    svc.configure(beta_arg, alpha_arg) + 1;"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "None pre-image must veto"
        );
        assert!(
            super::try_subline_method_perm_single(
                &["    svc.configure(alpha_arg, beta_arg) + 1;"],
                &["    svc.configure(beta_arg, gamma_arg) + 1;"],
                "f.rs",
                Some("fn m(svc: Svc) {\n    svc.configure(alpha_arg, beta_arg) + 1;\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "non-bijective delta must decline"
        );
        assert!(
            super::try_subline_method_perm_single(
                &["    svc.configure(alpha_arg, beta_arg) + 1;"],
                &["    other.configure(beta_arg, alpha_arg) + 1;"],
                "f.rs",
                Some("fn m(svc: Svc) {\n    svc.configure(alpha_arg, beta_arg) + 1;\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "receiver delta must decline"
        );
        assert!(
            super::try_subline_method_perm_single(
                &["    svc.old_method(alpha_arg, beta_arg) + 1;"],
                &["    svc.new_method(beta_arg, alpha_arg) + 1;"],
                "f.rs",
                Some("fn m(svc: Svc) {\n    svc.old_method(alpha_arg, beta_arg) + 1;\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "method delta must decline"
        );
    }
}
