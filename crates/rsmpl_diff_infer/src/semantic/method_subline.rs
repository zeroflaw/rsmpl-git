//! Sub-line method-to-method precision single (WI-7 slice-15, split
//! from `methods` per the D5 hygiene gate): N == 1 lifting of the
//! `try_method_to_method_sweep` WI-4 sub-line fallback under
//! `PrecisionUnique`.

use super::methods::{
    m2f_args, parse_method_same_literal_pair, parse_method_to_fn_pair, parse_method_to_method_pair,
};
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
) -> super::prov::Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return super::prov::Counted::Miss;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    // Whole-line pairs belong to the slice-2 single — decline here even
    // when it vetoed.
    if parse_method_to_method_pair(minuses[0], pluses[0]).is_some() {
        return super::prov::Counted::Miss;
    }
    let Some(sites) = super::subline::extract_method_sites(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    if sites.len() != 1 {
        return super::prov::Counted::Miss;
    }
    let (old_method, new_method, arity) = (
        sites[0].old_method.clone(),
        sites[0].new_method.clone(),
        sites[0].arity,
    );
    if old_method == new_method {
        return super::prov::Counted::Miss;
    }
    match crate::totality_counted::engine_verdict(
        file_old,
        crate::totality_counted::template_expr_counts(
            path,
            file_old,
            &format!("Receiver.{old_method}({})", m2f_args(arity)),
        ),
        1,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return super::prov::Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => {
            return super::prov::Counted::Opaque;
        }
        crate::totality_counted::FamilyVerdict::Unproven => {
            return super::prov::Counted::Miss;
        }
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
        return super::prov::Counted::Miss;
    }
    // T361 byte gate: the sub-expression rewrite must reproduce the
    // native line swap byte-identical.
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_tpl, &plus_tpl,
    ) {
        return super::prov::Counted::Miss;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_method}"));
    // Allow-one: directive-free ranged hunk (slice-2 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    super::prov::Counted::Fire(hunk)
}

/// WI-7 slice-16: sub-line method-to-function single (N=1) under
/// `PrecisionUnique`. Fires iff the pair does NOT parse whole-line as
/// a method-to-function refactor (slice-3 owns that shape), but
/// `subline::extract_method_fn_sites` yields exactly one site with a
/// receiver-prepend delta and the frame rule holds (e.g.
/// `val.old(alpha) + 1` → `new_func(val, alpha) + 1`, `let`-init
/// included via the shared `side_expr` entry). Uniqueness is proved by
/// `template_expr_holds` at n=1 over the emitted minus template
/// (same probe as the multi — one site file-wide, zero opaque hits
/// via the engine count); the T361 byte-compare pre-reservation
/// (`single_reproduces`, slices-4–15 posture) vetoes any TokenStream
/// spacing drift on the sub-expression rewrite path. Whole-line pairs
/// decline here (slice-3 owns them, even when it vetoed — no bypass
/// around its twin veto via a second counter). Multi-subterm lines
/// decline (`sites.len() == 1` only — uniform two-site lines are multi
/// territory by site count, even at one pair). Emission is the slice-3
/// allow-one spelling (tight range, directive-free, default
/// `allow=one`). Detector name `method_to_fn` (same family,
/// provenance unchanged).
pub(crate) fn try_subline_method_fn_single(
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
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    // Whole-line pairs belong to the slice-3 single — decline here even
    // when it vetoed.
    if parse_method_to_fn_pair(minuses[0], pluses[0]).is_some() {
        return super::prov::Counted::Miss;
    }
    let Some(sites) = super::subline::extract_method_fn_sites(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    if sites.len() != 1 {
        return super::prov::Counted::Miss;
    }
    let (old_method, new_func, arity) = (
        sites[0].old_method.clone(),
        sites[0].new_func.clone(),
        sites[0].arity,
    );
    match crate::totality_counted::engine_verdict(
        file_old,
        crate::totality_counted::template_expr_counts(
            path,
            file_old,
            &format!("Receiver.{old_method}({})", m2f_args(arity)),
        ),
        1,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return super::prov::Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => {
            return super::prov::Counted::Opaque;
        }
        crate::totality_counted::FamilyVerdict::Unproven => {
            return super::prov::Counted::Miss;
        }
    }
    let mut vars = Vec::with_capacity(arity);
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let tail = if args.is_empty() {
        "Receiver".to_string()
    } else {
        format!("Receiver, {args}")
    };
    let old_lit = super::shape::literal_callee(&old_method);
    let new_lit = super::shape::literal_callee(&new_func);
    let minus_tpl = format!("Receiver.{old_lit}({args})");
    let plus_tpl = format!("{new_lit}({tail})");
    // Defensive: both sides must re-parse as expressions (the totality
    // probe re-checks, but a non-parsing template must never leave this
    // core). The check runs on the raw idents: backticks are RSmPL-level
    // literal spelling (SPEC 15.4), not Rust syntax (TOOL-5 hygiene).
    // The plus side is a bare call with the receiver prepended.
    let raw_minus = format!("Receiver.{old_method}({args})");
    let raw_plus = format!("{new_func}({tail})");
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return super::prov::Counted::Miss;
    }
    // T361 byte gate: the sub-expression rewrite must reproduce the
    // native line swap byte-identical.
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_tpl, &plus_tpl,
    ) {
        return super::prov::Counted::Miss;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_func}"));
    // Allow-one: directive-free ranged hunk (slice-3 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    super::prov::Counted::Fire(hunk)
}

