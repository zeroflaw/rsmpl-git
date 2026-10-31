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
fn tok<T: ToTokens>(node: &T) -> String {
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
fn parse_bare_pair(
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

#[cfg(test)]
mod tests {
    use super::antiunify_call_args;

    fn exprs(texts: &[&str]) -> Vec<syn::Expr> {
        texts
            .iter()
            .map(|t| syn::parse_str::<syn::Expr>(t).expect("test expr must parse"))
            .collect()
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
}
