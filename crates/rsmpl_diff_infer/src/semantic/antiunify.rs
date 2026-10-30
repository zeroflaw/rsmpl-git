//! Anti-unification core for WI-4 (first slice: `let` statements).
//!
//! Given N ≥ 2 `(minus, plus)` native line pairs of the same `syn`
//! variant, compute the least general generalization — a template pair
//! where every position that is not token-equal across all sites stays
//! a fresh inferred metavariable (`P0` pat, `E0` expr, `T0` type, by
//! node kind) and every token-equal position stays literal. Guards
//! (IMPROVE.md WI-4): same `syn` variant at every generalized node;
//! generalization depth ≤ 1 below the root here (pattern, type,
//! initializer are direct children — deeper differences stay vetoed,
//! never a template that is mostly holes); ≥ 1 literal anchor token
//! must survive per side (`let`/`=`/`;` by construction); repeated
//! positions that differ at one site must differ identically at all
//! sites or stay independent (SPEC 15.7 — enforced by requiring
//! per-site token equality wherever a hole is reused).
//!
//! This module generalizes three shapes: **let type-addition** — minus
//! `let PAT = INIT;` (no ascription) → plus `let PAT: T = INIT;`
//! (uniform concrete `T`, per-site preserved `PAT`/`INIT`; a varying
//! type would need an unbound plus-side `T0` (T442-illegal), so it
//! vetoes; a per-site init/pattern delta is not this shape (verbatim
//! and the call sweeps own those)); **type rewrite** (WI-4b):
//! `antiunify_types` over `syn::Type` pairs with hole correspondence;
//! and **joint let templates** (WI-5 slice 1):
//! `antiunify_let_template` over typed→typed `let` pairs whose
//! pattern and type are preserved per site while the initializer
//! carries one uniform delta (`E0` → `E0.wrapping_add(1)`). Signature
//! evolution (WI-4c) reuses this module next.

use super::shape::init_ok;
use quote::ToTokens;
use syn::spanned::Spanned as _;

/// Uniform concrete type text for a let type-addition class, or `None`
/// when the pairs do not form one.
pub(crate) fn antiunify_let_type_add(minuses: &[&str], pluses: &[&str]) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    // All-identical classes stay for the tighter verbatim sweep (T422
    // precedent): a concrete literal hunk is always shorter than a
    // metavariable template over the same lines.
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut uniform_ty: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let minus = parse_untyped_let(m)?;
        let (plus_pat_inner, plus_ty, plus_init) = parse_typed_let(p)?;
        // Repeated-hole structural equality (SPEC 15.7): `P0` is reused
        // on the plus side, so the inner pattern must be token-equal
        // per site; same for `E0` and the initializer.
        if tok(&minus.pat) != tok(&plus_pat_inner) {
            return None;
        }
        if tok(&minus.init) != tok(&plus_init) {
            return None;
        }
        // Depth-1 engine gate: the bound init must be matchable (no
        // macros/lets/closures/unsupported nodes — the engine `Stmt`
        // path matches inits via structural `match_expr`, never the
        // call router, so there is no Call/MethodCall exception here).
        if !init_ok(&minus.init) {
            return None;
        }
        let ty = tok(&plus_ty);
        if uniform_ty.as_ref().is_some_and(|u| *u != ty) {
            return None;
        }
        uniform_ty = Some(ty);
    }
    let ty = uniform_ty?;
    // The uniform type must parse as a type on its own (it came from a
    // typed `let`, so this only rejects pathological renders).
    if syn::parse_str::<syn::Type>(&ty).is_err() {
        return None;
    }
    Some(ty)
}

/// Joint let template (WI-5 slice 1): N ≥ 2 typed→typed `let` pairs
/// with per-site preserved pattern and type plus one uniform
/// initializer delta. Returns the full statement template pair
/// (`let P0: T0 = E0;` → `let P0: T0 = E0.wrapping_add(1);`), or
/// `None` when the class is not one uniform delta.
///
/// Guards: every pair parses as `Stmt::Local` with a `Pat::Type`
/// ascription on both sides, no `let-else` diverge, no attributes;
/// per-site pattern and type token-equal (preserved — a per-site
/// pat/ty delta belongs to another detector, never guessed here);
/// cross-site hetero pat/ty ride `P0`/`T0`, uniform ones stay concrete
/// (first-site token text); the initializer is either a uniform
/// concrete delta (both sides token-uniform, templates differ) or one
/// `E0` hole with a uniform structural wrapper on the plus side
/// (method-call with varying receiver, bare call with one varying
/// arg, or binary with one varying side — each with ≥ 1 literal
/// anchor, exactly one `E0` occurrence, ordered value correspondence
/// per SPEC 15.7). All-identical line classes decline (verbatim stays
/// the tighter winner, T422).
pub(crate) fn antiunify_let_template(
    minuses: &[&str],
    pluses: &[&str],
) -> Option<(String, String)> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut pats: Vec<String> = Vec::with_capacity(minuses.len());
    let mut tys: Vec<String> = Vec::with_capacity(minuses.len());
    let mut minus_inits: Vec<syn::Expr> = Vec::with_capacity(minuses.len());
    let mut plus_inits: Vec<syn::Expr> = Vec::with_capacity(minuses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (m_pat, m_ty, m_init) = parse_typed_let_full(m)?;
        let (p_pat, p_ty, p_init) = parse_typed_let_full(p)?;
        // Preserved per site (SPEC 15.7 hole reuse): the shared `P0`/`T0`
        // bind per-site-equal text on both sides.
        if tok(&m_pat) != tok(&p_pat) || tok(&m_ty) != tok(&p_ty) {
            return None;
        }
        pats.push(tok(&m_pat));
        tys.push(tok(&m_ty));
        minus_inits.push(m_init);
        plus_inits.push(p_init);
    }
    let pat_tpl = if pats.iter().all(|x| *x == pats[0]) {
        pats[0].clone()
    } else {
        "P0".to_string()
    };
    let ty_tpl = if tys.iter().all(|x| *x == tys[0]) {
        tys[0].clone()
    } else {
        "T0".to_string()
    };
    let minus_vals: Vec<String> = minus_inits.iter().map(tok).collect();
    let plus_vals: Vec<String> = plus_inits.iter().map(tok).collect();
    let minus_uniform = minus_vals.iter().all(|v| *v == minus_vals[0]);
    let plus_uniform = plus_vals.iter().all(|v| *v == plus_vals[0]);
    let (minus_init_tpl, plus_init_tpl) = if minus_uniform && plus_uniform {
        if minus_vals[0] == plus_vals[0] {
            return None;
        }
        (minus_vals[0].clone(), plus_vals[0].clone())
    } else if !minus_uniform && !plus_uniform {
        let plus_tpl = antiunify_plus_with_hole(&minus_inits, &plus_inits)?;
        ("E0".to_string(), plus_tpl)
    } else {
        // One side uniform, the other varying: an unbound or dropped
        // hole (T442 posture) — stays ordinary.
        return None;
    };
    let minus_tpl = format!("let {pat_tpl}: {ty_tpl} = {minus_init_tpl};");
    let plus_tpl = format!("let {pat_tpl}: {ty_tpl} = {plus_init_tpl};");
    if minus_tpl == plus_tpl {
        return None;
    }
    // Defensive: both sides must re-parse as `let` statements (the
    // detector's probe re-checks, but a non-parsing template must never
    // leave this core).
    if !is_let_stmt(&minus_tpl) || !is_let_stmt(&plus_tpl) {
        return None;
    }
    Some((minus_tpl, plus_tpl))
}

