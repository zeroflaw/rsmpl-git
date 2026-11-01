//! Bare-call sweeps: callee-only rewrite + same-callee trailing-arg
//! addition (T447). Split from `super` (T468) to hold the 2000-line
//! hygiene gate.

use super::{call_parts, parse_call_expr, unique_rule};

/// Whole-arg permutation sweep (WI-6 slice 2, F6(a)): N ≥ 1 `-`/`+`
/// pairs whose every call subterm is a same-callee same-arity bare
/// call with one uniform non-trivial arg permutation across ALL
/// subterms of ALL lines (ordered value correspondence per Section
/// 15.7 — the sub-line form of the SPEC D.4 "plus templates may
/// permute holes" promise the whole-line `call_arg_template` sweep
/// already keeps). At least one pair must fail whole-line bare-call
/// parse (anti-shadow: pure whole-line classes belong to
/// `try_call_arg_template_sweep` later in the chain). Total subterm
/// sites must be ≥ 2 (t375: a single-site swap re-infers ordinary).
/// Totality is the real-pair engine-count check
/// (`template_pair_holds`): the emitted pair must match exactly the
/// subterm count in the pre-image — an untouched twin vetoes.
/// Detector name `call_arg_template` (same emission family).
pub(crate) fn try_call_perm_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.is_empty() {
        return None;
    }
    // All-identical multi-pair classes stay for the tighter verbatim
    // sweep (T422). A single pair is never "all-identical" in the
    // meaningful sense (minus always differs from plus, and verbatim
    // never claims single pairs on this path) — the subterm count gate
    // below owns singles.
    if minuses.len() >= 2 && super::subline::all_identical_pairs(minuses, pluses) {
        return None;
    }
    let mut cal: Option<String> = None;
    let mut arity: Option<usize> = None;
    let mut perm: Option<Vec<usize>> = None;
    let mut total = 0usize;
    let mut saw_subline_only = false;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        // Pure whole-line classes defer to `try_call_arg_template_sweep`:
        // a whole-line bare call is `parse_call_expr` + `call_parts`
        // (the former alone parses any `Expr`, including the Binary
        // lines this detector owns).
        let whole = |l: &str| parse_call_expr(l).and_then(|e| call_parts(&e)).is_some();
        if !whole(m) || !whole(p) {
            saw_subline_only = true;
        }
        let sites = super::subline::extract_arg_permutation_sites(m, p)?;
        for s in &sites {
            if cal.as_ref().is_some_and(|c| *c != s.cal) {
                return None;
            }
            if arity.is_some_and(|x| x != s.arity) {
                return None;
            }
            if perm.as_ref().is_some_and(|x| *x != s.perm) {
                return None;
            }
            cal = Some(s.cal.clone());
            arity = Some(s.arity);
            perm = Some(s.perm.clone());
            total += 1;
        }
    }
    if !saw_subline_only {
        return None;
    }
    let (cal, arity, perm) = (cal?, arity?, perm?);
    if total < 2 {
        return None;
    }
    let mut minus_args = Vec::with_capacity(arity);
    let mut plus_args = Vec::with_capacity(arity);
    for i in 0..arity {
        minus_args.push(format!("E{i}"));
    }
    for p in perm.iter().take(arity) {
        plus_args.push(format!("E{p}"));
    }
    // WI-7b parity: an uppercase callee rides backticks (literal), else
    // the engine infers it as an `expr` hole (unbound plus).
    let lit = super::shape::literal_callee(&cal);
    let minus_tpl = format!("{lit}({})", minus_args.join(", "));
    let plus_tpl = format!("{lit}({})", plus_args.join(", "));
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, total) {
        return None;
    }
    let rule = unique_rule(used_rules, "call_arg_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) bare-call rewrite with a callee-only delta:
/// every pair parses as `Old(args)` → `New(args)` with identical
/// old/new callees, uniform arity, and per-site textually identical
/// args. Emits one `# allow=many` hunk (tight original range, SPEC D.5).
pub(crate) fn try_call_sweep(
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
    let mut old_cal: Option<String> = None;
    let mut new_cal: Option<String> = None;
    let mut arity: Option<usize> = None;
    let mut whole_ok = true;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some((o, n, a)) = parse_call_pair(m, p) else {
            whole_ok = false;
            break;
        };
        if old_cal.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_cal.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        if arity.is_some_and(|x| x != a) {
            return None;
        }
        old_cal = Some(o);
        new_cal = Some(n);
        arity = Some(a);
    }
    if !whole_ok {
        old_cal = None;
        new_cal = None;
        arity = None;
    }
    if let (Some(old_cal), Some(new_cal), Some(arity)) = (old_cal, new_cal, arity) {
        if old_cal == new_cal {
            return None;
        }
        if !crate::totality::call_holds(file_old, &old_cal, arity, minuses, changed) {
            return None;
        }
        return Some(emit_call_sweep(
            path, used_rules, &old_cal, &new_cal, arity, changed,
        ));
    }
    // WI-4 sub-line fallback: no pair parsed whole-line (mixed
    // whole/sub lines veto — one shape per hunk). All-identical pairs
    // defer to the tighter verbatim sweep (T422); hetero pairs are
    // sub-line territory. Totality counts occurrences via the engine
    // probe.
    if super::subline::all_identical_pairs(minuses, pluses) {
        return None;
    }
    let mut sub_old: Option<String> = None;
    let mut sub_new: Option<String> = None;
    let mut sub_arity: Option<usize> = None;
    let mut total = 0usize;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let sites = super::subline::extract_bare_sites(m, p)?;
        for s in &sites {
            if sub_old.as_ref().is_some_and(|c| *c != s.old_cal) {
                return None;
            }
            if sub_new.as_ref().is_some_and(|c| *c != s.new_cal) {
                return None;
            }
            if sub_arity.is_some_and(|x| x != s.arity) {
                return None;
            }
            sub_old = Some(s.old_cal.clone());
            sub_new = Some(s.new_cal.clone());
            sub_arity = Some(s.arity);
            total += 1;
        }
    }
    let (old_cal, new_cal, arity) = (sub_old?, sub_new?, sub_arity?);
    if old_cal == new_cal || total < 2 {
        return None;
    }
    if !crate::totality::template_expr_holds(path, file_old, &call_template(&old_cal, arity), total)
    {
        return None;
    }
    Some(emit_call_sweep(
        path, used_rules, &old_cal, &new_cal, arity, changed,
    ))
}

