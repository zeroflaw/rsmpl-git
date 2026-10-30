//! Bare-call sweeps: callee-only rewrite + same-callee trailing-arg
//! addition (T447). Split from `super` (T468) to hold the 2000-line
//! hygiene gate.

use super::{call_parts, parse_call_expr, unique_rule};

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
