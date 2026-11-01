//! Hetero call-arg templates (WI-5): wrap/transform API migrations.
//!
//! A bare-call or method-call class where every argument position is
//! independently (a) token-equal concrete, (b) a whole-arg hole `E{i}`,
//! or (c) a uniform wrapper — one `E{i}` occurrence inside a plus
//! template with a literal anchor (`E.clone()`, `&E`, `f(E)`).
//! Plus templates may permute holes; dropped or duplicated holes veto.
//! At least one position carries a real delta (disjoint from the
//! callee-only `call_sweep` / method-rename sweeps, which own
//! token-equal-args classes). Macro-hiding arguments veto (the engine
//! never matches TokenStream-normalized macro text); all-identical
//! classes stay with verbatim (T422). Totality is the real-pair
//! engine-count probe (`template_pair_holds`).
//!
//! New module (not `antiunify.rs`: that file sits at the 2000-line gate).

use super::{call_parts, ident_len, strip_one_terminator, unique_rule};
use quote::ToTokens;

/// Token-normalized text for agreement comparison.
pub(crate) fn tok<T: ToTokens>(node: &T) -> String {
    node.to_token_stream().to_string()
}

/// Whole-token occurrences of `hole` (e.g. `E0`) in a template string.
/// A substring inside a longer ident (`E10`, `SOME0`) never counts.
fn count_hole(template: &str, hole: &str) -> usize {
    let mut n = 0;
    let mut cur = String::new();
    let mut flush = |cur: &mut String| {
        if cur.as_str() == hole {
            n += 1;
        }
        cur.clear();
    };
    for c in template.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            cur.push(c);
        } else {
            flush(&mut cur);
        }
    }
    flush(&mut cur);
    n
}

/// Anti-unify one call's argument lists across N ≥ 2 sites.
///
/// Inputs are per-site parsed arg vectors (minus/plus, same length N,
/// uniform arity k ≥ 1). Returns the minus/plus template arg lists, or
/// `None` when the class is not one uniform template (stays ordinary).
pub(crate) fn antiunify_call_args(
    minus_sets: &[Vec<syn::Expr>],
    plus_sets: &[Vec<syn::Expr>],
) -> Option<(Vec<String>, Vec<String>)> {
    let n = minus_sets.len();
    if n != plus_sets.len() || n < 2 {
        return None;
    }
    let k = minus_sets[0].len();
    if k == 0 {
        return None;
    }
    if minus_sets.iter().any(|a| a.len() != k) || plus_sets.iter().any(|a| a.len() != k) {
        return None;
    }
    // Macro/unsupported veto (T448 precedent): the engine never binds
    // inside macro bodies, so a template inlining macro text cannot match.
    for set in minus_sets.iter().chain(plus_sets.iter()) {
        for arg in set {
            let probe = super::shape::probe_expr(arg);
            if probe.macros > 0 || probe.unsupported > 0 {
                return None;
            }
        }
    }
    let minus_vals: Vec<Vec<String>> = minus_sets
        .iter()
        .map(|s| s.iter().map(tok).collect())
        .collect();
    let plus_vals: Vec<Vec<String>> = plus_sets
        .iter()
        .map(|s| s.iter().map(tok).collect())
        .collect();
    // Minus templates: uniform positions stay concrete (first-site
    // spelling is token-normalized, same as the literal sweeps);
    // varying positions ride `E{i}`.
    let mut minus_tpl = Vec::with_capacity(k);
    let mut holes: Vec<bool> = Vec::with_capacity(k);
    for i in 0..k {
        let uniform = minus_vals.iter().all(|v| v[i] == minus_vals[0][i]);
        if uniform {
            minus_tpl.push(minus_vals[0][i].clone());
            holes.push(false);
        } else {
            minus_tpl.push(format!("E{i}"));
            holes.push(true);
        }
    }
    // Plus templates, position by position.
    let mut plus_tpl = Vec::with_capacity(k);
    let mut used = vec![false; k];
    for j in 0..k {
        let minus_uniform_j = minus_vals.iter().all(|v| v[j] == minus_vals[0][j]);
        let plus_uniform_j = plus_vals.iter().all(|v| v[j] == plus_vals[0][j]);
        if minus_uniform_j && plus_uniform_j {
            // Concrete anchor or concrete delta — no holes involved.
            plus_tpl.push(plus_vals[0][j].clone());
            continue;
        }
        // Hole or wrapper: bind exactly one unused minus hole.
        let plus_col: Vec<syn::Expr> = plus_sets.iter().map(|s| s[j].clone()).collect();
        let mut candidates: Vec<(usize, String)> = Vec::new();
        for (i, _) in holes.iter().enumerate().filter(|(_, h)| **h) {
            if used[i] {
                continue;
            }
            let minus_col: Vec<syn::Expr> = minus_sets.iter().map(|s| s[i].clone()).collect();
            // Whole-arg hole first (tighter): ordered value equality.
            let mv: Vec<String> = minus_col.iter().map(tok).collect();
            let pv: Vec<String> = plus_col.iter().map(tok).collect();
            if mv == pv {
                candidates.push((i, format!("E{i}")));
                continue;
            }
            if let Some(w) = wrapper_tpl(&minus_col, &plus_col, &format!("E{i}")) {
                candidates.push((i, w));
            }
        }
        if candidates.len() != 1 {
            // Zero = unbound/dropped hole (T442); more = ambiguous
            // duplicate values — never guess.
            return None;
        }
        let (i, t) = candidates.pop().unwrap();
        used[i] = true;
        plus_tpl.push(t);
    }
    // Every defined hole appears exactly once in the plus side (drop or
    // duplicate stays ordinary — a duplicated hole would need
    // repeat-equality the emission cannot state).
    for (i, h) in holes.iter().enumerate() {
        if !h {
            continue;
        }
        let total: usize = plus_tpl
            .iter()
            .map(|t| count_hole(t, &format!("E{i}")))
            .sum();
        if total != 1 {
            return None;
        }
    }
    // At least one real delta (disjoint from the callee-only sweeps).
    if minus_tpl == plus_tpl {
        return None;
    }
    Some((minus_tpl, plus_tpl))
}