/// WI-7 slice-1: whole-line bare-call callee-only single (N=1) under
/// `PrecisionUnique`. Fires iff the pair parses whole-line as
/// `Old(args)` → `New(args)` with identical args, `old != new`, a
/// pre-image is present (`None` vetoes — uniqueness needs the whole
/// file, D3), and `call_holds` proves exactly one `old_cal/arity`
/// site file-wide with zero opaque hits (conditions (a)+(b); (c) is
/// structural: the bare-call template always carries the callee token
/// beyond the `E` holes). Emission is the WI-19b allow-one spelling
/// (tight range, directive-free, default `allow=one`).
pub(crate) fn try_call_single(
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
    file_old?;
    let (old_cal, new_cal, arity) = parse_call_pair(minuses[0], pluses[0])?;
    if old_cal == new_cal {
        return None;
    }
    if !crate::totality::call_holds(file_old, &old_cal, arity, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_cal}_to_{new_cal}"));
    let mut vars = Vec::with_capacity(arity);
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let old_lit = super::shape::literal_callee(&old_cal);
    let new_lit = super::shape::literal_callee(&new_cal);
    // Allow-one: directive-free ranged hunk (59/77 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {old_lit}({args})\n+ {new_lit}({args})\n"));
    Some(hunk)
}

/// WI-7 slice-14: sub-line bare-call callee-only single (N=1) under
/// `PrecisionUnique`. Fires iff the pair does NOT parse whole-line as a
/// bare call (slice-1 owns that shape), but
/// `subline::extract_bare_sites` yields exactly one site with a real
/// callee delta and the frame rule holds (e.g. `foo(a) + 1` →
/// `bar(a) + 1`, `let`-init included via the shared `side_expr` entry).
/// Uniqueness is proved by `template_expr_holds` at n=1 over the
/// emitted minus template (same probe as the multi — one site
/// file-wide, zero opaque hits via the engine count); the T361
/// byte-compare pre-reservation (`single_reproduces`, slices-4–13
/// posture) vetoes any TokenStream spacing drift on the sub-expression
/// rewrite path. Whole-line pairs decline here (slice-1 owns them, even
/// when it vetoed — no bypass around its twin veto via a second
/// counter). Multi-subterm lines decline (`sites.len() == 1` only —
/// uniform two-site lines are multi territory by site count, even at
/// one pair). Emission is the slice-1 allow-one spelling (tight range,
/// directive-free, default `allow=one`). Detector name `call_sweep`
/// (same family, provenance unchanged).
pub(crate) fn try_subline_call_single(
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
    // Whole-line (and T449 `let`-init bare-call) pairs belong to the
    // slice-1 single — decline here even when it vetoed.
    if parse_call_pair(minuses[0], pluses[0]).is_some() {
        return None;
    }
    let sites = super::subline::extract_bare_sites(minuses[0], pluses[0])?;
    if sites.len() != 1 {
        return None;
    }
    let (old_cal, new_cal, arity) = (
        sites[0].old_cal.clone(),
        sites[0].new_cal.clone(),
        sites[0].arity,
    );
    if old_cal == new_cal {
        return None;
    }
    if !crate::totality::template_expr_holds(path, file_old, &call_template(&old_cal, arity), 1) {
        return None;
    }
    let mut vars = Vec::with_capacity(arity);
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let old_lit = super::shape::literal_callee(&old_cal);
    let new_lit = super::shape::literal_callee(&new_cal);
    let minus_tpl = format!("{old_lit}({args})");
    let plus_tpl = format!("{new_lit}({args})");
    // T361 byte gate (slices-4–13 posture): the sub-expression rewrite
    // must reproduce the native line swap byte-identical.
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_tpl, &plus_tpl,
    ) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_cal}_to_{new_cal}"));
    // Allow-one: directive-free ranged hunk (slice-1 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// Shared emission for the whole-line and sub-line bare-call paths
