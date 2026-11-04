//! Bare-call sweeps: callee-only rewrite + same-callee trailing-arg
//! addition (T447). Split from `super` (T468) to hold the 2000-line
//! hygiene gate.

use super::{call_parts, parse_call_expr, prov::Counted, unique_rule};

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
) -> Counted<String> {
    if minuses.len() != pluses.len() || minuses.is_empty() {
        return Counted::Miss;
    }
    // All-identical multi-pair classes stay for the tighter verbatim
    // sweep (T422). A single pair is never "all-identical" in the
    // meaningful sense (minus always differs from plus, and verbatim
    // never claims single pairs on this path) — the subterm count gate
    // below owns singles.
    if minuses.len() >= 2 && super::subline::all_identical_pairs(minuses, pluses) {
        return Counted::Miss;
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
        let sites = match super::subline::extract_arg_permutation_sites(m, p) {
            Some(s) => s,
            None => return Counted::Miss,
        };
        for s in &sites {
            if cal.as_ref().is_some_and(|c| *c != s.cal) {
                return Counted::Miss;
            }
            if arity.is_some_and(|x| x != s.arity) {
                return Counted::Miss;
            }
            if perm.as_ref().is_some_and(|x| *x != s.perm) {
                return Counted::Miss;
            }
            cal = Some(s.cal.clone());
            arity = Some(s.arity);
            perm = Some(s.perm.clone());
            total += 1;
        }
    }
    if !saw_subline_only {
        return Counted::Miss;
    }
    let (Some(cal), Some(arity), Some(perm)) = (cal, arity, perm) else {
        return Counted::Miss;
    };
    if total < 2 {
        return Counted::Miss;
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
    // WI-4 slice 2b-rest-a: the shape matched, so a failing probe names
    // its counts (`totality:`). No opaque arm by construction: the
    // engine never matches inside macros/strings.
    match crate::totality_counted::engine_verdict(
        file_old,
        crate::totality_counted::template_pair_counts(path, file_old, &minus_tpl, &plus_tpl),
        total,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let rule = unique_rule(used_rules, "call_arg_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Counted::Fire(hunk)
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
) -> Counted<String> {
    if minuses.len() < 2 {
        return Counted::Miss;
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
            return Counted::Miss;
        }
        if new_cal.as_ref().is_some_and(|c| *c != n) {
            return Counted::Miss;
        }
        if arity.is_some_and(|x| x != a) {
            return Counted::Miss;
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
            return Counted::Miss;
        }
        // WI-17 slice 4: the shape matched, so a failing probe names its
        // counts (`totality:`) or its opaque twin (`veto:opaque-hit`).
        match crate::totality::call_verdict(file_old, &old_cal, arity, minuses, changed) {
            crate::totality::CallVerdict::Holds => {}
            crate::totality::CallVerdict::CountMismatch { found, want } => {
                return Counted::Totality { found, want };
            }
            crate::totality::CallVerdict::OpaqueHit => return Counted::Opaque,
            crate::totality::CallVerdict::Unproven => return Counted::Miss,
        }
        return Counted::Fire(emit_call_sweep(
            path, used_rules, &old_cal, &new_cal, arity, changed,
        ));
    }
    // WI-4 sub-line fallback: no pair parsed whole-line (mixed
    // whole/sub lines veto — one shape per hunk). All-identical pairs
    // defer to the tighter verbatim sweep (T422); hetero pairs are
    // sub-line territory. Totality counts occurrences via the engine
    // probe.
    if super::subline::all_identical_pairs(minuses, pluses) {
        return Counted::Miss;
    }
    let mut sub_old: Option<String> = None;
    let mut sub_new: Option<String> = None;
    let mut sub_arity: Option<usize> = None;
    let mut total = 0usize;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        // WI-17 slice 4 residue: the sub-line probe stays bool-only, so
        // its totality failure keeps the `Miss` mapping (a counted
        // `template_expr` probe is a later slice).
        let Some(sites) = super::subline::extract_bare_sites(m, p) else {
            return Counted::Miss;
        };
        for s in &sites {
            if sub_old.as_ref().is_some_and(|c| *c != s.old_cal) {
                return Counted::Miss;
            }
            if sub_new.as_ref().is_some_and(|c| *c != s.new_cal) {
                return Counted::Miss;
            }
            if sub_arity.is_some_and(|x| x != s.arity) {
                return Counted::Miss;
            }
            sub_old = Some(s.old_cal.clone());
            sub_new = Some(s.new_cal.clone());
            sub_arity = Some(s.arity);
            total += 1;
        }
    }
    let (Some(old_cal), Some(new_cal), Some(arity)) = (sub_old, sub_new, sub_arity) else {
        return Counted::Miss;
    };
    if old_cal == new_cal || total < 2 {
        return Counted::Miss;
    }
    if !crate::totality::template_expr_holds(path, file_old, &call_template(&old_cal, arity), total)
    {
        return Counted::Miss;
    }
    Counted::Fire(emit_call_sweep(
        path, used_rules, &old_cal, &new_cal, arity, changed,
    ))
}

/// WI-7 slice-1: whole-line bare-call callee-only single (N=1) under
/// `PrecisionUnique`. Fires iff the pair parses whole-line as
/// `Old(args)` → `New(args)` with identical args, `old != new`, a
/// pre-image is present (`None` vetoes — uniqueness needs the whole
/// file, D3), and `call_verdict` proves exactly one `old_cal/arity`
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
) -> Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return Counted::Miss;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    if file_old.is_none() {
        return Counted::Miss;
    }
    let Some((old_cal, new_cal, arity)) = parse_call_pair(minuses[0], pluses[0]) else {
        return Counted::Miss;
    };
    if old_cal == new_cal {
        return Counted::Miss;
    }
    // WI-17 slice 4: the shape matched, so a failing probe names its
    // counts (`totality:`) or its opaque twin (`veto:opaque-hit`).
    match crate::totality::call_verdict(file_old, &old_cal, arity, minuses, changed) {
        crate::totality::CallVerdict::Holds => {}
        crate::totality::CallVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality::CallVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality::CallVerdict::Unproven => return Counted::Miss,
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
    Counted::Fire(hunk)
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

/// WI-7 slice-23: sub-line bare-arg-template wrap single (N=1) under
/// `PrecisionUnique`. Fires iff the pair does NOT form a whole-line
/// bare wrap pair (slice-6 owns that shape via the same parser plus
/// the shared core, so there is no bypass around its twin veto via a
/// second counter), but `subline::extract_bare_wrap_sites` yields
/// exactly one site — a same-level wrap (`foo(alpha)` →
/// `foo(bar(alpha))` inside a frame) or a deep-inner wrap
/// (`foo(bar(alpha))` → `foo(bar(alpha.clone()))` where the outer
/// wrapper is frame) — and the frame rule holds. Uniqueness is proved
/// by `template_pair_holds` at n=1 over the emitted holed pair (same
/// probe as the slice-6 multi — one site file-wide, zero opaque hits;
/// twin discipline keys on callee+arity: any same-callee same-arity
/// twin vetoes, even with different args — slice-6 precedent). The
/// T361 byte-compare pre-reservation (`single_reproduces`, shared with
/// the general-expr singles) vetoes any TokenStream spacing drift.
/// Callee deltas stay with the slice-14 single; pure-concrete deltas
/// stay with the literal singles; permuted holes stay with the perm
/// singles (all veto in the extractor — never partially claimed).
/// Multi-subterm lines decline (`sites.len() == 1` only — uniform
/// two-site lines are multi territory by site count, even at one
/// pair). Emission is the slice-6 allow-one spelling (tight range,
/// directive-free, default `allow=one`). Detector name
/// `call_arg_template` (same family, provenance unchanged).
pub(crate) fn try_subline_bare_wrap_single(
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
    // Whole-line-owned pairs belong to the slice-6 single — decline
    // here even when it vetoed (shared counter, no bypass). Callee
    // deltas never parse here as owned (the extractor vetoes them).
    if let Some((o_cal, n_cal, o_args, n_args)) =
        super::arg_template::parse_bare_pair(minuses[0], pluses[0])
    {
        if o_cal == n_cal
            && super::arg_template::antiunify_call_args_single(&o_args, &n_args).is_some()
        {
            return None;
        }
    }
    let sites = super::subline::extract_bare_wrap_sites(minuses[0], pluses[0])?;
    if sites.len() != 1 {
        return None;
    }
    let site = &sites[0];
    // WI-7b: uppercase callee rides backticks (literal), else it
    // infers as an `expr` hole and over-matches.
    let lit = super::shape::literal_callee(&site.cal);
    let minus_call = format!("{lit}({})", site.minus_tpl.join(", "));
    let plus_call = format!("{lit}({})", site.plus_tpl.join(", "));
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    // The check runs on the raw callee: backticks are RSmPL-level
    // literal spelling (SPEC 15.4), not Rust syntax (TOOL-5).
    let raw_minus = format!("{}({})", site.cal, site.minus_tpl.join(", "));
    let raw_plus = format!("{}({})", site.cal, site.plus_tpl.join(", "));
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return None;
    }
    if !crate::totality::template_pair_holds(path, Some(pre), &minus_call, &plus_call, 1) {
        return None;
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
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{}_wrap", site.cal));
    // Allow-one: directive-free ranged hunk (slices-1-6 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
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
/// reuses `call_verdict` over the minus arity (exact for this shape:
/// every old-arity call matches the minus pattern).
pub(crate) fn try_call_arg_add_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() < 2 {
        return Counted::Miss;
    }
    use super::pat_template::site_spelling;
    use quote::ToTokens;
    let tok = |e: &syn::Expr| e.to_token_stream().to_string();
    let mut cal: Option<String> = None;
    let mut prefix: Option<usize> = None;
    let mut tail: Option<Vec<String>> = None;
    // WI-3 first-site grown-arg nodes: the plus tail replays the first
    // plus line's source bytes (span-sliced), never `TokenStream`
    // spacing (`&cand.body_text`, not `& cand . body_text`).
    let mut first_grown: Option<Vec<syn::Expr>> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some(o_expr) = parse_call_expr(m) else {
            return Counted::Miss;
        };
        let Some((o_cal, o_args)) = call_parts(&o_expr) else {
            return Counted::Miss;
        };
        let Some(n_expr) = parse_call_expr(p) else {
            return Counted::Miss;
        };
        let Some((n_cal, n_args)) = call_parts(&n_expr) else {
            return Counted::Miss;
        };
        if o_cal != n_cal || n_args.len() <= o_args.len() {
            return Counted::Miss;
        }
        if cal.as_ref().is_some_and(|c| *c != o_cal) {
            return Counted::Miss;
        }
        if prefix.is_some_and(|k| k != o_args.len()) {
            return Counted::Miss;
        }
        if !o_args
            .iter()
            .zip(n_args.iter())
            .all(|(a, b)| tok(a) == tok(b))
        {
            return Counted::Miss;
        }
        let grown_nodes: Vec<syn::Expr> = n_args[o_args.len()..].to_vec();
        let grown: Vec<String> = grown_nodes.iter().map(tok).collect();
        if tail.as_ref().is_some_and(|t| *t != grown) {
            return Counted::Miss;
        }
        if first_grown.is_none() {
            first_grown = Some(grown_nodes);
        }
        cal = Some(o_cal);
        prefix = Some(o_args.len());
        tail = Some(grown);
    }
    let (Some(cal), Some(k), Some(tail)) = (cal, prefix, tail) else {
        return Counted::Miss;
    };
    let first_grown = first_grown.unwrap_or_default();
    // WI-17 slice 4: the shape matched, so a failing probe names its
    // counts (`totality:`) or its opaque twin (`veto:opaque-hit`).
    match crate::totality::call_verdict(file_old, &cal, k, minuses, changed) {
        crate::totality::CallVerdict::Holds => {}
        crate::totality::CallVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality::CallVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality::CallVerdict::Unproven => return Counted::Miss,
    }
    let rule = unique_rule(used_rules, &format!("calls_{cal}_add_{}", tail.len()));
    let mut hunk = crate::emission_header(path, &rule, changed);
    let mut vars = Vec::new();
    for i in 0..k {
        let v = format!("E{i}");
        vars.push(v);
    }
    let head = vars.join(", ");
    // WI-3: grown tails replay the first plus line's bytes. Spans from
    // both `parse_call_expr` paths (stripped whole-line text, line
    // `Stmt` parse) share the trimmed line's leading columns, so the
    // first plus line slices every site's tail (uniform by the loop
    // above); any irregular span keeps its `tok` spelling.
    let grown = first_grown
        .iter()
        .zip(tail.iter())
        .map(|(node, fb)| site_spelling(pluses[0], node, fb))
        .collect::<Vec<_>>()
        .join(", ");
    // WI-7b: same literal-callee rule as the delta sweep — an uppercase
    // same callee would otherwise infer as an `expr` hole and over-match.
    let lit = super::shape::literal_callee(&cal);
    let (minus_call, plus_call) = if head.is_empty() {
        (format!("{lit}()"), format!("{lit}({grown})"))
    } else {
        (format!("{lit}({head})"), format!("{lit}({head}, {grown})"))
    };
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    Counted::Fire(hunk)
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
    fn wi3_arg_add_grown_tail_replays_verbatim_spacing() {
        // WI-3 audit: a grown `&cand.body_text` tail replays the first
        // plus line's source bytes, never TokenStream spacing
        // (`& cand . body_text`). Before the fix this class failed the
        // reprobe byte-compare and degraded to ordinary.
        let pre = "fn m() {\n    foo(a);\n    foo(b);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    foo(a);\n-    foo(b);\n+    foo(a, &cand.body_text);\n+    foo(b, &cand.body_text);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("arg-add sweep must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ foo(E0, &cand.body_text)\n"), "got: {out}");
        assert!(!out.contains("& cand"), "token spacing leaked: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    foo(a, &cand.body_text);\n    foo(b, &cand.body_text);\n}\n"
        );
    }

    #[test]
    fn wi7_call_single_twin_vetoes() {
        // An untouched same-callee twin anywhere vetoes the HOLED
        // template (totality); the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        // WI-17 slice 4: the veto names both counts, not `no-match`.
        let pre = "fn m() {\n    foo(1);\n    foo(9);\n}\n";
        assert!(
            matches!(
                super::try_call_single(
                    &["    foo(1);"],
                    &["    bar(1);"],
                    "f.rs",
                    Some(pre),
                    &[2],
                    &mut Vec::new()
                ),
                super::Counted::Totality { found: 2, want: 1 }
            ),
            "holed twin must veto with both counts at the core"
        );
        let raw = section("f.rs", "@@ -2 +2 @@\n-    foo(1);\n+    bar(1);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("literal verbatim single must fire");
        assert!(out.contains("- foo(1)\n"), "got: {out}");
        assert!(out.contains("+ bar(1)\n"), "got: {out}");
    }

    #[test]
    fn wi17_call_sweep_twin_names_totality_in_provenance() {
        // WI-17 slice 4: a 2-pair callee-only sweep with an untouched
        // twin stays wholly ordinary, and the chain records the counted
        // decline (`totality:3:2`) instead of `no-match`. Hetero args
        // defeat the literal/verbatim arms and the twin defeats the
        // general arm, so the ordinary entry carries the wire visibly.
        let pre = "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n     foo(1);\n-    foo(2);\n-    foo(3);\n+    bar(2);\n+    bar(3);\n }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let mut prov = Vec::new();
        let opt = super::super::infer_semantic_with_provenance(
            std::slice::from_ref(&raw),
            &pre_map,
            &mut prov,
        );
        assert!(opt.is_none(), "twin sweep stays ordinary, got: {opt:?}");
        assert_eq!(prov.len(), 1, "one ordinary entry, got: {prov:?}");
        assert!(
            prov[0]
                .declined
                .iter()
                .any(|d| d.detector == "call_sweep" && d.reason == "totality:3:2"),
            "counted decline named, got: {:?}",
            prov[0].declined
        );
        for d in &prov[0].declined {
            assert!(
                super::super::prov::reason_is_closed(&d.reason),
                "open reason: {d:?}"
            );
        }
    }

    #[test]
    fn wi17_call_sweep_opaque_twin_names_veto_in_provenance() {
        // WI-17 slice 4: a count-exact sweep whose needle hides in a
        // macro on a changed line stays wholly ordinary (existing
        // totality pin), and the chain records `veto:opaque-hit` under
        // `call_sweep` instead of `no-match`.
        let pre = "fn m() {\n    foo(1);\n    g(m!(foo(1)));\n    foo(1);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(1);\n+    bar(1);\n+    bar(1);\n }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let mut prov = Vec::new();
        let opt = super::super::infer_semantic_with_provenance(
            std::slice::from_ref(&raw),
            &pre_map,
            &mut prov,
        );
        assert!(opt.is_none(), "opaque twin stays ordinary, got: {opt:?}");
        assert_eq!(prov.len(), 1, "one ordinary entry, got: {prov:?}");
        assert!(
            prov[0]
                .declined
                .iter()
                .any(|d| d.detector == "call_sweep" && d.reason == "veto:opaque-hit"),
            "opaque veto named, got: {:?}",
            prov[0].declined
        );
        for d in &prov[0].declined {
            assert!(
                super::super::prov::reason_is_closed(&d.reason),
                "open reason: {d:?}"
            );
        }
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
        // No whole-file evidence → no guessing (D3). WI-17 slice 4:
        // unproven pre-images keep the `Miss` (`no-match`) mapping —
        // no counts to name.
        assert!(
            matches!(
                super::try_call_single(
                    &["    foo(1);"],
                    &["    bar(1);"],
                    "f.rs",
                    None,
                    &[2],
                    &mut Vec::new()
                ),
                super::Counted::Miss
            ),
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
        // An untouched same-callee same-arity twin anywhere vetoes the
        // HOLED sub-line template; the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    foo(alpha_argument) + 1;\n    foo(other_value);\n}\n";
        assert!(
            super::try_subline_call_single(
                &["    foo(alpha_argument) + 1;"],
                &["    bar(alpha_argument) + 1;"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "holed twin must veto at the core"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(alpha_argument) + 1;\n+    bar(alpha_argument) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("literal verbatim single must fire");
        assert!(out.contains("- foo(alpha_argument) + 1\n"), "got: {out}");
        assert!(out.contains("+ bar(alpha_argument) + 1\n"), "got: {out}");
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

    #[test]
    fn wi23_subline_bare_wrap_deep_clone_fires_and_round_trips() {
        // WI-7 slice-23: unique sub-line deep-inner single
        // `foo(bar(alpha_argument));` → `foo(bar(alpha_argument.clone()));`
        // raises to an allow-one hunk and real-applies byte-identical
        // (fixed point `S″ == S′` checked below).
        let pre = "fn m() {\n    foo(bar(alpha_argument));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(alpha_argument.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("unique sub-line bare-wrap single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- bar(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0.clone())\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    foo(bar(alpha_argument.clone()));\n}\n"
        );
        // Fixed point: re-diffing the applied tree re-emits the same
        // single (canonical `S″ == S′`).
        let re = infer_semantic_with_pre_images(
            std::slice::from_ref(&section(
                "f.rs",
                "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(alpha_argument.clone()));\n",
            )),
            &pre_map,
        )
        .expect("re-raise must fire");
        assert_eq!(out, re);
    }

    #[test]
    fn wi23_subline_bare_wrap_deep_reference_fires_holed() {
        // Reference wrappers fire holed (the slice-20 concrete spelling
        // never owns this pair — zero-hole skip).
        let pre = "fn m() {\n    foo(bar(alpha_argument));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(&alpha_argument));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("deep reference wrap must fire");
        assert!(out.contains("- bar(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(&E0)\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    foo(bar(&alpha_argument));\n}\n"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_deep_binary_fires_holed() {
        // Binary wrappers fire holed through the shared single core.
        let pre = "fn m() {\n    foo(bar(alpha_argument));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(alpha_argument + 1));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("deep binary wrap must fire");
        assert!(out.contains("- bar(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0 + 1)\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    foo(bar(alpha_argument + 1));\n}\n"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_deep_bare_fires_holed() {
        // Bare-call wrappers fire holed through the shared single core.
        let pre = "fn m() {\n    foo(bar(alpha_argument));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(baz(alpha_argument)));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("deep bare wrap must fire");
        assert!(out.contains("- bar(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(baz(E0))\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    foo(bar(baz(alpha_argument)));\n}\n"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_same_level_fires() {
        // Same-level wrap inside a frame: `foo(alpha)` →
        // `foo(bar(alpha))` where the binary frame keeps the whole-line
        // single from parsing.
        let pre = "fn m() {\n    let x = foo(alpha_argument) + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = foo(alpha_argument) + 1;\n+    let x = foo(bar(alpha_argument)) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("same-level sub-line bare-wrap must fire");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ foo(bar(E0))\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    let x = foo(bar(alpha_argument)) + 1;\n}\n"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_let_init_deep_fires() {
        // `let`-init framing shares the `side_expr` entry.
        let pre = "fn m() {\n    let x = foo(bar(alpha_argument));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = foo(bar(alpha_argument));\n+    let x = foo(bar(alpha_argument.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("let-init deep bare-wrap must fire");
        assert!(out.contains("- bar(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0.clone())\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    let x = foo(bar(alpha_argument.clone()));\n}\n"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_uppercase_fires_literal() {
        // Uppercase callees ride backticks (literal), else the engine
        // would infer them as `expr` holes.
        let pre = "fn m() {\n    foo(Bar(alpha_argument));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(Bar(alpha_argument));\n+    foo(Bar(alpha_argument.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("uppercase deep bare-wrap must fire");
        assert!(out.contains("`Bar`(E0)"), "got: {out}");
        assert!(out.contains("`Bar`(E0.clone())"), "got: {out}");
    }

    #[test]
    fn wi23_subline_bare_wrap_twin_vetoes() {
        // An untouched same-callee same-arity twin anywhere vetoes the
        // HOLED wrap (slice-6 totality precedent); the unique whole line
        // raises as a WI-6-slice-1 literal verbatim single.
        let pre = "fn m() {\n    foo(bar(alpha_argument));\n    bar(different_value);\n}\n";
        assert!(
            super::try_subline_bare_wrap_single(
                &["    foo(bar(alpha_argument));"],
                &["    foo(bar(alpha_argument.clone()));"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "holed twin must veto at the core"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(alpha_argument.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("literal verbatim single must fire");
        assert!(out.contains("- foo(bar(alpha_argument))\n"), "got: {out}");
        assert!(
            out.contains("+ foo(bar(alpha_argument.clone()))\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_different_arity_twin_keeps_firing() {
        // Totality keys on callee+arity: a different-arity twin never
        // matches the emitted pattern and never vetoes.
        let pre =
            "fn m() {\n    foo(bar(alpha_argument));\n    bar(alpha_argument, beta_value);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(alpha_argument.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("different-arity twin must not veto");
        assert!(out.contains("- bar(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi23_subline_bare_wrap_different_callee_twin_keeps_firing() {
        // A different-callee twin never matches the emitted pattern.
        let pre = "fn m() {\n    foo(bar(alpha_argument));\n    qux(alpha_argument);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(alpha_argument.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("different-callee twin must not veto");
        assert!(out.contains("- bar(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi23_subline_bare_wrap_whole_line_declines() {
        // Whole-line pairs belong to the slice-6 arm, never this one.
        assert!(
            super::try_subline_bare_wrap_single(
                &["    bar(alpha_argument);"],
                &["    bar(alpha_argument.clone());"],
                "f.rs",
                Some("fn m() {\n    bar(alpha_argument);\n}\n"),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "whole-line pair must decline"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_callee_delta_declines() {
        // Callee deltas are slice-14 territory, never partially
        // claimed by the wrap core; the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    foo(bar(alpha_argument)) + 1;\n}\n";
        assert!(
            super::try_subline_bare_wrap_single(
                &["    foo(bar(alpha_argument)) + 1;"],
                &["    foo(qux(alpha_argument)) + 1;"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "wrap core must decline callee deltas"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument)) + 1;\n+    foo(qux(alpha_argument)) + 1;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("literal verbatim single must fire");
        assert!(
            out.contains("- foo(bar(alpha_argument)) + 1\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ foo(qux(alpha_argument)) + 1\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_pure_concrete_declines() {
        // Pure-concrete deltas are literal territory (slice-20 owns the
        // spelling where it fires; here it vetoes as zero-hole); the
        // unique whole line raises as a WI-6-slice-1 literal verbatim
        // single (byte-probe verified).
        let pre = "fn m() {\n    foo(bar(alpha_argument));\n}\n";
        assert!(
            super::try_subline_bare_wrap_single(
                &["    foo(bar(alpha_argument));"],
                &["    foo(bar(beta_value));"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "wrap core must decline pure-concrete deltas"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(beta_value));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("literal verbatim single must fire");
        assert!(out.contains("- foo(bar(alpha_argument))\n"), "got: {out}");
        assert!(out.contains("+ foo(bar(beta_value))\n"), "got: {out}");
    }

    #[test]
    fn wi23_subline_bare_wrap_perm_declines() {
        // Permuted holes are perm territory (slice-17), never claimed
        // by the wrap core; the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    foo(bar(alpha_argument, beta_value));\n}\n";
        assert!(
            super::try_subline_bare_wrap_single(
                &["    foo(bar(alpha_argument, beta_value));"],
                &["    foo(bar(beta_value, alpha_argument));"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "wrap core must decline permuted holes"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument, beta_value));\n+    foo(bar(beta_value, alpha_argument));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("literal verbatim single must fire");
        assert!(
            out.contains("- foo(bar(alpha_argument, beta_value))\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ foo(bar(beta_value, alpha_argument))\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_no_pre_image_vetoes() {
        // No whole-file evidence → no guessing (D3).
        assert!(
            super::try_subline_bare_wrap_single(
                &["    foo(bar(alpha_argument));"],
                &["    foo(bar(alpha_argument.clone()));"],
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
    fn wi23_subline_bare_wrap_macro_vetoes() {
        // Macro-hiding args would inline unmatchable text (the engine
        // never binds inside macro bodies).
        let pre = "fn m() {\n    foo(bar(alpha_argument));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_argument));\n+    foo(bar(concat!(alpha_argument)));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).is_none(),
            "macro arg must veto"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_two_site_line_declines() {
        // Uniform two-site lines are multi territory by site count for
        // the WRAP core, even at one pair (the multi needs N>=2 pairs);
        // the unique whole line raises as a WI-6-slice-1 literal
        // verbatim single (byte-probe verified).
        let pre = "fn m() {\n    foo(bar(alpha_value)) + foo(bar(beta_value));\n}\n";
        assert!(
            super::try_subline_bare_wrap_single(
                &["    foo(bar(alpha_value)) + foo(bar(beta_value));"],
                &["    foo(bar(alpha_value.clone())) + foo(bar(beta_value.clone()));"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "wrap core must decline two-site lines"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(bar(alpha_value)) + foo(bar(beta_value));\n+    foo(bar(alpha_value.clone())) + foo(bar(beta_value.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("literal verbatim single must fire");
        assert!(
            out.contains("- foo(bar(alpha_value)) + foo(bar(beta_value))\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ foo(bar(alpha_value.clone())) + foo(bar(beta_value.clone()))\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_hetero_outer_inner_stays_ordinary() {
        // A same-level wrap plus a deep-inner wrap on one line: the
        // frame check vetoes the partial sub-site claim (never
        // partially claimed); the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    foo(alpha_argument) + foo(bar(beta_value));\n}\n";
        assert!(
            super::try_subline_bare_wrap_single(
                &["    foo(alpha_argument) + foo(bar(beta_value));"],
                &["    foo(bar(alpha_argument)) + foo(bar(beta_value.clone()));"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "wrap core must decline hetero partial claims"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo(alpha_argument) + foo(bar(beta_value));\n+    foo(bar(alpha_argument)) + foo(bar(beta_value.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("literal verbatim single must fire");
        assert!(
            out.contains("- foo(alpha_argument) + foo(bar(beta_value))\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ foo(bar(alpha_argument)) + foo(bar(beta_value.clone()))\n"),
            "got: {out}"
        );
    }

    #[test]
    fn wi23_subline_bare_wrap_method_frame_fires() {
        // Method-call frames are transparent: the inner bare wrap
        // sites while the outer method descends as frame (slice-22
        // declines the pair -- the shared core proves no method wrap).
        let pre = "fn m() {\n    svc.process(bar(alpha_argument));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    svc.process(bar(alpha_argument));\n+    svc.process(bar(alpha_argument.clone()));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("method-frame bare wrap must fire");
        assert!(out.contains("- bar(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0.clone())\n"), "got: {out}");
        let files = applied(&pre_map, &out);
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    svc.process(bar(alpha_argument.clone()));\n}\n"
        );
    }
}