/// WI-5c: one statement position across N ≥ 2 blocks. Tries the shared
/// let cores in order — type-addition (uniform concrete type, holed
/// `P0`/`E0`), joint typed template (hetero `P0`/`T0`, uniform
/// `E0`-hole init delta), untyped-init template (same without
/// ascriptions) — then the uniform-concrete fallback (a repeated
/// identical edit: every site rewrites the same literal lines, the
/// anchor of a hetero sequence). Returns full `let` statement
/// templates, or `None` when the position is not one uniform rewrite.
/// The whole-sequence all-identical class never reaches here (verbatim
/// owns it, T422); per-position identical lines are concrete anchors.
pub(crate) fn antiunify_let_position(
    minuses: &[&str],
    pluses: &[&str],
) -> Option<(String, String)> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if let Some(ty) = antiunify_let_type_add(minuses, pluses) {
        return Some(("let P0 = E0;".to_string(), format!("let P0: {ty} = E0;")));
    }
    if let Some(t) = antiunify_let_template(minuses, pluses) {
        return Some(t);
    }
    if let Some(t) = antiunify_untyped_let(minuses, pluses) {
        return Some(t);
    }
    if minuses.iter().all(|m| *m == minuses[0])
        && pluses.iter().all(|p| *p == pluses[0])
        && minuses[0] != pluses[0]
        && is_plain_let_line(minuses[0])
        && is_plain_let_line(pluses[0])
    {
        return Some((minuses[0].trim().to_string(), pluses[0].trim().to_string()));
    }
    None
}

/// WI-5c: anti-unification over N blocks × K statements (block-major
/// flat pairs: block `b` holds indices `b*K .. b*K+K`). Position `j`
/// anti-unifies across blocks via [`antiunify_let_position`]; hetero
/// positions ride holes, identical ones stay concrete. Returns the K
/// minus/plus template lines, or `None` (stays ordinary).
pub(crate) fn antiunify_sequence(
    minuses: &[&str],
    pluses: &[&str],
    k: usize,
) -> Option<(Vec<String>, Vec<String>)> {
    if !(2..=4).contains(&k) || minuses.len() != pluses.len() || !minuses.len().is_multiple_of(k) {
        return None;
    }
    let n = minuses.len() / k;
    if n < 2 {
        return None;
    }
    // All-identical whole classes stay for the tighter verbatim sweep
    // (T422 precedent): a concrete literal hunk over the same lines is
    // always shorter than a metavariable template.
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut minus_tpls = Vec::with_capacity(k);
    let mut plus_tpls = Vec::with_capacity(k);
    for j in 0..k {
        let mut pos_m = Vec::with_capacity(n);
        let mut pos_p = Vec::with_capacity(n);
        for b in 0..n {
            pos_m.push(minuses[b * k + j]);
            pos_p.push(pluses[b * k + j]);
        }
        let (m, p) = antiunify_let_position(&pos_m, &pos_p)?;
        // Per-position hole independence: the shared cores all emit
        // `P0`/`T0`/`E0`; position `j` renames them to `P{j}`/`T{j}`/
        // `E{j}` so each window statement binds independently (shared
        // names would force cross-statement equality, SPEC 15.7).
        let (m, p) = if j == 0 {
            (m, p)
        } else {
            (rename_holes(&m, j), rename_holes(&p, j))
        };
        minus_tpls.push(m);
        plus_tpls.push(p);
    }
    if minus_tpls.iter().zip(plus_tpls.iter()).all(|(m, p)| m == p) {
        return None;
    }
    // Defensive: every template line must re-parse as a plain `let` (the
    // detector's probe re-checks, but a non-parsing template must never
    // leave this core).
    if !minus_tpls.iter().all(|l| is_plain_let_line(l))
        || !plus_tpls.iter().all(|l| is_plain_let_line(l))
    {
        return None;
    }
    Some((minus_tpls, plus_tpls))
}

/// WI-5c: rename the shared cores' `P0`/`T0`/`E0` holes to the
/// position-`j` spelling (`P{j}`/`T{j}`/`E{j}`), whole tokens only.
fn rename_holes(s: &str, j: usize) -> String {
    let mut out = String::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut String| {
        let repl = match cur.as_str() {
            "P0" => format!("P{j}"),
            "T0" => format!("T{j}"),
            "E0" => format!("E{j}"),
            _ => cur.clone(),
        };
        out.push_str(&repl);
        cur.clear();
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            cur.push(c);
        } else {
            flush(&mut cur, &mut out);
            out.push(c);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// One untyped `let` side: inner pattern plus init expr (no ascription).
pub(crate) fn parse_untyped_let_full(line: &str) -> Option<(syn::Pat, syn::Expr)> {
    let untyped = parse_untyped_let(line)?;
    Some((untyped.pat, untyped.init))
}

/// WI-5c untyped-init template: N ≥ 2 untyped→untyped `let` pairs with
/// per-site preserved patterns plus one uniform initializer delta. The
/// joint-typed core without ascriptions: hetero patterns ride `P0`,
/// uniform ones stay concrete; the initializer is either a uniform
/// concrete delta or one `E0` hole with a uniform structural wrapper
/// (method-call with varying receiver, bare call with one varying
/// argument, or binary with one varying side — the shared
/// [`antiunify_plus_with_hole`] discipline). All-identical line
/// classes decline (verbatim owns them, T422).
fn antiunify_untyped_let(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    // All-identical line classes decline (verbatim owns them, T422).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut pats: Vec<String> = Vec::with_capacity(minuses.len());
    let mut minus_inits: Vec<syn::Expr> = Vec::with_capacity(minuses.len());
    let mut plus_inits: Vec<syn::Expr> = Vec::with_capacity(minuses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (m_pat, m_init) = parse_untyped_let_full(m)?;
        let (p_pat, p_init) = parse_untyped_let_full(p)?;
        // Preserved per site (SPEC 15.7 hole reuse): the shared `P0`
        // binds per-site-equal text on both sides.
        if tok(&m_pat) != tok(&p_pat) {
            return None;
        }
        pats.push(tok(&m_pat));
        minus_inits.push(m_init);
        plus_inits.push(p_init);
    }
    let pat_tpl = if pats.iter().all(|x| *x == pats[0]) {
        pats[0].clone()
    } else {
        "P0".to_string()
    };
    let minus_vals: Vec<String> = minus_inits.iter().map(tok).collect();
    let plus_vals: Vec<String> = plus_inits.iter().map(tok).collect();
    let minus_uniform = minus_vals.iter().all(|v| *v == minus_vals[0]);
    let plus_uniform = plus_vals.iter().all(|v| *v == plus_vals[0]);
    let (minus_init_tpl, plus_init_tpl) = if minus_uniform && plus_uniform {
        if minus_vals[0] == plus_vals[0] {
            return None;
        }
        (minus_vals[0].clone(), plus_vals[0].clone())
    } else if !minus_uniform && !plus_uniform {
        let plus_tpl = antiunify_plus_with_hole(&minus_inits, &plus_inits)?;
        ("E0".to_string(), plus_tpl)
    } else {
        // One side uniform, the other varying: an unbound or dropped
        // hole (T442 posture) — stays ordinary.
        return None;
    };
    let minus_tpl = format!("let {pat_tpl} = {minus_init_tpl};");
    let plus_tpl = format!("let {pat_tpl} = {plus_init_tpl};");
    if minus_tpl == plus_tpl {
        return None;
    }
    if !is_let_stmt(&minus_tpl) || !is_let_stmt(&plus_tpl) {
        return None;
    }
    Some((minus_tpl, plus_tpl))
}

/// True when the single-line text parses as a plain `let` (no attrs,
/// init present, no `let-else` diverge) — the WI-5c sequence member and
/// template gate, shared with the WI-5d single-block core.
pub(crate) fn is_plain_let_line(line: &str) -> bool {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return false;
    };
    if !local.attrs.is_empty() {
        return false;
    }
    let Some(init) = &local.init else {
        return false;
    };
    init.diverge.is_none()
}

/// One typed `let` side: inner pattern, ascribed type, init expr.
/// Attributes, missing inits, and `let-else` diverge veto (later
/// slices own those classes). Shared with the WI-5d single-block core.
pub(crate) fn parse_typed_let_full(line: &str) -> Option<(syn::Pat, syn::Type, syn::Expr)> {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return None;
    };
    if !local.attrs.is_empty() {
        return None;
    }
    let syn::Pat::Type(pt) = local.pat else {
        return None;
    };
    let init = local.init?;
    if init.diverge.is_some() {
        return None;
    }
    Some((*pt.pat, *pt.ty, *init.expr))
}

