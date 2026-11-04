//! Single-pair arg-template core: N=1 structural wrappers and singles.
//! split off verbatim under the WI-19 1900-line warning gate (D5): pure move,
//! no behavior change. Tests stay inline in the origin file per WI-22.
use super::arg_template::{parse_bare_pair, parse_method_line, tok, wrapper_tpl};
use super::unique_rule;

/// Single-pair structural wrapper (WI-7 slices 8-9): the N ≥ 2
/// `wrapper_tpl` column-uniformity gates are vacuous-but-blocking at
/// N=1 (every one-element column is trivially uniform, so the hole
/// column `continue`s past the wrap and the bare-call/binary/method
/// one-arg-hole arms can never bind). This helper proves the same
/// three vocabularies structurally from one pair, with no uniformity
/// gates:
///
/// * bare call: `plus` is `Expr::Call`, exactly one arg position
///   token-equals `minus` (the hole), every other arg rides verbatim
///   as a concrete anchor (vacuous-uniformity, as in multis);
///   zero-arg calls, zero holes, and two holes (`f(x, x)`, one-hole
///   discipline) veto. The callee token is the anchor beyond the hole
///   (condition (c) of `PrecisionUnique`).
/// * binary: `plus` is `Expr::Binary`, exactly one side token-equals
///   `minus`, the operator plus the other side are concrete anchors
///   (`E + 1` / `1 + E`); both/neither sides equal vetoes.
/// * method call: `plus` is `Expr::MethodCall`, either receiver-is-hole
///   (`E.foo(anchor…)` — structurally complete here, though in practice
///   `wrapper_tpl` already proves it at N=1 via direct token-equality
///   so the fallback only fires where the multi declines) or
///   one-arg-is-hole (`r.foo(E, anchor…)` — the slice-9 shape whose
///   multi column gate is vacuous-but-blocking at N=1). Exactly one
///   hole total: receiver-plus-arg double-match (`x.foo(x)`) and two
///   arg holes (`r.foo(x, x)`) veto (one-hole discipline). The receiver
///   (or the method ident plus uniform args in the receiver-hole case)
///   is the anchor beyond the hole (condition (c)).
///
/// Every other plus shape (reference, unary, cast, field,
/// index, await) declines here: reference keeps its `wrapper_tpl`
/// path (direct token-equality, no uniformity gate).
fn wrapper_tpl_single(minus: &syn::Expr, plus: &syn::Expr, hole: &str) -> Option<String> {
    if let syn::Expr::Call(call) = plus {
        let mut found = false;
        let mut rendered = Vec::with_capacity(call.args.len());
        for arg in call.args.iter() {
            if tok(arg) == tok(minus) {
                if found {
                    return None;
                }
                found = true;
                rendered.push(hole.to_string());
            } else {
                rendered.push(tok(arg));
            }
        }
        if !found {
            return None;
        }
        let callee = tok(&call.func);
        if callee.is_empty() {
            return None;
        }
        return Some(format!("{}({})", callee, rendered.join(", ")));
    }
    if let syn::Expr::Binary(b) = plus {
        let (l, r, op) = (tok(&b.left), tok(&b.right), tok(&b.op));
        let m = tok(minus);
        match (l == m, r == m) {
            (true, false) => return Some(format!("{hole} {op} {r}")),
            (false, true) => return Some(format!("{l} {op} {hole}")),
            _ => return None,
        }
    }
    if let syn::Expr::MethodCall(call) = plus {
        let method = call.method.to_string();
        if method.is_empty() {
            return None;
        }
        let turbo = tok(&call.turbofish);
        let recv = tok(&call.receiver);
        let m = tok(minus);
        let recv_is_hole = recv == m;
        let mut found = false;
        let mut rendered = Vec::with_capacity(call.args.len());
        for arg in call.args.iter() {
            if tok(arg) == m {
                if found {
                    return None;
                }
                found = true;
                rendered.push(hole.to_string());
            } else {
                rendered.push(tok(arg));
            }
        }
        match (recv_is_hole, found) {
            // Receiver is the hole: no arg may also match (two holes).
            (true, false) => {
                return Some(format!("{hole}.{method}{turbo}({})", rendered.join(", ")));
            }
            // One arg is the hole: the receiver token anchors it.
            (false, true) => {
                return Some(format!("{recv}.{method}{turbo}({})", rendered.join(", ")));
            }
            // Zero holes (pure concrete) or two holes (receiver + arg,
            // or two args) never prove from one pair.
            _ => return None,
        }
    }
    None
}