/// (identical hunks: the frame is byte-identical on both sides, so the
/// bare subterm rewrite reproduces the post-image).
fn emit_call_sweep(
    path: &str,
    used_rules: &mut Vec<String>,
    old_cal: &str,
    new_cal: &str,
    arity: usize,
    changed: &[usize],
) -> String {
    let rule = unique_rule(used_rules, &format!("calls_{old_cal}_to_{new_cal}"));
    let mut hunk = crate::emission_header(path, &rule, changed);
    let mut vars = Vec::new();
    for i in 0..arity {
        let v = format!("E{i}");
        vars.push(v);
    }
    let args = vars.join(", ");
    // WI-7b: uppercase constructor callees ride backticks (literal),
    // else the engine infers them as `expr` holes (unbound plus).
    let old_lit = super::shape::literal_callee(old_cal);
    let new_lit = super::shape::literal_callee(new_cal);
    hunk.push_str(&format!("- {old_lit}({args})\n+ {new_lit}({args})\n"));
    hunk
}

/// Minus template for the sub-line occurrence probe
/// (`template_expr_holds` counts engine matches of the bare pattern).
fn call_template(old_cal: &str, arity: usize) -> String {
    let mut vars = Vec::new();
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    format!("{old_cal}({})", vars.join(", "))
}