fn is_let_stmt(text: &str) -> bool {
    matches!(syn::parse_str::<syn::Stmt>(text), Ok(syn::Stmt::Local(_)))
}

/// Plus-side template with exactly one `E0` hole whose per-site values
/// equal the corresponding minus inits (ordered correspondence).
/// Slice-1 shapes: method-call with varying receiver, bare call with
/// one varying argument, binary with one varying side. Every other
/// shape vetoes (stays ordinary).
fn antiunify_plus_with_hole(minus: &[syn::Expr], plus: &[syn::Expr]) -> Option<String> {
    let minus_vals: Vec<String> = minus.iter().map(tok).collect();
    // Method call: `E0.wrapping_add(1)` — receiver varies per site
    // (= the minus init), method + turbofish + args uniform anchors.
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
        let method0 = calls[0].method.to_string();
        if calls
            .iter()
            .any(|c| tok(&c.turbofish) != tok(&calls[0].turbofish))
        {
            return None;
        }
        if calls.iter().any(|c| c.args.len() != calls[0].args.len()) {
            return None;
        }
        for (i, arg) in calls[0].args.iter().enumerate() {
            let first = tok(arg);
            if calls.iter().any(|c| tok(&c.args[i]) != first) {
                return None;
            }
        }
        for (rx, m) in calls
            .iter()
            .map(|c| c.receiver.as_ref())
            .zip(minus_vals.iter())
        {
            if tok(rx) != *m {
                return None;
            }
        }
        let turbo = tok(&calls[0].turbofish);
        let turbo = if turbo.is_empty() {
            String::new()
        } else {
            turbo
        };
        let args = calls[0].args.iter().map(tok).collect::<Vec<_>>().join(", ");
        return Some(format!("E0.{method0}{turbo}({args})"));
    }
    // Bare call: `foo(E0)` / `foo(x, E0)` — callee uniform anchor,
    // exactly one varying arg position bound to the minus init.
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
        for i in 0..arities[0] {
            let vals: Vec<String> = plus
                .iter()
                .map(|e| match e {
                    syn::Expr::Call(c) => Some(tok(&c.args[i])),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if vals.iter().all(|v| *v == vals[0]) {
                continue;
            }
            if hole_at.is_some() || vals != minus_vals {
                return None;
            }
            hole_at = Some(i);
        }
        let idx = hole_at?;
        let mut rendered: Vec<String> = Vec::with_capacity(arities[0]);
        for (i, arg) in plus
            .iter()
            .map(|e| match e {
                syn::Expr::Call(c) => &c.args,
                _ => unreachable!(),
            })
            .next()
            .unwrap()
            .iter()
            .enumerate()
        {
            if i == idx {
                rendered.push("E0".to_string());
            } else {
                rendered.push(tok(arg));
            }
        }
        return Some(format!("{}({})", callees[0], rendered.join(", ")));
    }
    // Binary: `E0 + 1` / `1 + E0` — operator + anchor side uniform,
    // exactly one varying side bound to the minus init.
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
        if !left_uniform && !right_uniform {
            return None;
        }
        if !left_uniform && lefts == minus_vals && right_uniform {
            return Some(format!("E0 {} {}", ops[0], rights[0]));
        }
        if !right_uniform && rights == minus_vals && left_uniform {
            return Some(format!("{} {} E0", lefts[0], ops[0]));
        }
        return None;
    }
    None
}

struct UntypedLet {
    pat: syn::Pat,
    init: syn::Expr,
}

/// Parse a minus-side `let PAT = INIT;`: complete single-line `let`
/// statement, no type ascription, with init, no `let-else` diverge
/// (the engine never moves into diverge targets).
fn parse_untyped_let(line: &str) -> Option<UntypedLet> {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return None;
    };
    if matches!(local.pat, syn::Pat::Type(_)) {
        return None;
    }
    let init = local.init?;
    if init.diverge.is_some() {
        return None;
    }
    Some(UntypedLet {
        pat: local.pat,
        init: *init.expr,
    })
}

/// Parse a plus-side `let PAT: T = INIT;` into (inner pattern, type,
/// init). The ascription must be present; init and diverge rules match
/// the minus side.
fn parse_typed_let(line: &str) -> Option<(syn::Pat, syn::Type, syn::Expr)> {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return None;
    };
    let syn::Pat::Type(pt) = local.pat else {
        return None;
    };
    let init = local.init?;
    if init.diverge.is_some() {
        return None;
    }
    Some((*pt.pat, *pt.ty, *init.expr))
}

pub(crate) fn tok<T: ToTokens>(node: &T) -> String {
    node.to_token_stream().to_string()
}

// Engine gate for a bound let initializer: the shared strict gate in
// `super::shape` (WI-4b debt — was a local copy of the verbatim gate;
// agreement is now by construction).

/// One anti-unified site pair: the parsed type plus its original
/// spelling. The text is the line-sliced type source the `ty` was
/// re-parsed from, so every span inside `ty` slices `text` — template
/// rendering inherits the first site's exact spacing (byte-exact
/// round-trips) instead of `ToTokens` normalization.
pub(crate) struct TypedSite {
    pub(crate) ty: syn::Type,
    pub(crate) text: String,
}

/// One inferred hole: the first site's child node (its span slices the
/// first site's text) plus every site's normalized value (correspondence
/// by ordered value vectors).
struct Hole {
    node: syn::Type,
    values: Vec<String>,
}

/// Anti-unification over `syn::Type` pairs (WI-4b core).
///
/// Given N ≥ 2 `(minus, plus)` types, compute the least general
/// template pair: positions token-equal across all sites stay literal,
/// varying `Type` positions become fresh inferred `T{i}` holes
/// (numbered in traversal order per side), and every other varying
/// position vetoes. Guards (IMPROVE.md WI-4):
/// - same `syn::Type` variant at every generalized node — a root
///   constructor delta is the literal anchor and never holes (so the
///   template is never a pure hole);
/// - holes only in `Type` positions at depth ≤ 2 below the root;
///   deeper variation coarsens to the shallowest varying level (the
///   template stays anchored; the engine-count totality is the
///   precision gate, never a guess);
/// - lifetimes, const-generic args, mutability, ABI/unsafety, bound
///   modifiers and all other non-`Type` slots must agree across sites
///   (a varying lifetime would mis-rewrite: the engine wildcards
///   pattern lifetimes while the plus side would emit the first
///   site's spelling);
/// - hole correspondence: same hole count on both sides with identical
///   ordered per-site values (an unbound plus-side hole is
///   T442-illegal; a dropped or reordered hole stays ordinary);
/// - macro/verbatype types are verbatim-only (WI-6c): agreeing
///   macro text stays literal, varying macro positions veto instead
///   of holing (the engine never binds inside macros); rendered
///   templates must differ and re-parse as `syn::Type`.
pub(crate) fn antiunify_types(minus: &[TypedSite], plus: &[TypedSite]) -> Option<(String, String)> {
    if minus.len() != plus.len() || minus.len() < 2 {
        return None;
    }
    let minus_tys: Vec<syn::Type> = minus.iter().map(|s| s.ty.clone()).collect();
    let plus_tys: Vec<syn::Type> = plus.iter().map(|s| s.ty.clone()).collect();
    let mut minus_holes: Vec<Hole> = Vec::new();
    let _ = gen_ty(&minus_tys, 0, &mut minus_holes)?;
    let mut plus_holes: Vec<Hole> = Vec::new();
    let _ = gen_ty(&plus_tys, 0, &mut plus_holes)?;
    if minus_holes.len() != plus_holes.len() {
        return None;
    }
    if minus_holes
        .iter()
        .zip(plus_holes.iter())
        .any(|(m, p)| m.values != p.values)
    {
        return None;
    }
    // Templates inherit the first site's exact spelling (span-sliced):
    // a normalized render would reformat every site (`Box<[u32]>` →
    // `Box < [u32] >`) and break byte-exact round-trips.
    let minus_text = render_template(&minus[0].text, &minus_holes)?;
    let plus_text = render_template(&plus[0].text, &plus_holes)?;
    if minus_text == plus_text {
        return None;
    }
    // Defensive: a render that no longer parses as a type must never emit.
    if syn::parse_str::<syn::Type>(&minus_text).is_err()
        || syn::parse_str::<syn::Type>(&plus_text).is_err()
    {
        return None;
    }
    Some((minus_text, plus_text))
}