/// Plus-side wrapper template with exactly one `hole` occurrence:
/// method-call (`E.clone()`), bare-call (`f(E)`), binary (`E + 1`),
/// or shared reference (`&E`). Every other shape vetoes.
fn wrapper_tpl(minus: &[syn::Expr], plus: &[syn::Expr], hole: &str) -> Option<String> {
    if minus.len() != plus.len() || minus.is_empty() {
        return None;
    }
    let minus_vals: Vec<String> = minus.iter().map(tok).collect();
    // Method call: receiver-is-hole (`E.clone()`) or one-arg-is-hole.
    if plus.iter().all(|e| matches!(e, syn::Expr::MethodCall(_))) {
        let calls: Vec<&syn::ExprMethodCall> = plus
            .iter()
            .map(|e| match e {
                syn::Expr::MethodCall(c) => Some(c),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if calls.iter().any(|c| c.method != calls[0].method) {
            return None;
        }
        if calls
            .iter()
            .any(|c| tok(&c.turbofish) != tok(&calls[0].turbofish))
        {
            return None;
        }
        if calls.iter().any(|c| c.args.len() != calls[0].args.len()) {
            return None;
        }
        let recv_vals: Vec<String> = calls.iter().map(|c| tok(&c.receiver)).collect();
        let recv_uniform = recv_vals.iter().all(|v| *v == recv_vals[0]);
        let mut hole_args: Vec<usize> = Vec::new();
        for a in 0..calls[0].args.len() {
            let vals: Vec<String> = calls.iter().map(|c| tok(&c.args[a])).collect();
            if vals.iter().all(|v| *v == vals[0]) {
                continue;
            }
            if vals == minus_vals {
                hole_args.push(a);
            } else {
                return None;
            }
        }
        let method = calls[0].method.to_string();
        let turbo = tok(&calls[0].turbofish);
        let args0: Vec<String> = calls[0].args.iter().map(tok).collect();
        if recv_vals == minus_vals {
            // Receiver is the hole: every arg must be a uniform anchor.
            if !hole_args.is_empty() {
                return None;
            }
            return Some(format!("{hole}.{method}{turbo}({})", args0.join(", ")));
        }
        if recv_uniform && hole_args.len() == 1 {
            let mut rendered = args0;
            rendered[hole_args[0]] = hole.to_string();
            return Some(format!(
                "{}.{method}{turbo}({})",
                recv_vals[0],
                rendered.join(", ")
            ));
        }
        return None;
    }
    // Bare call: `f(E)` — callee uniform anchor, exactly one varying
    // arg bound to the minus values.
    if plus.iter().all(|e| matches!(e, syn::Expr::Call(_))) {
        let callees: Vec<String> = plus
            .iter()
            .map(|e| match e {
                syn::Expr::Call(c) => Some(tok(&c.func)),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if callees.iter().any(|c| *c != callees[0]) {
            return None;
        }
        let arities: Vec<usize> = plus
            .iter()
            .map(|e| match e {
                syn::Expr::Call(c) => Some(c.args.len()),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if arities.iter().any(|a| *a != arities[0]) {
            return None;
        }
        let mut hole_at: Option<usize> = None;
        for a in 0..arities[0] {
            let vals: Vec<String> = plus
                .iter()
                .map(|e| match e {
                    syn::Expr::Call(c) => Some(tok(&c.args[a])),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if vals.iter().all(|v| *v == vals[0]) {
                continue;
            }
            if vals == minus_vals && hole_at.is_none() {
                hole_at = Some(a);
            } else {
                return None;
            }
        }
        let idx = hole_at?;
        let first_args: Vec<String> = match &plus[0] {
            syn::Expr::Call(c) => c.args.iter().map(tok).collect(),
            _ => unreachable!(),
        };
        let mut rendered = first_args;
        rendered[idx] = hole.to_string();
        return Some(format!("{}({})", callees[0], rendered.join(", ")));
    }
    // Binary: `E + 1` / `1 + E` — operator + anchor side uniform,
    // exactly one varying side bound to the minus values.
    if plus.iter().all(|e| matches!(e, syn::Expr::Binary(_))) {
        let ops: Vec<String> = plus
            .iter()
            .map(|e| match e {
                syn::Expr::Binary(b) => Some(tok(&b.op)),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if ops.iter().any(|o| *o != ops[0]) {
            return None;
        }
        let lefts: Vec<String> = plus
            .iter()
            .map(|e| match e {
                syn::Expr::Binary(b) => Some(tok(&b.left)),
                _ => None,
            })
            .collect::<Option<_>>()?;
        let rights: Vec<String> = plus
            .iter()
            .map(|e| match e {
                syn::Expr::Binary(b) => Some(tok(&b.right)),
                _ => None,
            })
            .collect::<Option<_>>()?;
        let left_uniform = lefts.iter().all(|l| *l == lefts[0]);
        let right_uniform = rights.iter().all(|r| *r == rights[0]);
        if !left_uniform && lefts == minus_vals && right_uniform {
            return Some(format!("{hole} {} {}", ops[0], rights[0]));
        }
        if !right_uniform && rights == minus_vals && left_uniform {
            return Some(format!("{} {} {hole}", lefts[0], ops[0]));
        }
        return None;
    }
    // Shared reference: `&E` — inner values equal the minus values.
    if plus.iter().all(|e| matches!(e, syn::Expr::Reference(_))) {
        let inners: Vec<String> = plus
            .iter()
            .map(|e| match e {
                syn::Expr::Reference(r) => Some(tok(&r.expr)),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if inners != minus_vals {
            return None;
        }
        let muts: Vec<bool> = plus
            .iter()
            .map(|e| match e {
                syn::Expr::Reference(r) => Some(r.mutability.is_some()),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if muts.iter().any(|m| *m != muts[0]) {
            return None;
        }
        if muts[0] {
            return Some(format!("&mut {hole}"));
        }
        return Some(format!("&{hole}"));
    }
    None
}

/// Parse one `-`/`+` line pair as bare calls; return
/// (old callee, new callee, minus args per site, plus args per site).
/// `pub(crate)` for the WI-7 slice-10 perm single (`perm_single.rs`,
/// D5 split — same whole-line/T449 entry, no duplicated parser).
pub(crate) fn parse_bare_pair(
    old_line: &str,
    new_line: &str,
) -> Option<(String, String, Vec<syn::Expr>, Vec<syn::Expr>)> {
    let old_expr = super::parse_call_expr(old_line)?;
    let new_expr = super::parse_call_expr(new_line)?;
    let (o_cal, o_args) = call_parts(&old_expr)?;
    let (n_cal, n_args) = call_parts(&new_expr)?;
    if o_args.len() != n_args.len() {
        return None;
    }
    Some((o_cal, n_cal, o_args, n_args))
}

/// N-site (N ≥ 2) hetero call-arg template sweep: uniform arity, callee
/// token-equal or one shared delta, every argument position concrete,
/// whole-hole, or uniform-wrapper (see module docs). Emits one
/// `# allow=many` hunk (tight original range). Detector name
/// `call_arg_template`.
pub(crate) fn try_call_arg_template_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    // All-identical classes stay for the tighter verbatim sweep (T422).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut old_cal: Option<String> = None;
    let mut new_cal: Option<String> = None;
    let mut minus_sets: Vec<Vec<syn::Expr>> = Vec::with_capacity(minuses.len());
    let mut plus_sets: Vec<Vec<syn::Expr>> = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n, o_args, n_args) = parse_bare_pair(m, p)?;
        if old_cal.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_cal.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        old_cal = Some(o);
        new_cal = Some(n);
        minus_sets.push(o_args);
        plus_sets.push(n_args);
    }
    let (old_cal, new_cal) = (old_cal?, new_cal?);
    let (m_tpl, p_tpl) = antiunify_call_args(&minus_sets, &plus_sets)?;
    // Disjoint from `call_sweep` (callee-only delta, identical args):
    // a template with no arg delta is that detector's class.
    if m_tpl == p_tpl {
        return None;
    }
    let minus_call = format!(
        "{}({})",
        super::shape::literal_callee(&old_cal),
        m_tpl.join(", ")
    );
    let plus_call = format!(
        "{}({})",
        super::shape::literal_callee(&new_cal),
        p_tpl.join(", ")
    );
    if !crate::totality::template_pair_holds(path, file_old, &minus_call, &plus_call, minuses.len())
    {
        return None;
    }
    let rule = unique_rule(used_rules, "call_arg_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    Some(hunk)
}

/// One method-call line: receiver token text, method ident, arg exprs.
/// Whole-line `recv.m(args)` with one trailing `;`/`,` tolerated, plus
/// the `let`-init unwrap (T449 parity with the bare-call path).
fn parse_method_line(line: &str) -> Option<(String, String, Vec<syn::Expr>)> {
    let text = strip_one_terminator(line.trim())?;
    if text.is_empty() || text.starts_with('&') {
        return None;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if !matches!(expr, syn::Expr::Let(_)) {
            if let Some(parts) = method_parts(&expr) {
                return Some(parts);
            }
        }
    }
    match syn::parse_str::<syn::Stmt>(line.trim()) {
        Ok(syn::Stmt::Local(local)) => {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            method_parts(&init.expr)
        }
        _ => None,
    }
}

/// Method-call parts of an expression: (receiver token text, method
/// ident, args). Turbofish and non-ident methods decline.
fn method_parts(expr: &syn::Expr) -> Option<(String, String, Vec<syn::Expr>)> {
    if let syn::Expr::MethodCall(call) = expr {
        if call.turbofish.is_some() {
            return None;
        }
        let method = call.method.to_string();
        if ident_len(&method)? != method.len() {
            return None;
        }
        return Some((
            tok(&call.receiver),
            method,
            call.args.iter().cloned().collect(),
        ));
    }
    None
}

/// N-site (N ≥ 2) hetero method-arg template sweep: per-site
/// token-equal receivers (bound as `Receiver`), method token-equal or
/// one shared delta, argument positions classified exactly as the
/// bare-call sibling. Emits one `# allow=many` hunk (tight original
/// range). Detector name `method_arg_template`.
pub(crate) fn try_method_arg_template_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut method_old: Option<String> = None;
    let mut method_new: Option<String> = None;
    let mut minus_sets: Vec<Vec<syn::Expr>> = Vec::with_capacity(minuses.len());
    let mut plus_sets: Vec<Vec<syn::Expr>> = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o_recv, o_method, o_args) = parse_method_line(m)?;
        let (n_recv, n_method, n_args) = parse_method_line(p)?;
        // Receivers preserved per site (a receiver delta is the
        // method-to-function domain, never guessed here).
        if o_recv != n_recv {
            return None;
        }
        if method_old.as_ref().is_some_and(|c| *c != o_method) {
            return None;
        }
        if method_new.as_ref().is_some_and(|c| *c != n_method) {
            return None;
        }
        if o_args.len() != n_args.len() {
            return None;
        }
        method_old = Some(o_method);
        method_new = Some(n_method);
        minus_sets.push(o_args);
        plus_sets.push(n_args);
    }
    let (method_old, method_new) = (method_old?, method_new?);
    let (m_tpl, p_tpl) = antiunify_call_args(&minus_sets, &plus_sets)?;
    // Disjoint from `method_to_method` (method-only delta, identical
    // args): a template with no arg delta is that detector's class.
    if m_tpl == p_tpl {
        return None;
    }
    let old_lit = super::shape::literal_callee(&method_old);
    let new_lit = super::shape::literal_callee(&method_new);
    let minus_tpl = format!("Receiver.{old_lit}({})", m_tpl.join(", "));
    let plus_tpl = format!("Receiver.{new_lit}({})", p_tpl.join(", "));
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "method_arg_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

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
/// macro/unsupported args (multi precedent).
fn antiunify_call_args_single(
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
    let (m_tpl, p_tpl) = antiunify_call_args_single(&o_args, &n_args)?;
    // WI-7b: uppercase same callee rides backticks (literal), else it
    // infers as an `expr` hole and over-matches.
    let lit = super::shape::literal_callee(&o_cal);
    let minus_call = format!("{lit}({})", m_tpl.join(", "));
    let plus_call = format!("{lit}({})", p_tpl.join(", "));
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    if syn::parse_str::<syn::Expr>(&minus_call).is_err()
        || syn::parse_str::<syn::Expr>(&plus_call).is_err()
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
    // Allow-one: directive-free ranged hunk (slices-1-5 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    Some(hunk)
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
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let pre = file_old?;
    let (o_recv, o_method, o_args) = parse_method_line(minuses[0])?;
    let (n_recv, n_method, n_args) = parse_method_line(pluses[0])?;
    // Receiver deltas stay with the m2f single; method deltas stay with
    // the slice-2 single.
    if o_recv != n_recv || o_method != n_method {
        return None;
    }
    let (m_tpl, p_tpl) = antiunify_call_args_single(&o_args, &n_args)?;
    // WI-7b: uppercase method rides backticks (literal), else it infers
    // as an `expr` hole and over-matches.
    let lit = super::shape::literal_callee(&o_method);
    let minus_tpl = format!("Receiver.{lit}({})", m_tpl.join(", "));
    let plus_tpl = format!("Receiver.{lit}({})", p_tpl.join(", "));
    // Defensive: both sides must re-parse as calls (the totality probe
    // re-checks, but a non-parsing template must never leave this core).
    if syn::parse_str::<syn::Expr>(&minus_tpl).is_err()
        || syn::parse_str::<syn::Expr>(&plus_tpl).is_err()
    {
        return None;
    }
    if !crate::totality::template_pair_holds(path, Some(pre), &minus_tpl, &plus_tpl, 1) {
        return None;
    }
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_tpl, &plus_tpl,
    ) {
        return None;
    }
    let rule = unique_rule(used_rules, "method_arg_template");
    // Allow-one: directive-free ranged hunk (slices-1-6 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

#[cfg(test)]
mod tests {
    use super::antiunify_call_args;
    use std::collections::HashMap;

    fn exprs(texts: &[&str]) -> Vec<syn::Expr> {
        texts
            .iter()
            .map(|t| syn::parse_str::<syn::Expr>(t).expect("test expr must parse"))
            .collect()
    }

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
    fn wrap_clone_template() {
        let minus = [exprs(&["x"]), exprs(&["y"])];
        let minus_refs: Vec<Vec<syn::Expr>> = vec![minus[0].clone(), minus[1].clone()];
        let plus = [exprs(&["x.clone()"]), exprs(&["y.clone()"])];
        let plus_refs: Vec<Vec<syn::Expr>> = vec![plus[0].clone(), plus[1].clone()];
        let (m, p) = antiunify_call_args(&minus_refs, &plus_refs).expect("must fire");
        assert_eq!(m, vec!["E0"]);
        assert_eq!(p, vec!["E0.clone()"]);
    }

    #[test]
    fn borrow_template() {
        let minus_refs = vec![exprs(&["x"]), exprs(&["y"])];
        let plus_refs = vec![exprs(&["&x"]), exprs(&["&y"])];
        let (m, p) = antiunify_call_args(&minus_refs, &plus_refs).expect("must fire");
        assert_eq!(m, vec!["E0"]);
        assert_eq!(p, vec!["&E0"]);
    }

    #[test]
    fn concrete_delta_position() {
        let minus_refs = vec![exprs(&["a", "1"]), exprs(&["b", "1"])];
        let plus_refs = vec![exprs(&["a", "2"]), exprs(&["b", "2"])];
        let (m, p) = antiunify_call_args(&minus_refs, &plus_refs).expect("must fire");
        assert_eq!(m, vec!["E0", "1"]);
        assert_eq!(p, vec!["E0", "2"]);
    }

    #[test]
    fn permuted_holes() {
        let minus_refs = vec![exprs(&["a", "b"]), exprs(&["c", "d"])];
        let plus_refs = vec![exprs(&["b", "a"]), exprs(&["d", "c"])];
        let (m, p) = antiunify_call_args(&minus_refs, &plus_refs).expect("must fire");
        assert_eq!(m, vec!["E0", "E1"]);
        assert_eq!(p, vec!["E1", "E0"]);
    }

    #[test]
    fn dropped_hole_vetoes() {
        // Minus varies at 0, plus uniform — the hole would drop.
        let minus_refs = vec![exprs(&["a", "1"]), exprs(&["b", "1"])];
        let plus_refs = vec![exprs(&["a", "1"]), exprs(&["a", "1"])];
        assert!(antiunify_call_args(&minus_refs, &plus_refs).is_none());
    }

    #[test]
    fn duplicated_hole_vetoes() {
        // Both plus positions wrap the same minus hole.
        let minus_refs = vec![exprs(&["a"]), exprs(&["b"])];
        let plus_refs = vec![exprs(&["a.clone()", "a"]), exprs(&["b.clone()", "b"])];
        assert!(antiunify_call_args(&minus_refs, &plus_refs).is_none());
    }

    #[test]
    fn macro_arg_vetoes() {
        let minus_refs = vec![exprs(&["x"]), exprs(&["y"])];
        let plus_refs = vec![exprs(&["x"]), exprs(&["y"])];
        // Identical sides carry no delta anyway; a macro arg must veto
        // even where a wrapper exists.
        let wrap_minus = vec![exprs(&["x"]), exprs(&["y"])];
        let wrap_plus: Vec<Vec<syn::Expr>> = vec![
            vec![syn::parse_str::<syn::Expr>("wrap!(x)").unwrap()],
            vec![syn::parse_str::<syn::Expr>("wrap!(x)").unwrap()],
        ];
        assert!(antiunify_call_args(&minus_refs, &plus_refs).is_none());
        assert!(antiunify_call_args(&wrap_minus, &wrap_plus).is_none());
    }

    #[test]
    fn count_hole_ignores_longer_idents() {
        assert_eq!(super::count_hole("E0.clone()", "E0"), 1);
        assert_eq!(super::count_hole("E10", "E1"), 0);
        assert_eq!(super::count_hole("E1, E0", "E0"), 1);
    }

    fn single(minus: &str, plus: &str, pre: Option<&str>, changed: &[usize]) -> Option<String> {
        super::try_call_arg_template_single(
            &[minus],
            &[plus],
            "f.rs",
            pre,
            changed,
            &mut Vec::new(),
        )
    }

    #[test]
    fn wi7_arg_template_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-6: unique single `use_it(x)` -> `use_it(x.clone())`
        // raises to an allow-one hunk and real-applies byte-identical.
        let pre = "fn use_it(x: i32) -> i32 { x }\nfn a(x: i32) -> i32 {\n    use_it(x)\n}\n";
        let raw = section(
            "f.rs",
            "@@ -3 +3 @@\n-    use_it(x)\n+    use_it(x.clone())\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique wrap single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(E0.clone())\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn use_it(x: i32) -> i32 { x }\nfn a(x: i32) -> i32 {\n    use_it(x.clone())\n}\n"
        );
    }

    #[test]
    fn wi7_arg_template_single_let_init_fires_and_round_trips() {
        // The shared T449 entry unwraps `let` inits: the emission stays
        // the bare-call pattern and the engine rewrites the call
        // sub-expression in place (pattern/type ride the site wrapper).
        let pre = "fn m() {\n    let a = use_it(x);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let a = use_it(x);\n+    let a = use_it(x.clone());\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique let-init wrap single must fire");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(E0.clone())\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    let a = use_it(x.clone());\n}\n"
        );
    }

    #[test]
    fn wi7_arg_template_single_twin_vetoes() {
        // An untouched same-callee same-arity twin matches the minus
        // pattern (2 != 1) and vetoes.
        let pre = "fn m() {\n    use_it(x);\n    use_it(y);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    use_it(x);\n+    use_it(x.clone());\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi7_arg_template_single_different_arity_twin_keeps_firing() {
        // Totality keys on callee + arity: a different-arity twin never
        // matches the minus pattern and never vetoes.
        let pre = "fn m() {\n    use_it(x);\n    use_it(x, y);\n}\n";
        assert!(
            single("    use_it(x);", "    use_it(x.clone());", Some(pre), &[2]).is_some(),
            "different-arity twin must not veto"
        );
    }

    #[test]
    fn wi7_arg_template_single_no_pre_image_vetoes() {
        // No whole-file evidence -> no guessing (D3).
        assert!(
            single("    use_it(x);", "    use_it(x.clone());", None, &[2]).is_none(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi7_arg_template_single_zero_wrap_vetoes() {
        // Pure concrete delta (`1` -> `2`) is literal territory, never a
        // wrap single (disjoint by construction).
        assert!(
            single(
                "    check(a, 1);",
                "    check(a, 2);",
                Some("fn m() {\n    check(a, 1);\n}\n"),
                &[2]
            )
            .is_none(),
            "concrete delta must veto"
        );
    }

    #[test]
    fn wi7_arg_template_single_two_wrap_vetoes() {
        // Two wraps need a two-hole template — future work, never
        // guessed at N=1.
        assert!(
            single(
                "    f(a, b);",
                "    f(a.clone(), b.clone());",
                Some("fn m() {\n    f(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "two wraps must veto"
        );
    }

    #[test]
    fn wi7_arg_template_single_perm_vetoes() {
        // Permuted holes (`swap(a, b)` -> `swap(b, a)`) are the perm
        // domain, never a wrap single.
        assert!(
            single(
                "    swap(a, b);",
                "    swap(b, a);",
                Some("fn m() {\n    swap(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "perm must veto"
        );
    }

    #[test]
    fn wi7_arg_template_single_callee_delta_declines() {
        // Callee deltas stay with the slice-1 single.
        assert!(
            single(
                "    foo(1);",
                "    bar(1);",
                Some("fn m() {\n    foo(1);\n}\n"),
                &[2]
            )
            .is_none(),
            "callee delta must decline here"
        );
    }

    #[test]
    fn wi7_arg_template_single_subline_declines() {
        // A wrap buried in a larger line (`use_it(a) + other(b)`) never
        // parses as a bare call — sub-line shapes stay ordinary.
        assert!(
            single(
                "    use_it(a) + other(b);",
                "    use_it(a.clone()) + other(b);",
                Some("fn m() {\n    use_it(a) + other(b);\n}\n"),
                &[2]
            )
            .is_none(),
            "sub-line wrap must decline"
        );
    }

    #[test]
    fn wi7_arg_template_single_reference_wrap_fires() {
        // Reference wraps ride the same core (`x` -> `&x`).
        let out = single(
            "    f(x);",
            "    f(&x);",
            Some("fn m() {\n    f(x);\n}\n"),
            &[2],
        )
        .expect("reference wrap single must fire");
        assert!(out.contains("- f(E0)\n"), "got: {out}");
        assert!(out.contains("+ f(&E0)\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        let pre_map = pre_map(&[("f.rs", "fn m() {\n    f(x);\n}\n")]);
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi7_arg_template_single_macro_arg_vetoes() {
        // Macro-hiding args never raise: the engine cannot match
        // TokenStream-normalized macro text.
        assert!(
            single(
                "    f(m!(x));",
                "    f(m!(x).clone());",
                Some("fn m() {\n    f(m!(x));\n}\n"),
                &[2]
            )
            .is_none(),
            "macro arg must veto"
        );
    }

    #[test]
    fn wi8_bare_wrap_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-8: unique single `use_it(x)` -> `use_it(f(x))`
        // raises to an allow-one hunk and real-applies byte-identical
        // (the N>=2 column-uniformity gates are vacuous at N=1, so the
        // structural `wrapper_tpl_single` proof carries this shape).
        let pre = "fn use_it(x: i32) -> i32 { x }\nfn f(x: i32) -> i32 { x }\nfn a(x: i32) -> i32 {\n    use_it(x)\n}\n";
        let raw = section("f.rs", "@@ -4 +4 @@\n-    use_it(x)\n+    use_it(f(x))\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique bare wrap single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(f(E0))\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn use_it(x: i32) -> i32 { x }\nfn f(x: i32) -> i32 { x }\nfn a(x: i32) -> i32 {\n    use_it(f(x))\n}\n"
        );
    }

    #[test]
    fn wi8_bare_wrap_single_let_init_fires_and_round_trips() {
        // The shared T449 entry unwraps `let` inits: the emission stays
        // the bare-call pattern and the engine rewrites the call
        // sub-expression in place.
        let pre = "fn m() {\n    let a = use_it(x);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let a = use_it(x);\n+    let a = use_it(f(x));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique let-init bare wrap single must fire");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(f(E0))\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    let a = use_it(f(x));\n}\n");
    }

    #[test]
    fn wi8_binary_wrap_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-8: unique single `use_it(x)` -> `use_it(x + 1)`
        // raises to an allow-one hunk and real-applies byte-identical.
        let pre = "fn m() {\n    use_it(x);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    use_it(x);\n+    use_it(x + 1);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique binary wrap single must fire");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(E0 + 1)\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    use_it(x + 1);\n}\n");
    }

    #[test]
    fn wi8_binary_wrap_single_flipped_side_fires() {
        // Anchor on the left (`1 + x`): the same structural proof with
        // the hole on the right.
        let out = single(
            "    use_it(x);",
            "    use_it(1 + x);",
            Some("fn m() {\n    use_it(x);\n}\n"),
            &[2],
        )
        .expect("flipped binary wrap single must fire");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(1 + E0)\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        let pre_map = pre_map(&[("f.rs", "fn m() {\n    use_it(x);\n}\n")]);
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi8_binary_wrap_single_let_init_fires() {
        let out = single(
            "    let a = use_it(x);",
            "    let a = use_it(x + 1);",
            Some("fn m() {\n    let a = use_it(x);\n}\n"),
            &[2],
        )
        .expect("let-init binary wrap single must fire");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(E0 + 1)\n"), "got: {out}");
        let pre_map = pre_map(&[("f.rs", "fn m() {\n    let a = use_it(x);\n}\n")]);
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi8_bare_wrap_single_twin_vetoes() {
        // An untouched same-callee same-arity twin matches the minus
        // pattern (2 != 1) and vetoes.
        let pre = "fn m() {\n    use_it(x);\n    use_it(y);\n}\n";
        let raw = section("f.rs", "@@ -2 +2 @@\n-    use_it(x);\n+    use_it(f(x));\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi8_binary_wrap_single_twin_vetoes() {
        assert!(
            single(
                "    use_it(x);",
                "    use_it(x + 1);",
                Some("fn m() {\n    use_it(x);\n    use_it(y);\n}\n"),
                &[2]
            )
            .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi8_bare_wrap_single_no_pre_image_vetoes() {
        // No whole-file evidence -> no guessing (D3, shared gate).
        assert!(
            single("    use_it(x);", "    use_it(f(x));", None, &[2]).is_none(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi8_bare_wrap_single_two_hole_vetoes() {
        // Two holes need a two-hole template — future work, never
        // guessed at N=1 (one-hole discipline).
        assert!(
            single(
                "    f(x);",
                "    f(g(x, x));",
                Some("fn m() {\n    f(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "two holes must veto"
        );
    }

    #[test]
    fn wi8_bare_wrap_single_zero_hole_vetoes() {
        // A wrapper with no hole (`f()` carries no `E0`) is not a wrap
        // of the minus arg — pure concrete territory, never a single.
        assert!(
            single(
                "    use_it(x);",
                "    use_it(f());",
                Some("fn m() {\n    use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "zero-hole wrapper must veto"
        );
    }

    #[test]
    fn wi8_binary_wrap_single_both_sides_equal_vetoes() {
        // `x + x` binds the hole twice — duplicated-hole territory,
        // never guessed (T442 discipline).
        assert!(
            single(
                "    use_it(x);",
                "    use_it(x + x);",
                Some("fn m() {\n    use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "double-hole binary must veto"
        );
    }

    #[test]
    fn wi8_binary_wrap_single_neither_side_equal_vetoes() {
        // Neither side carries the minus — a concrete delta, literal
        // territory (disjoint by construction).
        assert!(
            single(
                "    use_it(x);",
                "    use_it(y + 1);",
                Some("fn m() {\n    use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "concrete binary must veto"
        );
    }

    #[test]
    fn wi8_bare_wrap_single_macro_interior_vetoes() {
        // A wrapper carrying macro text never raises: the engine never
        // matches inside macro bodies (multi precedent).
        assert!(
            single(
                "    use_it(x);",
                "    use_it(f(m!(x)));",
                Some("fn m() {\n    use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "macro interior must veto"
        );
    }

    #[test]
    fn wi8_method_entry_bare_wrap_single_fires_and_round_trips() {
        // The shared single-pair core serves the method outer entry too:
        // `r.use_it(x)` -> `r.use_it(f(x))` raises `Receiver.use_it(E0)`
        // -> `Receiver.use_it(f(E0))` under the same gates.
        let pre = "fn m() {\n    r.use_it(x);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    r.use_it(x);\n+    r.use_it(f(x));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique method-entry bare wrap single must fire");
        assert!(out.contains("- Receiver.use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.use_it(f(E0))\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    r.use_it(f(x));\n}\n");
    }

    #[test]
    fn wi8_method_entry_binary_wrap_single_fires() {
        let out = msingle(
            "    r.use_it(x);",
            "    r.use_it(x + 1);",
            Some("fn m() {\n    r.use_it(x);\n}\n"),
            &[2],
        )
        .expect("method-entry binary wrap single must fire");
        assert!(out.contains("- Receiver.use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.use_it(E0 + 1)\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        let pre_map = pre_map(&[("f.rs", "fn m() {\n    r.use_it(x);\n}\n")]);
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi8_method_entry_bare_wrap_single_twin_vetoes() {
        assert!(
            msingle(
                "    r.use_it(x);",
                "    r.use_it(f(x));",
                Some("fn m() {\n    r.use_it(x);\n    q.use_it(y);\n}\n"),
                &[2]
            )
            .is_none(),
            "twin must veto"
        );
    }

    fn msingle(minus: &str, plus: &str, pre: Option<&str>, changed: &[usize]) -> Option<String> {
        super::try_method_arg_template_single(
            &[minus],
            &[plus],
            "f.rs",
            pre,
            changed,
            &mut Vec::new(),
        )
    }

    #[test]
    fn wi7_method_arg_template_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-7: unique single `r.use_it(x)` -> `r.use_it(x.clone())`
        // raises to an allow-one hunk and real-applies byte-identical.
        let pre = "fn m() {\n    r.use_it(x);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    r.use_it(x);\n+    r.use_it(x.clone());\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique method wrap single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- Receiver.use_it(E0)\n"), "got: {out}");
        assert!(
            out.contains("+ Receiver.use_it(E0.clone())\n"),
            "got: {out}"
        );
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    r.use_it(x.clone());\n}\n");
    }

    #[test]
    fn wi7_method_arg_template_single_let_init_fires_and_round_trips() {
        // The shared T449 entry unwraps `let` inits: the emission stays
        // the `Receiver.m(...)` pattern and the engine rewrites the
        // method call sub-expression in place.
        let pre = "fn m() {\n    let a = r.use_it(x);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let a = r.use_it(x);\n+    let a = r.use_it(x.clone());\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique let-init method wrap single must fire");
        assert!(out.contains("- Receiver.use_it(E0)\n"), "got: {out}");
        assert!(
            out.contains("+ Receiver.use_it(E0.clone())\n"),
            "got: {out}"
        );
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    let a = r.use_it(x.clone());\n}\n"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_twin_vetoes() {
        // An untouched same-method same-arity twin matches the minus
        // pattern (2 != 1) and vetoes.
        let pre = "fn m() {\n    r.use_it(x);\n    q.use_it(y);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    r.use_it(x);\n+    r.use_it(x.clone());\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_different_arity_twin_keeps_firing() {
        // Totality keys on method + arity: a different-arity twin never
        // matches the minus pattern and never vetoes.
        let pre = "fn m() {\n    r.use_it(x);\n    q.use_it(x, y);\n}\n";
        assert!(
            msingle(
                "    r.use_it(x);",
                "    r.use_it(x.clone());",
                Some(pre),
                &[2]
            )
            .is_some(),
            "different-arity twin must not veto"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_no_pre_image_vetoes() {
        // No whole-file evidence -> no guessing (D3).
        assert!(
            msingle("    r.use_it(x);", "    r.use_it(x.clone());", None, &[2]).is_none(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_receiver_delta_declines() {
        // Receiver deltas stay with the m2f single, never a wrap.
        assert!(
            msingle(
                "    r.use_it(x);",
                "    q.use_it(x.clone());",
                Some("fn m() {\n    r.use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "receiver delta must decline"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_method_delta_declines() {
        // Method deltas stay with the slice-2 single.
        assert!(
            msingle(
                "    r.use_it(x);",
                "    r.other(x);",
                Some("fn m() {\n    r.use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "method delta must decline here"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_zero_wrap_vetoes() {
        // Pure concrete delta (`1` -> `2`) is literal territory, never a
        // wrap single (disjoint by construction).
        assert!(
            msingle(
                "    r.check(a, 1);",
                "    r.check(a, 2);",
                Some("fn m() {\n    r.check(a, 1);\n}\n"),
                &[2]
            )
            .is_none(),
            "concrete delta must veto"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_two_wrap_vetoes() {
        // Two wraps need a two-hole template — future work, never
        // guessed at N=1.
        assert!(
            msingle(
                "    r.f(a, b);",
                "    r.f(a.clone(), b.clone());",
                Some("fn m() {\n    r.f(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "two wraps must veto"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_perm_vetoes() {
        // Permuted holes (`swap(a, b)` -> `swap(b, a)`) are the perm
        // domain, never a wrap single.
        assert!(
            msingle(
                "    r.swap(a, b);",
                "    r.swap(b, a);",
                Some("fn m() {\n    r.swap(a, b);\n}\n"),
                &[2]
            )
            .is_none(),
            "perm must veto"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_subline_declines() {
        // A wrap buried in a larger line never parses as a method call —
        // sub-line shapes stay ordinary.
        assert!(
            msingle(
                "    r.use_it(a) + other(b);",
                "    r.use_it(a.clone()) + other(b);",
                Some("fn m() {\n    r.use_it(a) + other(b);\n}\n"),
                &[2]
            )
            .is_none(),
            "sub-line wrap must decline"
        );
    }

    #[test]
    fn wi7_method_arg_template_single_reference_wrap_fires() {
        // Reference wraps ride the same core (`x` -> `&x`).
        let out = msingle(
            "    r.f(x);",
            "    r.f(&x);",
            Some("fn m() {\n    r.f(x);\n}\n"),
            &[2],
        )
        .expect("reference wrap single must fire");
        assert!(out.contains("- Receiver.f(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.f(&E0)\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        let pre_map = pre_map(&[("f.rs", "fn m() {\n    r.f(x);\n}\n")]);
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi7_method_arg_template_single_macro_arg_vetoes() {
        // Macro-hiding args never raise: the engine cannot match
        // TokenStream-normalized macro text.
        assert!(
            msingle(
                "    r.f(m!(x));",
                "    r.f(m!(x).clone());",
                Some("fn m() {\n    r.f(m!(x));\n}\n"),
                &[2]
            )
            .is_none(),
            "macro arg must veto"
        );
    }

    #[test]
    fn wi9_method_inner_wrap_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-9: unique single `use_it(x)` -> `use_it(r.foo(x))`
        // raises to an allow-one hunk and real-applies byte-identical
        // (the N>=2 column-uniformity gate is vacuous at N=1, so the
        // structural `wrapper_tpl_single` method arm carries this shape).
        let pre = "fn use_it(x: i32) -> i32 { x }\nfn m(x: i32) -> i32 {\n    use_it(x)\n}\n";
        let raw = section(
            "f.rs",
            "@@ -3 +3 @@\n-    use_it(x)\n+    use_it(r.foo(x))\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique method inner wrap single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(r.foo(E0))\n"), "got: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn use_it(x: i32) -> i32 { x }\nfn m(x: i32) -> i32 {\n    use_it(r.foo(x))\n}\n"
        );
    }

    #[test]
    fn wi9_method_inner_wrap_single_let_init_fires() {
        let out = single(
            "    let a = use_it(x);",
            "    let a = use_it(r.foo(x));",
            Some("fn m() {\n    let a = use_it(x);\n}\n"),
            &[2],
        )
        .expect("let-init method inner wrap single must fire");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(r.foo(E0))\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        let pre_map = pre_map(&[("f.rs", "fn m() {\n    let a = use_it(x);\n}\n")]);
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi9_method_inner_wrap_single_second_arg_fires() {
        // Hole at arg 1 with a leading concrete anchor.
        let out = single(
            "    use_it(x);",
            "    use_it(r.foo(1, x));",
            Some("fn m() {\n    use_it(x);\n}\n"),
            &[2],
        )
        .expect("second-arg method inner wrap single must fire");
        assert!(out.contains("- use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ use_it(r.foo(1, E0))\n"), "got: {out}");
        let pre_map = pre_map(&[("f.rs", "fn m() {\n    use_it(x);\n}\n")]);
        validated_live(&out, &pre_map);
    }

    #[test]
    fn wi9_method_inner_wrap_single_twin_vetoes() {
        let pre = "fn m() {\n    use_it(x);\n    use_it(y);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    use_it(x);\n+    use_it(r.foo(x));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "twin must veto"
        );
    }

    #[test]
    fn wi9_method_inner_wrap_single_two_hole_vetoes() {
        // Two holes need a two-hole template — never guessed at N=1.
        assert!(
            single(
                "    use_it(x);",
                "    use_it(r.foo(x, x));",
                Some("fn m() {\n    use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "two holes must veto"
        );
    }

    #[test]
    fn wi9_method_inner_wrap_single_recv_plus_arg_vetoes() {
        // `x.foo(x)` binds the hole twice — duplicated-hole territory.
        assert!(
            single(
                "    use_it(x);",
                "    use_it(x.foo(x));",
                Some("fn m() {\n    use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "receiver+arg double match must veto"
        );
    }

    #[test]
    fn wi9_method_inner_wrap_single_zero_hole_vetoes() {
        // No position carries the minus — pure concrete territory.
        assert!(
            single(
                "    use_it(x);",
                "    use_it(r.foo(1));",
                Some("fn m() {\n    use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "zero-hole method wrapper must veto"
        );
    }

    #[test]
    fn wi9_method_inner_wrap_single_macro_interior_vetoes() {
        // A wrapper carrying macro text never raises (multi precedent).
        assert!(
            single(
                "    use_it(x);",
                "    use_it(r.foo(m!(x)));",
                Some("fn m() {\n    use_it(x);\n}\n"),
                &[2]
            )
            .is_none(),
            "macro interior must veto"
        );
    }

    #[test]
    fn wi9_method_entry_inner_wrap_single_fires_and_round_trips() {
        // The shared single-pair core serves the method outer entry too.
        let pre = "fn m() {\n    r.use_it(x);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    r.use_it(x);\n+    r.use_it(q.foo(x));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique method-entry inner wrap single must fire");
        assert!(out.contains("- Receiver.use_it(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.use_it(q.foo(E0))\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    r.use_it(q.foo(x));\n}\n");
    }

    #[test]
    fn wi9_method_entry_inner_wrap_single_twin_vetoes() {
        assert!(
            msingle(
                "    r.use_it(x);",
                "    r.use_it(q.foo(x));",
                Some("fn m() {\n    r.use_it(x);\n    q.use_it(y);\n}\n"),
                &[2]
            )
            .is_none(),
            "twin must veto"
        );
    }
}