/// Single-pair anti-unification (WI-7 slices 6+8+9): one minus/plus arg
/// vector pair of uniform arity `k ≥ 1` yields a template arg pair with
/// holes on every token-equal position and a wrapper on exactly one
/// differing position, or `None` (stays ordinary). Within-pair rule: a
/// position token-equal across minus/plus binds hole `E{i}`; the single
/// differing position must be a structural wrap via `wrapper_tpl`
/// (method-receiver and reference wraps, same vocabulary as the multi)
/// or via `wrapper_tpl_single` (bare-call, binary, and method
/// one-arg-hole wraps, whose multi column-uniformity gates are
/// vacuous-but-blocking at N=1 — no new template language, no substring
/// surgery). Zero wraps (callee/perm territory), two wraps, pure concrete deltas
/// (`1` → `2`, literal territory), and permuted holes all veto, as do
/// macro/unsupported args (multi precedent). `pub(crate)` for the WI-7
/// slice-22 sub-line method-wrap extractor (`subline.rs`, D5 split —
/// no duplicated core).
pub(crate) fn antiunify_call_args_single(
    minus_args: &[syn::Expr],
    plus_args: &[syn::Expr],
) -> Option<(Vec<String>, Vec<String>)> {
    if minus_args.len() != plus_args.len() || minus_args.is_empty() {
        return None;
    }
    for arg in minus_args.iter().chain(plus_args.iter()) {
        let probe = super::shape::probe_expr(arg);
        if probe.macros > 0 || probe.unsupported > 0 {
            return None;
        }
    }
    let mut minus_tpl = Vec::with_capacity(minus_args.len());
    let mut plus_tpl = Vec::with_capacity(plus_args.len());
    let mut wraps = 0usize;
    for (i, (m, p)) in minus_args.iter().zip(plus_args.iter()).enumerate() {
        if tok(m) == tok(p) {
            minus_tpl.push(format!("E{i}"));
            plus_tpl.push(format!("E{i}"));
            continue;
        }
        let hole = format!("E{i}");
        let w = wrapper_tpl(std::slice::from_ref(m), std::slice::from_ref(p), &hole)
            .or_else(|| wrapper_tpl_single(m, p, &hole))?;
        // N=1 soundness fence (slice-9): the multi `wrapper_tpl`
        // receiver-hole path treats uniform args as concrete anchors,
        // which is vacuous at N=1 — a concrete arg coincidentally
        // token-equal to the minus (`x.foo(x)`) would slip through as a
        // second hole in disguise. Enforce one-hole discipline
        // structurally: the minus may appear exactly once across
        // receiver + args.
        if let syn::Expr::MethodCall(call) = p {
            let m_tok = tok(m);
            let recv_eq = tok(&call.receiver) == m_tok;
            let arg_eqs = call.args.iter().filter(|a| tok(a) == m_tok).count();
            if (recv_eq as usize) + arg_eqs != 1 {
                return None;
            }
        }
        minus_tpl.push(format!("E{i}"));
        plus_tpl.push(w);
        wraps += 1;
    }
    if wraps != 1 || minus_tpl == plus_tpl {
        return None;
    }
    Some((minus_tpl, plus_tpl))
}