/// Render one template from the first site's original type text: each
/// hole's span (single-line snippet coordinates) is replaced end to
/// start with its inferred `T{i}` name. Any irregular span (multi-line,
/// out of bounds, non-boundary, overlapping) vetoes — a guess here
/// would mis-render every site.
fn render_template(first_text: &str, holes: &[Hole]) -> Option<String> {
    let mut repls: Vec<(usize, usize, usize)> = Vec::with_capacity(holes.len());
    for (idx, hole) in holes.iter().enumerate() {
        let span = hole.node.span();
        if span.start().line != 1 || span.end().line != 1 {
            return None;
        }
        repls.push((span.start().column, span.end().column, idx));
    }
    repls.sort();
    for w in repls.windows(2) {
        if w[0].1 > w[1].0 {
            return None;
        }
    }
    for (s, e, _) in &repls {
        if s >= e
            || *e > first_text.len()
            || !first_text.is_char_boundary(*s)
            || !first_text.is_char_boundary(*e)
        {
            return None;
        }
    }
    let mut out = first_text.to_string();
    for (s, e, idx) in repls.iter().rev() {
        out.replace_range(*s..*e, &format!("T{idx}"));
    }
    Some(out)
}

/// Fresh inferred type hole `T{i}` as a `syn::Type` node (parse-built,
/// so every template renders to re-parseable text by construction).
fn hole_ty(i: usize) -> syn::Type {
    syn::parse_str::<syn::Type>(&format!("T{i}")).expect("T{i} parses as a type")
}

/// Render a bare-fn argument name for agreement comparison
/// (`Option<(Ident, Colon)>` has no `ToTokens` impl — the ident text
/// pins it; the engine ignores argument names, so any delta vetoes).
fn tok_opt_name(name: &Option<(proc_macro2::Ident, syn::token::Colon)>) -> String {
    match name {
        None => String::new(),
        Some((id, _)) => id.to_string(),
    }
}

/// Render a `QSelf` for agreement comparison (`QSelf` has no
/// `ToTokens` impl — the angle position plus self type plus path pin
/// it; differing qself vetoes, same as every other non-`Type` slot).
fn tok_qself(q: &Option<syn::QSelf>) -> String {
    match q {
        None => String::new(),
        Some(q) => format!("<{}>@{}", tok(&q.ty), q.position),
    }
}

/// Generalize one `Type`-child position at `depth` below the root:
/// agreeing children stay literal, else structural recursion first and
/// a fresh inferred hole on failure (depth ≤ 2 — varying frames only
/// ever invoke this at depth ≤ 2, so the `depth > 2` arm is defensive).
/// Transactional: a failed structural attempt truncates the holes it
/// recorded before the whole-position hole lands, so stale partial
/// holes can never sit inside a fallback hole.
fn hole_or_gen(tys: &[syn::Type], depth: usize, holes: &mut Vec<Hole>) -> Option<syn::Type> {
    if tys.iter().all(|t| tok(t) == tok(&tys[0])) {
        return Some(tys[0].clone());
    }
    // WI-6c: macro/verbatype types are verbatim-only (engine
    // `match_type` never binds inside them) — a varying nested macro
    // must veto instead of holing whole (a `T{i}` hole would match
    // any type, including macro text the engine keeps literal).
    if tys
        .iter()
        .any(|t| matches!(t, syn::Type::Macro(_) | syn::Type::Verbatim(_)))
    {
        return None;
    }
    let mark = holes.len();
    if let Some(tpl) = gen_ty(tys, depth, holes) {
        return Some(tpl);
    }
    holes.truncate(mark);
    if depth > 2 {
        return None;
    }
    let values = tys.iter().map(tok).collect::<Vec<_>>();
    let idx = holes.len();
    holes.push(Hole {
        node: tys[0].clone(),
        values,
    });
    Some(hole_ty(idx))
}