/// Shared emission tail of the method same-literal paths (whole-line
/// slice-13 and WI-7 slice-21 sub-line): file-wide uniqueness at n=1
/// over the emitted `(method, old_args)` tuple, the backtick method,
/// the raw-method re-parse check (TOOL-5), the T361 byte-compare
/// pre-reservation, and the allow-one emission. Returns `None` on any
/// veto; reserves the rule name only for emissions that reproduce.
/// `pub(crate)` so the whole-line single reuses the same tail (no
/// mirrored emission).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_method_literal_single(
    path: &str,
    pre: &str,
    file_old: Option<&str>,
    minus_line: &str,
    plus_line: &str,
    minuses: &[&str],
    changed: &[usize],
    o_method: &str,
    old_toks: &[String],
    new_toks: &[String],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    match crate::totality_counted::method_literal_verdict(
        file_old, o_method, old_toks, minuses, changed,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return super::prov::Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => {
            return super::prov::Counted::Opaque;
        }
        crate::totality_counted::FamilyVerdict::Unproven => {
            return super::prov::Counted::Miss;
        }
    }
    // Permuted tuples belong to the perm domain (slices 10/11/17/18):
    // a swap is a concrete-tuple delta too, so without this fence the
    // literal arm would steal perm-vetoed pairs (same twin the perm
    // template matches but the concrete tuple never counts). The fence
    // lives in the shared tail so whole-line and sub-line agree.
    let is_perm = {
        let mut o_sorted = old_toks.to_vec();
        let mut n_sorted = new_toks.to_vec();
        o_sorted.sort();
        n_sorted.sort();
        o_sorted == n_sorted
    };
    if is_perm {
        return super::prov::Counted::Miss;
    }
    let old = old_toks.join(", ");
    let new = new_toks.join(", ");
    // WI-7b: uppercase same method rides backticks (literal).
    let lit = super::shape::literal_callee(o_method);
    let minus_call = format!("Receiver.{lit}({old})");
    let plus_call = format!("Receiver.{lit}({new})");
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    // The check runs on the raw method: backticks are RSmPL-level
    // literal spelling (SPEC 15.4), not Rust syntax (TOOL-5).
    let raw_minus = format!("Receiver.{o_method}({old})");
    let raw_plus = format!("Receiver.{o_method}({new})");
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return super::prov::Counted::Miss;
    }
    if !super::general_expr::single_reproduces(
        path,
        pre,
        changed[0],
        minus_line,
        plus_line,
        &minus_call,
        &plus_call,
    ) {
        return super::prov::Counted::Miss;
    }
    let rule = unique_rule(used_rules, &format!("calls_{o_method}_literal"));
    // Allow-one: directive-free ranged hunk (slices-1-13 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    super::prov::Counted::Fire(hunk)
}