/// WI-7 slice-6: bare-call arg-template wrap single (N=1) under
/// `PrecisionUnique`. Fires iff the pair parses (whole-line or the
/// shared T449 `let`-init entry — the emission stays the bare-call
/// pattern and the engine rewrites the call sub-expression in place)
/// as one same-callee call whose args anti-unify to exactly one wrap
/// plus holes (slice-6 shapes via `wrapper_tpl`, slice-8 bare-call and
/// binary plus slice-9 method one-arg-hole inner wraps via
/// `wrapper_tpl_single`), a pre-image is present (`None` vetoes — uniqueness needs
/// the whole file, D3), and `template_pair_holds` proves exactly one
/// site file-wide with zero opaque hits (conditions (a)+(b); (c) is
/// structural: the callee token plus the wrapper anchor beyond the
/// holes). The T361 byte-compare pre-reservation
/// (`single_reproduces`, shared with the general-expr singles) vetoes
/// any TokenStream spacing drift. Emission is the WI-19b allow-one
/// spelling (tight range, directive-free, default `allow=one`).
/// Detector name `call_arg_template` (same family, provenance
/// unchanged).
pub(crate) fn try_call_arg_template_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> crate::semantic::prov::Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return super::prov::Counted::Miss;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    let Some((o_cal, n_cal, o_args, n_args)) = parse_bare_pair(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    // Callee deltas stay with the slice-1 single.
    if o_cal != n_cal {
        return super::prov::Counted::Miss;
    }
    let Some((m_tpl, p_tpl)) = antiunify_call_args_single(&o_args, &n_args) else {
        return super::prov::Counted::Miss;
    };
    // WI-7b: uppercase same callee rides backticks (literal), else it
    // infers as an `expr` hole and over-matches.
    let lit = super::shape::literal_callee(&o_cal);
    let minus_call = format!("{lit}({})", m_tpl.join(", "));
    let plus_call = format!("{lit}({})", p_tpl.join(", "));
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    // The check runs on the raw callee: backticks are RSmPL-level
    // literal spelling (SPEC 15.4), not Rust syntax, so the backticked
    // emission would never parse as an `Expr` (TOOL-5).
    let raw_minus = format!("{o_cal}({})", m_tpl.join(", "));
    let raw_plus = format!("{o_cal}({})", p_tpl.join(", "));
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
    let rule = unique_rule(used_rules, "call_arg_template");
    // Allow-one: directive-free ranged hunk (slices-1-5 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    super::prov::Counted::Fire(hunk)
}

/// WI-7 slice-7: method-arg-template wrap single (N=1) under
/// `PrecisionUnique`. Fires iff the pair parses (whole-line or the
/// shared T449 `let`-init entry — the emission stays the
/// `Receiver.m(...)` pattern and the engine rewrites the method-call
/// sub-expression in place) as one same-receiver same-method call whose
/// args anti-unify to exactly one wrap plus holes (slice-7 shapes via
/// `wrapper_tpl`, slice-8 bare-call/binary plus slice-9 method one-arg-hole
/// inner wraps via `wrapper_tpl_single`), a pre-image is
/// present (`None` vetoes — uniqueness needs the whole file, D3), and
/// `template_pair_holds` proves exactly one site file-wide with zero
/// opaque hits (conditions (a)+(b); (c) is structural: the method ident
/// plus the wrapper anchor beyond the holes). Receiver deltas stay with
/// the m2f single; method deltas stay with the slice-2 single. The T361
/// byte-compare pre-reservation (`single_reproduces`, shared with the
/// general-expr and bare-call singles) vetoes any TokenStream spacing
/// drift. Emission is the WI-19b allow-one spelling (tight range,
/// directive-free, default `allow=one`). Detector name
/// `method_arg_template` (same family, provenance unchanged).
pub(crate) fn try_method_arg_template_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> crate::semantic::prov::Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return super::prov::Counted::Miss;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    let Some((o_recv, o_method, o_args)) = parse_method_line(minuses[0]) else {
        return super::prov::Counted::Miss;
    };
    let Some((n_recv, n_method, n_args)) = parse_method_line(pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    // Receiver deltas stay with the m2f single; method deltas stay with
    // the slice-2 single.
    if o_recv != n_recv || o_method != n_method {
        return super::prov::Counted::Miss;
    }
    let Some((m_tpl, p_tpl)) = antiunify_call_args_single(&o_args, &n_args) else {
        return super::prov::Counted::Miss;
    };
    // WI-7b: uppercase method rides backticks (literal), else it infers
    // as an `expr` hole and over-matches.
    let lit = super::shape::literal_callee(&o_method);
    let minus_tpl = format!("Receiver.{lit}({})", m_tpl.join(", "));
    let plus_tpl = format!("Receiver.{lit}({})", p_tpl.join(", "));
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    // Raw method for the check: backticks are RSmPL-level literal
    // spelling (SPEC 15.4), not Rust syntax (TOOL-5).
    let raw_minus = format!("Receiver.{o_method}({})", m_tpl.join(", "));
    let raw_plus = format!("Receiver.{o_method}({})", p_tpl.join(", "));
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return super::prov::Counted::Miss;
    }
    match crate::totality_counted::engine_verdict(
        Some(pre),
        crate::totality_counted::template_pair_counts(path, Some(pre), &minus_tpl, &plus_tpl),
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
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_tpl, &plus_tpl,
    ) {
        return super::prov::Counted::Miss;
    }
    let rule = unique_rule(used_rules, "method_arg_template");
    // Allow-one: directive-free ranged hunk (slices-1-6 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    super::prov::Counted::Fire(hunk)
}