/// Generalize N type trees at `depth` below the root (0 = roots).
/// Agreeing trees stay literal; varying trees need the same variant
/// with agreeing non-`Type` slots, else `None` (the caller holes the
/// whole position instead — except at the roots, which have no caller
/// and veto, keeping the anchor).
fn gen_ty(tys: &[syn::Type], depth: usize, holes: &mut Vec<Hole>) -> Option<syn::Type> {
    if tys.iter().all(|t| tok(t) == tok(&tys[0])) {
        return Some(tys[0].clone());
    }
    if depth >= 2 {
        return None;
    }
    match &tys[0] {
        syn::Type::Path(_) => {
            let paths: Vec<&syn::Path> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Path(tp) => Some(&tp.path),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            let qself0 = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Path(tp) => tok_qself(&tp.qself),
                    _ => String::new(),
                })
                .collect::<Vec<_>>();
            if qself0.iter().any(|q| *q != qself0[0]) {
                return None;
            }
            let mut out = match &tys[0] {
                syn::Type::Path(tp) => tp.clone(),
                _ => return None,
            };
            out.path = gen_path(&paths, depth, holes)?;
            Some(syn::Type::Path(out))
        }
        syn::Type::Reference(_) => {
            let refs: Vec<&syn::TypeReference> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Reference(r) => Some(r),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if refs
                .iter()
                .any(|r| r.mutability.is_some() != refs[0].mutability.is_some())
            {
                return None;
            }
            if refs
                .iter()
                .any(|r| tok(&r.lifetime) != tok(&refs[0].lifetime))
            {
                return None;
            }
            let elems: Vec<syn::Type> = refs.iter().map(|r| (*r.elem).clone()).collect();
            let mut out = refs[0].clone();
            out.elem = Box::new(hole_or_gen(&elems, depth + 1, holes)?);
            Some(syn::Type::Reference(out))
        }
        syn::Type::Slice(_) => {
            let elems: Vec<syn::Type> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Slice(s) => Some((*s.elem).clone()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            let mut out = match &tys[0] {
                syn::Type::Slice(s) => (*s).clone(),
                _ => return None,
            };
            out.elem = Box::new(hole_or_gen(&elems, depth + 1, holes)?);
            Some(syn::Type::Slice(out))
        }
        syn::Type::Array(_) => {
            let arrs: Vec<&syn::TypeArray> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Array(a) => Some(a),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if arrs.iter().any(|a| tok(&a.len) != tok(&arrs[0].len)) {
                return None;
            }
            let elems: Vec<syn::Type> = arrs.iter().map(|a| (*a.elem).clone()).collect();
            let mut out = arrs[0].clone();
            out.elem = Box::new(hole_or_gen(&elems, depth + 1, holes)?);
            Some(syn::Type::Array(out))
        }
        syn::Type::Tuple(_) => {
            let tups: Vec<&syn::TypeTuple> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Tuple(u) => Some(u),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if tups.iter().any(|u| u.elems.len() != tups[0].elems.len()) {
                return None;
            }
            let mut out = tups[0].clone();
            for (i, elem) in out.elems.iter_mut().enumerate() {
                let nth: Vec<syn::Type> = tups.iter().map(|u| u.elems[i].clone()).collect();
                *elem = hole_or_gen(&nth, depth + 1, holes)?;
            }
            Some(syn::Type::Tuple(out))
        }
        syn::Type::Paren(_) => {
            let inners: Vec<syn::Type> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Paren(p) => Some((*p.elem).clone()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            let mut out = match &tys[0] {
                syn::Type::Paren(p) => (*p).clone(),
                _ => return None,
            };
            out.elem = Box::new(hole_or_gen(&inners, depth + 1, holes)?);
            Some(syn::Type::Paren(out))
        }
        syn::Type::Group(_) => {
            let inners: Vec<syn::Type> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Group(g) => Some((*g.elem).clone()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            let mut out = match &tys[0] {
                syn::Type::Group(g) => (*g).clone(),
                _ => return None,
            };
            out.elem = Box::new(hole_or_gen(&inners, depth + 1, holes)?);
            Some(syn::Type::Group(out))
        }
        syn::Type::Ptr(_) => {
            let ptrs: Vec<&syn::TypePtr> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::Ptr(p) => Some(p),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if ptrs
                .iter()
                .any(|p| tok(&p.const_token) != tok(&ptrs[0].const_token))
            {
                return None;
            }
            if ptrs
                .iter()
                .any(|p| p.mutability.is_some() != ptrs[0].mutability.is_some())
            {
                return None;
            }
            let elems: Vec<syn::Type> = ptrs.iter().map(|p| (*p.elem).clone()).collect();
            let mut out = ptrs[0].clone();
            out.elem = Box::new(hole_or_gen(&elems, depth + 1, holes)?);
            Some(syn::Type::Ptr(out))
        }
        syn::Type::BareFn(_) => {
            let fns: Vec<&syn::TypeBareFn> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::BareFn(f) => Some(f),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if fns
                .iter()
                .any(|f| tok(&f.unsafety) != tok(&fns[0].unsafety))
            {
                return None;
            }
            if fns.iter().any(|f| tok(&f.abi) != tok(&fns[0].abi)) {
                return None;
            }
            if fns.iter().any(|f| f.variadic.is_some()) {
                return None;
            }
            if fns.iter().any(|f| f.inputs.len() != fns[0].inputs.len()) {
                return None;
            }
            let mut out = fns[0].clone();
            for (i, input) in out.inputs.iter_mut().enumerate() {
                let first = &fns[0].inputs[i];
                if fns
                    .iter()
                    .any(|f| tok_opt_name(&f.inputs[i].name) != tok_opt_name(&first.name))
                {
                    return None;
                }
                let nth: Vec<syn::Type> = fns.iter().map(|f| f.inputs[i].ty.clone()).collect();
                input.ty = hole_or_gen(&nth, depth + 1, holes)?;
            }
            match (&fns[0].output, &mut out.output) {
                (syn::ReturnType::Default, syn::ReturnType::Default) => {}
                (syn::ReturnType::Type(_, _), syn::ReturnType::Type(_, out_ty)) => {
                    let nth: Vec<syn::Type> = fns
                        .iter()
                        .map(|f| match &f.output {
                            syn::ReturnType::Type(_, ty) => Some(ty.as_ref().clone()),
                            _ => None,
                        })
                        .collect::<Option<_>>()?;
                    **out_ty = hole_or_gen(&nth, depth + 1, holes)?;
                }
                _ => return None,
            }
            Some(syn::Type::BareFn(out))
        }
        syn::Type::ImplTrait(_) => {
            let bounds: Vec<&syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>> =
                tys.iter()
                    .map(|t| match t {
                        syn::Type::ImplTrait(i) => Some(&i.bounds),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
            let mut out = match &tys[0] {
                syn::Type::ImplTrait(i) => (*i).clone(),
                _ => return None,
            };
            gen_bounds(&bounds, &mut out.bounds, depth + 1, holes)?;
            Some(syn::Type::ImplTrait(out))
        }
        syn::Type::TraitObject(_) => {
            let objs: Vec<&syn::TypeTraitObject> = tys
                .iter()
                .map(|t| match t {
                    syn::Type::TraitObject(o) => Some(o),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if objs
                .iter()
                .any(|o| o.dyn_token.is_some() != objs[0].dyn_token.is_some())
            {
                return None;
            }
            let bounds: Vec<&syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>> =
                objs.iter().map(|o| &o.bounds).collect();
            let mut out = objs[0].clone();
            gen_bounds(&bounds, &mut out.bounds, depth + 1, holes)?;
            Some(syn::Type::TraitObject(out))
        }
        // `_`, `!`, macros, verbatim and friends: agreeing trees took
        // the fast path above; anything varying here has no structural
        // generalization (macros are engine-opaque besides).
        _ => None,
    }
}

/// Generalize N paths at `depth` below the root: leading `::`,
/// segment count and every segment ident agree (the anchor); generic
/// arguments generalize per position.
fn gen_path(paths: &[&syn::Path], depth: usize, holes: &mut Vec<Hole>) -> Option<syn::Path> {
    if paths
        .iter()
        .any(|p| p.leading_colon.is_some() != paths[0].leading_colon.is_some())
    {
        return None;
    }
    if paths
        .iter()
        .any(|p| p.segments.len() != paths[0].segments.len())
    {
        return None;
    }
    let mut out = paths[0].clone();
    for (i, seg) in out.segments.iter_mut().enumerate() {
        if paths
            .iter()
            .any(|p| p.segments[i].ident != paths[0].segments[i].ident)
        {
            return None;
        }
        let nth: Vec<&syn::PathArguments> =
            paths.iter().map(|p| &p.segments[i].arguments).collect();
        seg.arguments = gen_path_args(&nth, depth + 1, holes)?;
    }
    Some(out)
}

/// Generalize N segment-argument lists: all bare, or all `<>` with
/// agreeing `::` and arity and per-position generalization; all
/// `Fn(..)` parenthesized args generalize inputs/output structurally
/// (WI-6c — mirrors the engine `match_path_args` arm); mixed kinds
/// veto. Agreeing parenthesized lists (identical text) took the fast
/// path in `hole_or_gen` before reaching here.
fn gen_path_args(
    args: &[&syn::PathArguments],
    depth: usize,
    holes: &mut Vec<Hole>,
) -> Option<syn::PathArguments> {
    if args.iter().all(|a| matches!(a, syn::PathArguments::None)) {
        return Some(syn::PathArguments::None);
    }
    if args
        .iter()
        .all(|a| matches!(a, syn::PathArguments::Parenthesized(_)))
    {
        let pars: Vec<&syn::ParenthesizedGenericArguments> = args
            .iter()
            .map(|a| match a {
                syn::PathArguments::Parenthesized(p) => Some(p),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if pars.iter().any(|p| p.inputs.len() != pars[0].inputs.len()) {
            return None;
        }
        // Output shapes must agree across sites (all absent or all
        // present) — a mixed Default/Type pair is inexpressible.
        let out_is_default = matches!(pars[0].output, syn::ReturnType::Default);
        if pars
            .iter()
            .any(|p| matches!(p.output, syn::ReturnType::Default) != out_is_default)
        {
            return None;
        }
        let mut out = pars[0].clone();
        for (i, input) in out.inputs.iter_mut().enumerate() {
            let nth: Vec<syn::Type> = pars.iter().map(|p| p.inputs[i].clone()).collect();
            *input = hole_or_gen(&nth, depth, holes)?;
        }
        match (&pars[0].output, &mut out.output) {
            (syn::ReturnType::Default, syn::ReturnType::Default) => {}
            (syn::ReturnType::Type(_, _), syn::ReturnType::Type(_, out_ty)) => {
                let nth: Vec<syn::Type> = pars
                    .iter()
                    .map(|p| match &p.output {
                        syn::ReturnType::Type(_, ty) => Some(ty.as_ref().clone()),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                **out_ty = hole_or_gen(&nth, depth, holes)?;
            }
            _ => return None,
        }
        return Some(syn::PathArguments::Parenthesized(out));
    }
    let mut abs = Vec::with_capacity(args.len());
    for a in args {
        match a {
            syn::PathArguments::AngleBracketed(_) => abs.push(*a),
            _ => return None,
        }
    }
    let first = match abs[0] {
        syn::PathArguments::AngleBracketed(ab) => ab,
        _ => return None,
    };
    if abs.iter().any(|a| match a {
        syn::PathArguments::AngleBracketed(ab) => {
            ab.colon2_token.is_some() != first.colon2_token.is_some()
                || ab.args.len() != first.args.len()
        }
        _ => true,
    }) {
        return None;
    }
    let mut out = first.clone();
    for (i, arg) in out.args.iter_mut().enumerate() {
        let nth: Vec<&syn::GenericArgument> = abs
            .iter()
            .map(|a| match a {
                syn::PathArguments::AngleBracketed(ab) => Some(&ab.args[i]),
                _ => None,
            })
            .collect::<Option<_>>()?;
        *arg = gen_generic_arg(&nth, depth, holes)?;
    }
    Some(syn::PathArguments::AngleBracketed(out))
}

/// Generalize one generic-argument position: `Type` args hole-or-recurse;
/// lifetimes, consts and bindings must agree (the engine wildcards
/// pattern lifetimes — a varying one would mis-rewrite).
fn gen_generic_arg(
    nth: &[&syn::GenericArgument],
    depth: usize,
    holes: &mut Vec<Hole>,
) -> Option<syn::GenericArgument> {
    match nth[0] {
        syn::GenericArgument::Type(_) => {
            let tys: Vec<syn::Type> = nth
                .iter()
                .map(|g| match g {
                    syn::GenericArgument::Type(t) => Some((*t).clone()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            hole_or_gen(&tys, depth, holes).map(syn::GenericArgument::Type)
        }
        _ => {
            let first = tok(nth[0]);
            if nth.iter().any(|g| tok(g) != first) {
                return None;
            }
            Some((*nth[0]).clone())
        }
    }
}

/// Generalize N same-length bound lists in place (trait-object and
/// `impl Trait` arms): trait bounds generalize their paths like type
/// paths; lifetime and verbatim bounds must agree.
fn gen_bounds(
    all: &[&syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>],
    out: &mut syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    depth: usize,
    holes: &mut Vec<Hole>,
) -> Option<()> {
    if all.iter().any(|b| b.len() != all[0].len()) {
        return None;
    }
    for (i, bound) in out.iter_mut().enumerate() {
        let nth: Vec<&syn::TypeParamBound> = all.iter().map(|b| &b[i]).collect();
        match nth[0] {
            syn::TypeParamBound::Trait(_) => {
                let tbs: Vec<&syn::TraitBound> = nth
                    .iter()
                    .map(|b| match b {
                        syn::TypeParamBound::Trait(tb) => Some(tb),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                if tbs
                    .iter()
                    .any(|t| tok(&t.modifier) != tok(&tbs[0].modifier))
                {
                    return None;
                }
                if tbs
                    .iter()
                    .any(|t| tok(&t.lifetimes) != tok(&tbs[0].lifetimes))
                {
                    return None;
                }
                let paths: Vec<&syn::Path> = tbs.iter().map(|t| &t.path).collect();
                let mut out_tb = tbs[0].clone();
                out_tb.path = gen_path(&paths, depth, holes)?;
                *bound = syn::TypeParamBound::Trait(out_tb);
            }
            _ => {
                let first = tok(nth[0]);
                if nth.iter().any(|b| tok(b) != first) {
                    return None;
                }
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_add_hetero_patterns_and_inits_generalizes() {
        let minuses = [
            "    let alpha_value = compute_first(input);",
            "    let beta_value = compute_second(other);",
        ];
        let pluses = [
            "    let alpha_value: u32 = compute_first(input);",
            "    let beta_value: u32 = compute_second(other);",
        ];
        assert_eq!(
            antiunify_let_type_add(&minuses, &pluses).as_deref(),
            Some("u32")
        );
    }

    #[test]
    fn type_add_varying_type_vetoes() {
        // A varying type would need an unbound plus-side T0 (T442).
        let minuses = ["    let a = f(x);", "    let b = f(y);"];
        let pluses = ["    let a: u32 = f(x);", "    let b: u64 = f(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_per_site_init_delta_vetoes() {
        let minuses = ["    let a = f(x);", "    let b = f(y);"];
        let pluses = ["    let a: u32 = f(x);", "    let b: u32 = g(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_per_site_pattern_delta_vetoes() {
        let minuses = ["    let a = f(x);", "    let b = f(y);"];
        let pluses = ["    let a2: u32 = f(x);", "    let b: u32 = f(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_all_identical_declines_for_verbatim() {
        let minuses = ["    let a = f(x);", "    let a = f(x);"];
        let pluses = ["    let a: u32 = f(x);", "    let a: u32 = f(x);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_minus_with_type_vetoes() {
        let minuses = ["    let a: u8 = f(x);", "    let b: u8 = f(y);"];
        let pluses = ["    let a: u32 = f(x);", "    let b: u32 = f(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_plus_without_type_vetoes() {
        let minuses = ["    let a = f(x);", "    let b = f(y);"];
        let pluses = ["    let a = f(x);", "    let b = f(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_macro_init_vetoes() {
        let minuses = ["    let a = m!(x);", "    let b = m!(y);"];
        let pluses = ["    let a: u32 = m!(x);", "    let b: u32 = m!(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_single_vetoes() {
        let minuses = ["    let a = f(x);"];
        let pluses = ["    let a: u32 = f(x);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    fn ty(s: &str) -> syn::Type {
        syn::parse_str::<syn::Type>(s).expect("test type must parse")
    }

    /// Templates inherit the input's exact spacing (span-sliced), so
    /// compact inputs expect compact templates — no normalization.
    fn site(s: &str) -> TypedSite {
        // The text is its own source: spans from `parse_str` slice it.
        TypedSite {
            ty: ty(s),
            text: s.to_string(),
        }
    }

    fn antiunify_strs(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
        let m: Vec<TypedSite> = minuses.iter().map(|s| site(s)).collect();
        let p: Vec<TypedSite> = pluses.iter().map(|s| site(s)).collect();
        antiunify_types(&m, &p)
    }

    #[test]
    fn types_hetero_hole_generalizes() {
        // The canonical WI-4b shape: varying `T` holes one level deep.
        let out = antiunify_strs(
            &["Vec<u32>", "Vec<String>"],
            &["Box<[u32]>", "Box<[String]>"],
        );
        assert_eq!(
            out,
            Some(("Vec<T0>".to_string(), "Box<[T0]>".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn types_multi_hole_generalizes() {
        let out = antiunify_strs(
            &["HashMap<String, u32>", "HashMap<u8, bool>"],
            &["BTreeMap<String, u32>", "BTreeMap<u8, bool>"],
        );
        assert_eq!(
            out,
            Some((
                "HashMap<T0, T1>".to_string(),
                "BTreeMap<T0, T1>".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn types_uniform_concrete_pair_fires() {
        // No holes: a uniform literal rewrite still compresses (the
        // verbatim sweep owns identical *lines*; hetero lines with
        // uniform types are this detector's).
        let out = antiunify_strs(&["Vec<u32>", "Vec<u32>"], &["Box<[u32]>", "Box<[u32]>"]);
        assert_eq!(
            out,
            Some(("Vec<u32>".to_string(), "Box<[u32]>".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn types_deep_variation_coarsens_to_shallow_hole() {
        // Variation at depth 3 coarsens to a depth-2 hole (stays
        // anchored at the root; totality is the precision gate).
        let out = antiunify_strs(
            &["A<B<C<x1>>>", "A<B<C<x2>>>"],
            &["A<B<C<x1>>>", "A<B<C<x2>>>"],
        );
        // Same text both sides: no rewrite at all.
        assert_eq!(out, None, "got: {out:?}");
        let out = antiunify_strs(
            &["A<B<C<x1>>>", "A<B<C<x2>>>"],
            &["D<B<C<x1>>>", "D<B<C<x2>>>"],
        );
        assert_eq!(
            out,
            Some(("A<B<T0>>".to_string(), "D<B<T0>>".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn types_inherit_first_site_spacing() {
        // Span-sliced rendering: the template keeps the first site's
        // exact spacing instead of `ToTokens` normalization, so
        // byte-exact round-trips hold for any uniform style.
        let out = antiunify_strs(
            &["Vec < u32 >", "Vec < String >"],
            &["Box < [u32] >", "Box < [String] >"],
        );
        assert_eq!(
            out,
            Some(("Vec < T0 >".to_string(), "Box < [T0] >".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn types_root_constructor_delta_vetoes() {
        // `Vec<..>` vs `HashMap<..>` across sites: no shared anchor.
        let out = antiunify_strs(&["Vec<u32>", "HashMap<u32>"], &["Box<[u32]>", "Box<[u32]>"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn types_varying_lifetime_vetoes() {
        // The engine wildcards pattern lifetimes while the plus side
        // would emit the first site's spelling — a mis-rewrite.
        let out = antiunify_strs(&["&'a u32", "&'b u32"], &["&'a u32", "&'b u32"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn types_unbound_plus_hole_vetoes() {
        // Minus uniform, plus varying: the plus hole binds nothing (T442).
        let out = antiunify_strs(&["Vec<u32>", "Vec<u32>"], &["Box<[u32]>", "Box<[String]>"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn types_dropped_hole_vetoes() {
        // Minus varying, plus uniform: the minus hole is dropped.
        let out = antiunify_strs(&["Vec<u32>", "Vec<String>"], &["Box<[u8]>", "Box<[u8]>"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn types_reordered_holes_veto() {
        let out = antiunify_strs(
            &["Pair<u32, u64>", "Pair<u8, bool>"],
            &["Pair<u64, u32>", "Pair<bool, u8>"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn types_macro_vetoes() {
        // WI-6c: macro types are verbatim-only, never holed.
        assert!(syn::parse_str::<syn::Type>("m!(x)").is_ok());
        let out = antiunify_strs(&["m!(x)", "m!(y)"], &["Box<m!(x)>", "Box<m!(y)>"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn types_nested_macro_vetoes_instead_of_holing() {
        // WI-6c: a varying nested macro must veto (a `T{i}` hole would
        // match any type, but the engine keeps macros literal).
        let out = antiunify_strs(&["Vec<m!(x)>", "Vec<m!(y)>"], &["Box<m!(x)>", "Box<m!(y)>"]);
        assert_eq!(out, None, "got: {out:?}");
        // Agreeing macro text stays literal while a sibling position
        // generalizes around it.
        let out = antiunify_strs(
            &["Pair<m!(x), u8>", "Pair<m!(x), u16>"],
            &["Other<m!(x), u8>", "Other<m!(x), u16>"],
        );
        assert!(out.is_some(), "agreeing macro + varying sibling must fire");
    }

    #[test]
    fn types_parenthesized_fn_args_generalize() {
        // WI-6c: parenthesized inputs generalize like angle args
        // (mirrors the engine structural arm); the uniform output rides
        // literal while the template delta fires.
        let out = antiunify_strs(
            &["dyn Fn(u8) -> u8", "dyn Fn(u16) -> u8"],
            &["dyn Fn(u8) -> bool", "dyn Fn(u16) -> bool"],
        );
        assert_eq!(
            out,
            Some((
                "dyn Fn(T0) -> u8".to_string(),
                "dyn Fn(T0) -> bool".to_string()
            )),
            "got: {out:?}"
        );
        // Arity mismatch vetoes.
        let out = antiunify_strs(
            &["dyn Fn(u8) -> u8", "dyn Fn(u8, u8) -> u8"],
            &["dyn Fn(u8) -> bool", "dyn Fn(u8) -> bool"],
        );
        assert_eq!(out, None, "got: {out:?}");
        // Output-shape mismatch within one side (Default vs Type) vetoes.
        let out = antiunify_strs(
            &["dyn Fn(u8)", "dyn Fn(u8) -> u8"],
            &["dyn Fn(u8) -> bool", "dyn Fn(u8) -> bool"],
        );
        assert_eq!(out, None, "got: {out:?}");
        // Mixed parenthesized/angle kinds veto.
        let out = antiunify_strs(&["dyn Fn(u8)", "dyn Vec<u8>"], &["Box<u8>", "Box<u8>"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    fn joint(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
        antiunify_let_template(minuses, pluses)
    }

    #[test]
    fn joint_method_receiver_hole_generalizes() {
        // The WI-5 slice-1 canonical shape: hetero pat/ty ride P0/T0,
        // the lit inits ride E0, the method + arg are the anchor.
        let out = joint(
            &["let a: i32 = 0;", "let b: u32 = 1;"],
            &[
                "let a: i32 = 0.wrapping_add(1);",
                "let b: u32 = 1.wrapping_add(1);",
            ],
        );
        assert_eq!(
            out,
            Some((
                "let P0: T0 = E0;".to_string(),
                "let P0: T0 = E0.wrapping_add(1);".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn joint_uniform_pat_ty_stay_concrete() {
        let out = joint(
            &["let a: u32 = f(x);", "let a: u32 = f(y);"],
            &["let a: u32 = g(f(x));", "let a: u32 = g(f(y));"],
        );
        assert_eq!(
            out,
            Some((
                "let a: u32 = E0;".to_string(),
                "let a: u32 = g(E0);".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn joint_binary_hole_generalizes() {
        let out = joint(
            &["let a: i32 = x;", "let b: u32 = y;"],
            &["let a: i32 = x + 1;", "let b: u32 = y + 1;"],
        );
        assert_eq!(
            out,
            Some((
                "let P0: T0 = E0;".to_string(),
                "let P0: T0 = E0 + 1;".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn joint_uniform_concrete_delta_fires() {
        // No holes in the init: a uniform concrete delta over hetero
        // pats still compresses (the anchor is the whole template).
        let out = joint(
            &["let a: u32 = 0;", "let b: u32 = 0;"],
            &["let a: u32 = 1;", "let b: u32 = 1;"],
        );
        assert_eq!(
            out,
            Some((
                "let P0: u32 = 0;".to_string(),
                "let P0: u32 = 1;".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn joint_per_site_pat_delta_vetoes() {
        let out = joint(
            &["let a: i32 = 0;", "let b: u32 = 1;"],
            &[
                "let ax: i32 = 0.wrapping_add(1);",
                "let b: u32 = 1.wrapping_add(1);",
            ],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn joint_per_site_ty_delta_vetoes() {
        let out = joint(
            &["let a: i32 = 0;", "let b: u32 = 1;"],
            &[
                "let a: u64 = 0.wrapping_add(1);",
                "let b: u32 = 1.wrapping_add(1);",
            ],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn joint_divergent_plus_vetoes() {
        let out = joint(
            &["let a: i32 = 0;", "let b: u32 = 1;"],
            &[
                "let a: i32 = 0.wrapping_add(1);",
                "let b: u32 = 1.wrapping_add(2);",
            ],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn joint_receiver_mismatch_vetoes() {
        // Plus receiver is not the minus init at one site: no ordered
        // correspondence (SPEC 15.7) — stays ordinary.
        let out = joint(
            &["let a: i32 = 0;", "let b: u32 = 1;"],
            &[
                "let a: i32 = 0.wrapping_add(1);",
                "let b: u32 = 9.wrapping_add(1);",
            ],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn joint_untyped_minus_vetoes() {
        // Minus-untyped classes belong to the type-add slice.
        let out = joint(
            &["let a = f(x);", "let b = f(y);"],
            &["let a: u32 = f(x);", "let b: u32 = f(y);"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn joint_all_identical_declines_for_verbatim() {
        let out = joint(
            &["let a: u32 = 0;", "let a: u32 = 0;"],
            &[
                "let a: u32 = 0.wrapping_add(1);",
                "let a: u32 = 0.wrapping_add(1);",
            ],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn joint_single_vetoes() {
        let out = joint(&["let a: i32 = 0;"], &["let a: i32 = 0.wrapping_add(1);"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn joint_no_rewrite_vetoes() {
        let out = joint(
            &["let a: i32 = 0;", "let b: u32 = 1;"],
            &["let a: i32 = 0;", "let b: u32 = 1;"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn types_single_and_mismatched_len_veto() {
        let m = [site("Vec<u32>")];
        let p = [site("Box<[u32]>"), site("Box<[u64]>")];
        assert_eq!(antiunify_types(&m, &m), None);
        let m2 = [site("Vec<u32>"), site("Vec<u64>")];
        assert_eq!(antiunify_types(&m2, &p[..1]), None);
    }

    fn seq(minuses: &[&str], pluses: &[&str], k: usize) -> Option<(Vec<String>, Vec<String>)> {
        antiunify_sequence(minuses, pluses, k)
    }

    #[test]
    fn sequence_type_add_2x2_generalizes() {
        // WI-5c canonical shape: 2 blocks × 2 type-add positions with
        // hetero pats/inits ride P{j}/E{j}; the uniform type stays
        // concrete per position.
        let out = seq(
            &[
                "let a = f(x);",
                "let b = g(y);",
                "let c = f(z);",
                "let d = g(w);",
            ],
            &[
                "let a: u32 = f(x);",
                "let b: u32 = g(y);",
                "let c: u32 = f(z);",
                "let d: u32 = g(w);",
            ],
            2,
        );
        assert_eq!(
            out,
            Some((
                vec!["let P0 = E0;".to_string(), "let P1 = E1;".to_string()],
                vec![
                    "let P0: u32 = E0;".to_string(),
                    "let P1: u32 = E1;".to_string()
                ]
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn sequence_joint_and_concrete_positions_mix() {
        // Position 0: joint typed template (hetero pat/ty, uniform
        // method-delta init). Position 1: repeated identical edit
        // (concrete anchor — verbatim cannot span the hetero position,
        // so the sequence owns the window).
        let out = seq(
            &[
                "let a: i32 = x;",
                "let done = finish();",
                "let b: u32 = y;",
                "let done = finish();",
            ],
            &[
                "let a: i32 = x.wrapping_add(1);",
                "let done: bool = finish();",
                "let b: u32 = y.wrapping_add(1);",
                "let done: bool = finish();",
            ],
            2,
        );
        assert_eq!(
            out,
            Some((
                vec![
                    "let P0: T0 = E0;".to_string(),
                    "let done = finish();".to_string()
                ],
                vec![
                    "let P0: T0 = E0.wrapping_add(1);".to_string(),
                    "let done: bool = finish();".to_string()
                ]
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn sequence_untyped_init_delta_generalizes() {
        // Untyped both sides with a uniform method-delta init: E0-hole
        // plus per-site-preserved (here hetero → P0) patterns.
        let out = seq(
            &["let a = x;", "let b = y;", "let c = z;", "let d = w;"],
            &[
                "let a = x.foo();",
                "let b = y.foo();",
                "let c = z.foo();",
                "let d = w.foo();",
            ],
            2,
        );
        assert_eq!(
            out,
            Some((
                vec!["let P0 = E0;".to_string(), "let P1 = E1;".to_string()],
                vec![
                    "let P0 = E0.foo();".to_string(),
                    "let P1 = E1.foo();".to_string()
                ]
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn sequence_k_mismatch_vetoes() {
        // 4 pairs are not 3-wide.
        let minuses = [
            "let a = f(x);",
            "let b = g(y);",
            "let c = f(z);",
            "let d = g(w);",
        ];
        let pluses = [
            "let a: u32 = f(x);",
            "let b: u32 = g(y);",
            "let c: u32 = f(z);",
            "let d: u32 = g(w);",
        ];
        assert_eq!(seq(&minuses, &pluses, 3), None);
        // Single block (N = 1) never forms a repeated sequence.
        assert_eq!(seq(&minuses[..2], &pluses[..2], 2), None);
    }

    #[test]
    fn sequence_all_identical_declines_for_verbatim() {
        // One repeated line (flat-identical class) belongs to verbatim
        // (T422): a literal hunk is shorter than any template.
        let out = seq(
            &[
                "let a = f(x);",
                "let a = f(x);",
                "let a = f(x);",
                "let a = f(x);",
            ],
            &[
                "let a: u32 = f(x);",
                "let a: u32 = f(x);",
                "let a: u32 = f(x);",
                "let a: u32 = f(x);",
            ],
            2,
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn sequence_repeated_identical_blocks_fire_concrete() {
        // Two byte-identical blocks: per-position concrete anchors under
        // one shared header — tighter than two verbatim patterns, and
        // totality still proves the count.
        let out = seq(
            &[
                "let a = f(x);",
                "let b = g(y);",
                "let a = f(x);",
                "let b = g(y);",
            ],
            &[
                "let a: u32 = f(x);",
                "let b: u32 = g(y);",
                "let a: u32 = f(x);",
                "let b: u32 = g(y);",
            ],
            2,
        );
        assert_eq!(
            out,
            Some((
                vec!["let a = f(x);".to_string(), "let b = g(y);".to_string()],
                vec![
                    "let a: u32 = f(x);".to_string(),
                    "let b: u32 = g(y);".to_string()
                ]
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn sequence_divergent_position_vetoes() {
        // Position 1 carries two different init deltas — not one
        // uniform rewrite.
        let out = seq(
            &[
                "let a: i32 = x;",
                "let b: u32 = y;",
                "let c: i64 = z;",
                "let d: u8 = w;",
            ],
            &[
                "let a: i32 = x.wrapping_add(1);",
                "let b: u32 = y.wrapping_add(1);",
                "let c: i64 = z.wrapping_add(2);",
                "let d: u8 = w.wrapping_add(2);",
            ],
            2,
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn sequence_non_let_member_vetoes() {
        // Expression statements are not sequence members.
        let out = seq(
            &["let a = f(x);", "g(y);", "let c = f(z);", "g(w);"],
            &[
                "let a: u32 = f(x);",
                "g2(y);",
                "let c: u32 = f(z);",
                "g2(w);",
            ],
            2,
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn sequence_per_site_pat_delta_vetoes() {
        // A per-site pattern rename inside one position is not a
        // sequence template (rename detectors own it).
        let out = seq(
            &[
                "let a: i32 = 0;",
                "let b: u32 = 1;",
                "let c: i64 = 2;",
                "let d: u8 = 3;",
            ],
            &[
                "let ax: i32 = 0.wrapping_add(1);",
                "let b: u32 = 1.wrapping_add(1);",
                "let c: i64 = 2.wrapping_add(1);",
                "let d: u8 = 3.wrapping_add(1);",
            ],
            2,
        );
        assert_eq!(out, None, "got: {out:?}");
    }
}