/// WI-7 slice-21: sub-line method same-literal single (N=1) under
/// `PrecisionUnique`. Fires iff the pair does NOT form a whole-line
/// method same-literal pair (slice-13 owns that shape — the emission
/// tail and the `method_literal_holds` counter are shared, so there is
/// no bypass around its twin veto via a second counter), but
/// `subline::extract_method_literal_sites` yields exactly one inner
/// site with a real arg-tuple delta and the frame rule holds (e.g.
/// `foo(svc.configure(alpha, 1));` -> `foo(svc.configure(alpha, 2));`,
/// `let`-init included via the shared `side_expr` entry). Unchanged
/// subterms skip (slice-19/20 discipline); a twin that WOULD match the
/// emitted template vetoes via the engine totality over the exact
/// old-args tuple. Multi-subterm lines decline (`sites.len() == 1`
/// only — uniform two-site lines are multi territory by site count,
/// even at one pair). Emission is the slice-13 allow-one spelling
/// (tight range, directive-free, default `allow=one`). Detector name
/// `method_same_literal` (same family, provenance unchanged).
pub(crate) fn try_subline_method_literal_single(
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
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    // Whole-line-owned pairs belong to the slice-13 single — decline
    // here even when it vetoed (shared counter, no bypass).
    if parse_method_same_literal_pair(minuses[0], pluses[0]).is_some() {
        return super::prov::Counted::Miss;
    }
    let Some(sites) = super::subline::extract_method_literal_sites(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    if sites.len() != 1 {
        return super::prov::Counted::Miss;
    }
    let site = &sites[0];
    emit_method_literal_single(
        path,
        pre,
        file_old,
        minuses[0],
        pluses[0],
        minuses,
        changed,
        &site.method,
        &site.old_args,
        &site.new_args,
        used_rules,
    )
}

/// WI-7 slice-22: sub-line method-arg-template wrap single (N=1) under
/// `PrecisionUnique`. Fires iff the pair does NOT form a whole-line
/// method wrap pair (slice-7 owns that shape — the emission tail and
/// the `template_pair_holds` counter are shared, so there is no bypass
/// around its twin veto via a second counter), but
/// `subline::extract_method_wrap_sites` yields exactly one inner site
/// with a real single-wrap delta and the frame rule holds (e.g.
/// `foo(svc.process(alpha_arg));` ->
/// `foo(svc.process(alpha_arg.clone()));`, `let`-init included via the
/// shared `side_expr` entry). Unchanged subterms skip (slice-19/20/21
/// discipline); pure-concrete deltas veto the line (literal territory —
/// slices 20/21 own them), permuted holes veto the line (perm
/// territory). A twin that WOULD match the emitted holed template
/// vetoes via the engine totality (which keys on method + arity, so any
/// same-method same-arity twin vetoes even with different args —
/// slice-7 precedent). Multi-subterm lines decline (`sites.len() == 1`
/// only — uniform two-site lines are multi territory by site count,
/// even at one pair). Emission is the slice-7 allow-one spelling
/// (tight range, directive-free, default `allow=one`). Detector name
/// `method_arg_template` (same family, provenance unchanged).
pub(crate) fn try_subline_method_wrap_single(
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
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    // Whole-line-owned pairs belong to the slice-7 single — decline
    // here even when it vetoed (shared counter, no bypass).
    if let (Some((o_recv, o_method, o_args)), Some((n_recv, n_method, n_args))) = (
        super::arg_template::parse_method_line(minuses[0]),
        super::arg_template::parse_method_line(pluses[0]),
    ) {
        if o_recv == n_recv
            && o_method == n_method
            && super::arg_template::antiunify_call_args_single(&o_args, &n_args).is_some()
        {
            return super::prov::Counted::Miss;
        }
    }
    let Some(sites) = super::subline::extract_method_wrap_sites(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    if sites.len() != 1 {
        return super::prov::Counted::Miss;
    }
    let site = &sites[0];
    // WI-7b: uppercase method rides backticks (literal).
    let lit = super::shape::literal_callee(&site.method);
    let minus_call = format!("Receiver.{lit}({})", site.minus_tpl.join(", "));
    let plus_call = format!("Receiver.{lit}({})", site.plus_tpl.join(", "));
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    // The check runs on the raw method: backticks are RSmPL-level
    // literal spelling (SPEC 15.4), not Rust syntax (TOOL-5).
    let raw_minus = format!("Receiver.{}({})", site.method, site.minus_tpl.join(", "));
    let raw_plus = format!("Receiver.{}({})", site.method, site.plus_tpl.join(", "));
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return super::prov::Counted::Miss;
    }
    match crate::totality_counted::engine_verdict(
        Some(pre),
        crate::totality_counted::template_pair_counts(path, Some(pre), &minus_call, &plus_call),
        1,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return super::prov::Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => {
            return super::prov::Counted::Opaque;
        }
        crate::totality_counted::FamilyVerdict::Unproven => {
            return super::prov::Counted::Miss;
        }
    }
    // T361 byte gate: the sub-expression rewrite must reproduce the
    // native line swap byte-identical.
    if !super::general_expr::single_reproduces(
        path,
        pre,
        changed[0],
        minuses[0],
        pluses[0],
        &minus_call,
        &plus_call,
    ) {
        return super::prov::Counted::Miss;
    }
    let rule = unique_rule(used_rules, &format!("calls_{}_wrap", site.method));
    // Allow-one: directive-free ranged hunk (slices-1-7 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    super::prov::Counted::Fire(hunk)
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
        // An untouched same-method same-arity twin anywhere vetoes the
        // HOLED sub-line template; the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    val.old_method(alpha_argument) + 1;\n    other.old_method(different_value);\n}\n";
        assert!(
            matches!(
                super::try_subline_method_single(
                    &["    val.old_method(alpha_argument) + 1;"],
                    &["    val.new_method(alpha_argument) + 1;"],
                    "f.rs",
                    Some(pre),
                    &[2],
                    &mut Vec::new()
                ),
                crate::semantic::prov::Counted::Totality { found: 2, want: 1 }
            ),
            "holed twin must veto at the core"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    val.new_method(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("literal verbatim single must fire");
        assert!(
            out.contains("- val.old_method(alpha_argument) + 1\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ val.new_method(alpha_argument) + 1\n"),
            "got: {out}"
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
            .is_miss(),
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
            .is_miss(),
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
            .is_miss(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi16_subline_method_fn_single_fires_and_round_trips() {
        // WI-7 slice-16: unique sub-line single
        // `val.old_method(a) + 1` → `new_func(val, a) + 1` raises to
        // an allow-one hunk and real-applies byte-identical (fixed
        // point `S″ == S′` checked below).
        let pre = "fn m() {\n    val.old_method(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    new_func(val, alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique sub-line m2f single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_func(Receiver, E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    new_func(val, alpha_argument) + 1;\n}\n"
        );
        // Fixed point: re-diffing the applied tree re-emits the same
        // single (canonical `S″ == S′`).
        let re = super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    new_func(val, alpha_argument) + 1;\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi16_subline_method_fn_single_let_init_fires() {
        // `let`-init framing shares the `side_expr` entry.
        let pre = "fn m() {\n    let x = val.old_method(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = val.old_method(alpha_argument) + 1;\n+    let x = new_func(val, alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init sub-line m2f single must fire");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_func(Receiver, E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi16_subline_method_fn_single_uppercase_fires_literal() {
        let pre = "fn m() {\n    val.OldMethod(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.OldMethod(alpha_argument) + 1;\n+    NewFunc(val, alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("uppercase sub-line m2f single must fire");
        assert!(out.contains("Receiver.`OldMethod`(E0)"), "got: {out}");
        assert!(out.contains("`NewFunc`(Receiver, E0)"), "got: {out}");
    }

    #[test]
    fn wi16_subline_method_fn_single_twin_vetoes() {
        // An untouched same-method same-arity twin anywhere vetoes the
        // HOLED sub-line template; the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    val.old_method(alpha_argument) + 1;\n    other.old_method(different_value);\n}\n";
        assert!(
            matches!(
                super::try_subline_method_fn_single(
                    &["    val.old_method(alpha_argument) + 1;"],
                    &["    new_func(val, alpha_argument) + 1;"],
                    "f.rs",
                    Some(pre),
                    &[2],
                    &mut Vec::new()
                ),
                crate::semantic::prov::Counted::Totality { found: 2, want: 1 }
            ),
            "holed twin must veto at the core"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    new_func(val, alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("literal verbatim single must fire");
        assert!(
            out.contains("- val.old_method(alpha_argument) + 1\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ new_func(val, alpha_argument) + 1\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi16_subline_method_fn_single_different_arity_twin_keeps_firing() {
        let pre =
            "fn m() {\n    val.old_method(alpha_argument) + 1;\n    val.old_method(alpha_argument, beta_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    val.old_method(alpha_argument) + 1;\n+    new_func(val, alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-arity twin must not veto");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi16_subline_method_fn_single_whole_line_declines() {
        // Whole-line pairs belong to the slice-3 arm, never this one.
        assert!(
            super::try_subline_method_fn_single(
                &["    val.old_method(alpha_argument);"],
                &["    new_func(val, alpha_argument);"],
                "f.rs",
                Some("fn m() {\n    val.old_method(alpha_argument);\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi16_subline_method_fn_single_two_site_line_declines() {
        // Uniform two-site lines are multi territory by site count, even
        // at one pair (the multi needs N>=2 pairs).
        let pre = "fn m() {\n    val.old_method(alpha_argument) + val.old_method(beta_value);\n}\n";
        assert!(
            super::try_subline_method_fn_single(
                &["    val.old_method(alpha_argument) + val.old_method(beta_value);"],
                &["    new_func(val, alpha_argument) + new_func(val, beta_value);"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "two-site line must decline"
        );
    }

    #[test]
    fn wi16_subline_method_fn_single_no_pre_image_vetoes() {
        assert!(
            super::try_subline_method_fn_single(
                &["    val.old_method(alpha_argument) + 1;"],
                &["    new_func(val, alpha_argument) + 1;"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_fires_and_round_trips() {
        // WI-7 slice-21: unique sub-line single
        // `foo(svc.configure(alpha_arg, 1));` ->
        // `foo(svc.configure(alpha_arg, 2));` raises to an allow-one
        // hunk and real-applies byte-identical (fixed point `S" == S'`).
        let pre = "fn m() {\n    foo(svc.configure(alpha_arg, 1));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.configure(alpha_arg, 1));\n+    foo(svc.configure(alpha_arg, 2));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique sub-line method literal single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(
            out.contains("- Receiver.configure(alpha_arg, 1)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.configure(alpha_arg, 2)\n"),
            "got: {out}"
        );
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    foo(svc.configure(alpha_arg, 2));\n}\n"
        );
        // Fixed point: re-diffing the applied tree re-emits the same
        // single (canonical `S" == S'`).
        let re = super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    foo(svc.configure(alpha_arg, 1));\n+    foo(svc.configure(alpha_arg, 2));\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi21_subline_method_literal_single_double_wrapped_fires() {
        // Double-wrapped framing: the outer wrappers skip, the inner
        // method literal site fires.
        let pre = "fn m() {\n    foo(bar(svc.configure(alpha_arg, 1)));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(svc.configure(alpha_arg, 1)));\n+    foo(bar(svc.configure(alpha_arg, 2)));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("double-wrapped sub-line method literal must fire");
        assert!(
            out.contains("- Receiver.configure(alpha_arg, 1)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.configure(alpha_arg, 2)\n"),
            "got: {out}"
        );
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi21_subline_method_literal_single_let_init_fires() {
        // `let`-init framing shares the `side_expr` entry.
        let pre = "fn m() {\n    let y = foo(svc.configure(alpha_arg, 1));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let y = foo(svc.configure(alpha_arg, 1));\n+    let y = foo(svc.configure(alpha_arg, 2));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init sub-line method literal must fire");
        assert!(
            out.contains("- Receiver.configure(alpha_arg, 1)\n"),
            "got: {out}"
        );
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi21_subline_method_literal_single_uppercase_fires_literal() {
        let pre = "fn m() {\n    foo(svc.Configure(alpha_arg, 1));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.Configure(alpha_arg, 1));\n+    foo(svc.Configure(alpha_arg, 2));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("uppercase sub-line method literal must fire");
        assert!(
            out.contains("Receiver.`Configure`(alpha_arg, 1)"),
            "got: {out}"
        );
        assert!(
            out.contains("Receiver.`Configure`(alpha_arg, 2)"),
            "got: {out}"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_twin_vetoes() {
        // An untouched same-method same-args twin anywhere vetoes the
        // HOLED sub-line template; the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    foo(svc.configure(alpha_arg, 1));\n    svc.configure(alpha_arg, 1);\n}\n";
        assert!(
            matches!(
                super::try_subline_method_literal_single(
                    &["    foo(svc.configure(alpha_arg, 1));"],
                    &["    foo(svc.configure(alpha_arg, 2));"],
                    "f.rs",
                    Some(pre),
                    &[2],
                    &mut Vec::new()
                ),
                crate::semantic::prov::Counted::Totality { found: 2, want: 1 }
            ),
            "holed twin must veto at the core"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.configure(alpha_arg, 1));\n+    foo(svc.configure(alpha_arg, 2));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("literal verbatim single must fire");
        assert!(
            out.contains("- foo(svc.configure(alpha_arg, 1))\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ foo(svc.configure(alpha_arg, 2))\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_different_args_twin_keeps_firing() {
        let pre = "fn m() {\n    foo(svc.configure(alpha_arg, 1));\n    svc.configure(other_arg, 9);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.configure(alpha_arg, 1));\n+    foo(svc.configure(alpha_arg, 2));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-args twin must not veto");
        assert!(
            out.contains("- Receiver.configure(alpha_arg, 1)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_whole_line_declines() {
        // Whole-line pairs belong to the slice-13 arm, never this one.
        assert!(
            super::try_subline_method_literal_single(
                &["    svc.configure(alpha_arg, 1);"],
                &["    svc.configure(alpha_arg, 2);"],
                "f.rs",
                Some("fn m() {\n    svc.configure(alpha_arg, 1);\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_receiver_delta_declines() {
        // Receiver deltas are m2f territory: the extractor vetoes.
        assert!(
            super::try_subline_method_literal_single(
                &["    foo(svc.configure(alpha_arg, 1));"],
                &["    foo(other.configure(alpha_arg, 2));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.configure(alpha_arg, 1));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "receiver delta must decline"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_method_delta_declines() {
        // Method deltas belong to the slice-15 arm, never this one.
        assert!(
            super::try_subline_method_literal_single(
                &["    foo(svc.configure(alpha_arg, 1));"],
                &["    foo(svc.setup(alpha_arg, 2));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.configure(alpha_arg, 1));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "method delta must decline"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_no_pre_image_vetoes() {
        assert!(
            super::try_subline_method_literal_single(
                &["    foo(svc.configure(alpha_arg, 1));"],
                &["    foo(svc.configure(alpha_arg, 2));"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_macro_vetoes() {
        assert!(
            super::try_subline_method_literal_single(
                &["    foo(svc.configure(m!(x), 1));"],
                &["    foo(svc.configure(m!(x), 2));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.configure(m!(x), 1));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "macro arg must veto"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_two_site_line_declines() {
        // Uniform two-site lines are multi territory by site count, even
        // at one pair (the multi needs N>=2 pairs).
        let pre = "fn m() {\n    foo(svc.configure(alpha_arg, 1)) + foo(svc.configure(alpha_arg, 1));\n}\n";
        assert!(
            super::try_subline_method_literal_single(
                &["    foo(svc.configure(alpha_arg, 1)) + foo(svc.configure(alpha_arg, 1));"],
                &["    foo(svc.configure(alpha_arg, 2)) + foo(svc.configure(alpha_arg, 2));"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "two-site line must decline"
        );
    }

    #[test]
    fn wi21_subline_method_literal_single_perm_declines() {
        // Permuted tuples belong to the perm domain (slices
        // 10/11/17/18): a swap is a concrete-tuple delta too, so the
        // literal arm fences it even when unique (pins the shared-tail
        // fence that keeps the slice-18 twin-veto ordinary).
        assert!(
            super::try_subline_method_literal_single(
                &["    foo(svc.configure(alpha_arg, beta_arg));"],
                &["    foo(svc.configure(beta_arg, alpha_arg));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.configure(alpha_arg, beta_arg));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "perm must decline"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_fires_and_round_trips() {
        // WI-7 slice-22: unique sub-line single
        // `foo(svc.process(alpha_arg));` ->
        // `foo(svc.process(alpha_arg.clone()));` raises to an allow-one
        // hunk and real-applies byte-identical (fixed point `S" == S'`).
        let pre = "fn m() {\n    foo(svc.process(alpha_arg));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.process(alpha_arg));\n+    foo(svc.process(alpha_arg.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique sub-line method wrap single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- Receiver.process(E0)\n"), "got: {out}");
        assert!(
            out.contains("+ Receiver.process(E0.clone())\n"),
            "got: {out}"
        );
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    foo(svc.process(alpha_arg.clone()));\n}\n"
        );
        // Fixed point: re-diffing the applied tree re-emits the same
        // single (canonical `S" == S'`).
        let re = super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    foo(svc.process(alpha_arg));\n+    foo(svc.process(alpha_arg.clone()));\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi22_subline_method_wrap_single_reference_fires_holed() {
        // Reference wraps (`a` -> `&a`) keep the method-call counts
        // balanced, so slice-21 used to claim them as concrete literal
        // singles — the holed wrap now beats the concrete spelling where
        // both prove the same site set (D10 upgrade pin).
        let pre = "fn m() {\n    foo(svc.process(alpha_arg));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.process(alpha_arg));\n+    foo(svc.process(&alpha_arg));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("reference sub-line method wrap must fire");
        assert!(out.contains("- Receiver.process(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.process(&E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi22_subline_method_wrap_single_binary_fires_holed() {
        // Binary wraps (`a` -> `a + 1`) likewise upgrade from the
        // slice-21 concrete spelling to the holed template.
        let pre = "fn m() {\n    foo(svc.process(alpha_arg));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.process(alpha_arg));\n+    foo(svc.process(alpha_arg + 1));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("binary sub-line method wrap must fire");
        assert!(out.contains("- Receiver.process(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.process(E0 + 1)\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi22_subline_method_wrap_single_double_wrapped_fires() {
        // Double-wrapped framing: the outer wrappers skip, the inner
        // method wrap site fires.
        let pre = "fn m() {\n    foo(bar(svc.process(alpha_arg)));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(svc.process(alpha_arg)));\n+    foo(bar(svc.process(alpha_arg.clone())));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("double-wrapped sub-line method wrap must fire");
        assert!(out.contains("- Receiver.process(E0)\n"), "got: {out}");
        assert!(
            out.contains("+ Receiver.process(E0.clone())\n"),
            "got: {out}"
        );
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi22_subline_method_wrap_single_let_init_fires() {
        // `let`-init framing shares the `side_expr` entry.
        let pre = "fn m() {\n    let y = foo(svc.process(alpha_arg));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let y = foo(svc.process(alpha_arg));\n+    let y = foo(svc.process(alpha_arg.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init sub-line method wrap must fire");
        assert!(out.contains("- Receiver.process(E0)\n"), "got: {out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi22_subline_method_wrap_single_uppercase_fires_literal() {
        let pre = "fn m() {\n    foo(svc.Process(alpha_arg));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.Process(alpha_arg));\n+    foo(svc.Process(alpha_arg.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("uppercase sub-line method wrap must fire");
        assert!(out.contains("Receiver.`Process`(E0)"), "got: {out}");
        assert!(out.contains("Receiver.`Process`(E0.clone())"), "got: {out}");
    }

    #[test]
    fn wi22_subline_method_wrap_single_twin_vetoes() {
        // Totality keys on method + arity (slice-7 precedent): an
        // untouched same-method same-arity twin vetoes the HOLED
        // template even when its args differ from the changed site;
        // the unique whole line raises as a WI-6-slice-1 literal
        // verbatim single (byte-probe verified).
        let pre = "fn m() {\n    foo(svc.process(alpha_arg));\n    svc.process(other_arg);\n}\n";
        assert!(
            matches!(
                super::try_subline_method_wrap_single(
                    &["    foo(svc.process(alpha_arg));"],
                    &["    foo(svc.process(alpha_arg.clone()));"],
                    "f.rs",
                    Some(pre),
                    &[2],
                    &mut Vec::new()
                ),
                crate::semantic::prov::Counted::Totality { found: 2, want: 1 }
            ),
            "holed twin must veto at the core"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.process(alpha_arg));\n+    foo(svc.process(alpha_arg.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("literal verbatim single must fire");
        assert!(
            out.contains("- foo(svc.process(alpha_arg))\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ foo(svc.process(alpha_arg.clone()))\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_different_arity_twin_keeps_firing() {
        let pre =
            "fn m() {\n    foo(svc.process(alpha_arg));\n    svc.process(alpha_arg, beta_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.process(alpha_arg));\n+    foo(svc.process(alpha_arg.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-arity twin must not veto");
        assert!(out.contains("- Receiver.process(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi22_subline_method_wrap_single_different_method_twin_keeps_firing() {
        let pre = "fn m() {\n    foo(svc.process(alpha_arg));\n    svc.setup(alpha_arg);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(svc.process(alpha_arg));\n+    foo(svc.process(alpha_arg.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-method twin must not veto");
        assert!(out.contains("- Receiver.process(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi22_subline_method_wrap_single_whole_line_declines() {
        // Whole-line pairs belong to the slice-7 arm, never this one.
        assert!(
            super::try_subline_method_wrap_single(
                &["    svc.process(alpha_arg);"],
                &["    svc.process(alpha_arg.clone());"],
                "f.rs",
                Some("fn m() {\n    svc.process(alpha_arg);\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_receiver_delta_declines() {
        // Receiver deltas are m2f territory: the extractor vetoes.
        assert!(
            super::try_subline_method_wrap_single(
                &["    foo(svc.process(alpha_arg));"],
                &["    foo(other.process(alpha_arg.clone()));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.process(alpha_arg));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "receiver delta must decline"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_method_delta_declines() {
        // Method deltas belong to the slice-15 arm, never this one.
        assert!(
            super::try_subline_method_wrap_single(
                &["    foo(svc.process(alpha_arg));"],
                &["    foo(svc.setup(alpha_arg.clone()));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.process(alpha_arg));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "method delta must decline"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_concrete_delta_declines() {
        // Pure-concrete deltas belong to the literal arms (slices
        // 20/21), never this one.
        assert!(
            super::try_subline_method_wrap_single(
                &["    foo(svc.configure(alpha_arg, 1));"],
                &["    foo(svc.configure(alpha_arg, 2));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.configure(alpha_arg, 1));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "concrete delta must decline"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_perm_declines() {
        // Permuted tuples belong to the perm domain (slices
        // 10/11/17/18).
        assert!(
            super::try_subline_method_wrap_single(
                &["    foo(svc.configure(alpha_arg, beta_arg));"],
                &["    foo(svc.configure(beta_arg, alpha_arg));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.configure(alpha_arg, beta_arg));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "perm must decline"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_no_pre_image_vetoes() {
        assert!(
            super::try_subline_method_wrap_single(
                &["    foo(svc.process(alpha_arg));"],
                &["    foo(svc.process(alpha_arg.clone()));"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_macro_vetoes() {
        assert!(
            super::try_subline_method_wrap_single(
                &["    foo(svc.process(m!(x)));"],
                &["    foo(svc.process(m!(x).clone()));"],
                "f.rs",
                Some("fn m() {\n    foo(svc.process(m!(x)));\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "macro arg must veto"
        );
    }

    #[test]
    fn wi22_subline_method_wrap_single_two_site_line_declines() {
        // Uniform two-site lines are multi territory by site count, even
        // at one pair (the multi needs N>=2 pairs).
        let pre = "fn m() {\n    foo(svc.process(alpha_arg)) + foo(svc.process(alpha_arg));\n}\n";
        assert!(
            super::try_subline_method_wrap_single(
                &["    foo(svc.process(alpha_arg)) + foo(svc.process(alpha_arg));"],
                &["    foo(svc.process(alpha_arg.clone())) + foo(svc.process(alpha_arg.clone()));"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_miss(),
            "two-site line must decline"
        );
    }
}