/// N-site (N >= 2) same-callee trailing-arg-addition call sweep (T447):
/// every pair parses as a bare single-segment no-turbofish call with
/// one shared callee, uniform minus arity `k`, and uniform plus arity
/// `k + t` (`t >= 1`); each site's minus args token-equal its plus-side
/// leading args (bound as `E0…`); the `t` trailing plus-only args are
/// token-equal concrete text across ALL sites and emit verbatim —
/// per-site trailing metavars would bind nothing on the minus side
/// (T442-illegal unbound emission), so hetero trailing additions
/// (09d3446's `symlink` vs `false`) stay ordinary. Pre-image totality
/// reuses `call_holds` over the minus arity (exact for this shape:
/// every old-arity call matches the minus pattern).
pub(crate) fn try_call_arg_add_sweep(
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
    use quote::ToTokens;
    let tok = |e: &syn::Expr| e.to_token_stream().to_string();
    let mut cal: Option<String> = None;
    let mut prefix: Option<usize> = None;
    let mut tail: Option<Vec<String>> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o_cal, o_args) = call_parts(&parse_call_expr(m)?)?;
        let (n_cal, n_args) = call_parts(&parse_call_expr(p)?)?;
        if o_cal != n_cal || n_args.len() <= o_args.len() {
            return None;
        }
        if cal.as_ref().is_some_and(|c| *c != o_cal) {
            return None;
        }
        if prefix.is_some_and(|k| k != o_args.len()) {
            return None;
        }
        if !o_args
            .iter()
            .zip(n_args.iter())
            .all(|(a, b)| tok(a) == tok(b))
        {
            return None;
        }
        let grown: Vec<String> = n_args[o_args.len()..].iter().map(tok).collect();
        if tail.as_ref().is_some_and(|t| *t != grown) {
            return None;
        }
        cal = Some(o_cal);
        prefix = Some(o_args.len());
        tail = Some(grown);
    }
    let (cal, k, tail) = (cal?, prefix?, tail?);
    if !crate::totality::call_holds(file_old, &cal, k, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{cal}_add_{}", tail.len()));
    let mut hunk = crate::emission_header(path, &rule, changed);
    let mut vars = Vec::new();
    for i in 0..k {
        let v = format!("E{i}");
        vars.push(v);
    }
    let head = vars.join(", ");
    let grown = tail.join(", ");
    // WI-7b: same literal-callee rule as the delta sweep — an uppercase
    // same callee would otherwise infer as an `expr` hole and over-match.
    let lit = super::shape::literal_callee(&cal);
    let (minus_call, plus_call) = if head.is_empty() {
        (format!("{lit}()"), format!("{lit}({grown})"))
    } else {
        (format!("{lit}({head})"), format!("{lit}({head}, {grown})"))
    };
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as bare calls; return
/// (old callee, new callee, arity) when the delta is callee-only.
fn parse_call_pair(old_line: &str, new_line: &str) -> Option<(String, String, usize)> {
    let old_expr = parse_call_expr(old_line)?;
    let new_expr = parse_call_expr(new_line)?;
    let (o_cal, o_args) = call_parts(&old_expr)?;
    let (n_cal, n_args) = call_parts(&new_expr)?;
    if o_args.len() != n_args.len() {
        return None;
    }
    for (a, b) in o_args.iter().zip(n_args.iter()) {
        use quote::ToTokens;
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    Some((o_cal, n_cal, o_args.len()))
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
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

    fn applied(pre: &HashMap<String, String>, patch: &str) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
        files
    }

    #[test]
    fn wi7_call_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-1: unique single `foo(1)` → `bar(1)` raises to an
        // allow-one hunk and real-applies byte-identical.
        let pre = "fn m() {\n    foo(1);\n}\n";
        let raw = section("f.rs", "@@ -2 +2 @@\n-    foo(1);\n+    bar(1);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("unique call single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(files["f.rs"], "fn m() {\n    bar(1);\n}\n");
    }

    #[test]
    fn wi7_call_single_twin_vetoes() {
        // An untouched same-callee twin anywhere vetoes (totality).
        let pre = "fn m() {\n    foo(1);\n    foo(9);\n}\n";
        let raw = section("f.rs", "@@ -2 +2 @@\n-    foo(1);\n+    bar(1);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi7_call_single_different_arity_twin_keeps_firing() {
        // Totality keys on callee+arity: a different-arity twin never
        // matches the emitted pattern and never vetoes.
        let pre = "fn m() {\n    foo(1);\n    foo(1, 2);\n}\n";
        let raw = section("f.rs", "@@ -2 +2 @@\n-    foo(1);\n+    bar(1);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("different-arity twin must not veto");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi7_call_single_no_pre_image_vetoes() {
        // No whole-file evidence → no guessing (D3).
        assert!(
            super::try_call_single(
                &["    foo(1);"],
                &["    bar(1);"],
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
    fn wi7_call_single_uppercase_fires_literal() {
        // Uppercase callees ride backticks (literal), else the engine
        // would infer them as `expr` holes.
        let pre = "fn m() {\n    Foo(1);\n}\n";
        let raw = section("f.rs", "@@ -2 +2 @@\n-    Foo(1);\n+    Bar(1);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("uppercase single must fire");
        assert!(out.contains("`Foo`(E0)"), "got: {out}");
        assert!(out.contains("`Bar`(E0)"), "got: {out}");
    }

    #[test]
    fn wi14_subline_call_single_fires_and_round_trips() {
        // WI-7 slice-14: unique sub-line single `foo(a) + 1` →
        // `bar(a) + 1` raises to an allow-one hunk and real-applies
        // byte-identical (fixed point `S″ == S′` checked below).
        let pre = "fn m() {\n    foo(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(alpha_argument) + 1;\n+    bar(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("unique sub-line call single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(files["f.rs"], "fn m() {\n    bar(alpha_argument) + 1;\n}\n");
        // Fixed point: re-diffing the applied tree re-emits the same
        // single (canonical `S″ == S′`).
        let re = infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    foo(alpha_argument) + 1;\n+    bar(alpha_argument) + 1;\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi14_subline_call_single_let_init_fires() {
        // `let`-init framing shares the `side_expr` entry.
        let pre = "fn m() {\n    let x = foo(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = foo(alpha_argument) + 1;\n+    let x = bar(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("let-init sub-line single must fire");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    let x = bar(alpha_argument) + 1;\n}\n"
        );
    }

    #[test]
    fn wi14_subline_call_single_uppercase_fires_literal() {
        let pre = "fn m() {\n    Foo(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    Foo(alpha_argument) + 1;\n+    Bar(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("uppercase sub-line single must fire");
        assert!(out.contains("`Foo`(E0)"), "got: {out}");
        assert!(out.contains("`Bar`(E0)"), "got: {out}");
    }

    #[test]
    fn wi14_subline_call_single_twin_vetoes() {
        // An untouched same-callee same-arity twin anywhere vetoes.
        let pre = "fn m() {\n    foo(alpha_argument) + 1;\n    foo(other_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(alpha_argument) + 1;\n+    bar(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi14_subline_call_single_different_arity_twin_keeps_firing() {
        let pre =
            "fn m() {\n    foo(alpha_argument) + 1;\n    foo(alpha_argument, beta_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(alpha_argument) + 1;\n+    bar(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("different-arity twin must not veto");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi14_subline_call_single_whole_line_declines() {
        // Whole-line pairs belong to the slice-1 arm, never this one.
        assert!(
            super::try_subline_call_single(
                &["    foo(1);"],
                &["    bar(1);"],
                "f.rs",
                Some("fn m() {\n    foo(1);\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi14_subline_call_single_two_site_line_declines() {
        // Uniform two-site lines are multi territory by site count, even
        // at one pair (the multi needs N>=2 pairs).
        let pre = "fn m() {\n    foo(alpha_argument) + foo(beta_value);\n}\n";
        assert!(
            super::try_subline_call_single(
                &["    foo(alpha_argument) + foo(beta_value);"],
                &["    bar(alpha_argument) + bar(beta_value);"],
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
    fn wi14_subline_call_single_no_pre_image_vetoes() {
        assert!(
            super::try_subline_call_single(
                &["    foo(alpha_argument) + 1;"],
                &["    bar(alpha_argument) + 1;"],
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
