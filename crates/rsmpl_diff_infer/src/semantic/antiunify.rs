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

pub(crate) use super::antiunify_types::{antiunify_types, tok, TypedSite};
use super::pat_template::site_spelling;
use super::shape::init_ok;

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
    let mut pat_asts: Vec<syn::Pat> = Vec::with_capacity(minuses.len());
    let mut tys: Vec<String> = Vec::with_capacity(minuses.len());
    let mut ty_asts: Vec<syn::Type> = Vec::with_capacity(minuses.len());
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
        pat_asts.push(m_pat);
        tys.push(tok(&m_ty));
        ty_asts.push(m_ty);
        minus_inits.push(m_init);
        plus_inits.push(p_init);
    }
    // Uniform concrete spots keep the first site's spelling
    // (byte-identical real-apply, WI-3 D1).
    let pat_tpl = if pats.iter().all(|x| *x == pats[0]) {
        site_spelling(minuses[0], &pat_asts[0], &pats[0])
    } else if let Some(tpl) = super::pat_template::gen_pat_template(&pat_asts) {
        // WI-3: destructuring keeps its anchor (`Some(P0)`).
        tpl
    } else {
        "P0".to_string()
    };
    let ty_tpl = if tys.iter().all(|x| *x == tys[0]) {
        site_spelling(minuses[0], &ty_asts[0], &tys[0])
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
        (
            site_spelling(minuses[0], &minus_inits[0], &minus_vals[0]),
            site_spelling(pluses[0], &plus_inits[0], &plus_vals[0]),
        )
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
/// WI-3: numbered `P1…` holes rename to `P{j}_{k}` (SPEC 15.7).
fn rename_holes(s: &str, j: usize) -> String {
    let mut out = String::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut String| {
        let repl = match cur.as_str() {
            "P0" => format!("P{j}"),
            "T0" => format!("T{j}"),
            "E0" => format!("E{j}"),
            _ => match cur.strip_prefix('P') {
                Some(k) if !k.is_empty() && k.chars().all(|c| c.is_ascii_digit()) => {
                    format!("P{j}_{k}")
                }
                _ => cur.clone(),
            },
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
/// classes decline (verbatim owns them, T422). WI-8 slice 1 publishes
/// this core for the whole-section untyped-let detector (previously
/// reachable only through sequence windows).
pub(crate) fn antiunify_untyped_let(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    // All-identical line classes decline (verbatim owns them, T422).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut pats: Vec<String> = Vec::with_capacity(minuses.len());
    let mut pat_asts: Vec<syn::Pat> = Vec::with_capacity(minuses.len());
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
        // Macro/unsupported inits veto (WI-8): the engine never binds
        // inside macro bodies, so a template inlining macro text can
        // only match token-equal — nested-macro classes belong to the
        // macro-interior detector, never to a whole-line let template.
        if !init_ok(&m_init) || !init_ok(&p_init) {
            return None;
        }
        pats.push(tok(&m_pat));
        pat_asts.push(m_pat);
        minus_inits.push(m_init);
        plus_inits.push(p_init);
    }
    let pat_tpl = if pats.iter().all(|x| *x == pats[0]) {
        site_spelling(minuses[0], &pat_asts[0], &pats[0])
    } else if let Some(tpl) = super::pat_template::gen_pat_template(&pat_asts) {
        tpl // WI-3 parity with the typed core above.
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
        (
            site_spelling(minuses[0], &minus_inits[0], &minus_vals[0]),
            site_spelling(pluses[0], &plus_inits[0], &plus_vals[0]),
        )
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

/// WI-8 slice 1 tuple-position swap: N ≥ 2 untyped→untyped `let`
/// pairs where each minus/plus pattern is an arity-2 tuple with the
/// same two elements in swapped order (`let (P, Q) = E;` →
/// `let (Q, P) = E;`), the initializer token-equal per site (a
/// per-site init delta is an untyped-let template, never guessed
/// here), and the swap is real per site (no `let (x, x)` self-swap).
/// Cross-site hetero elements ride `P0`/`P1` (minus-first is `P0`,
/// minus-second is `P1`, so the plus side replays them swapped);
/// uniform elements stay concrete (first-site spelling).
/// All-identical line classes decline (verbatim owns them, T422).
/// The typed twin (`let (P, Q): T = E;`) stays ordinary.
pub(crate) fn antiunify_pat_swap(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut firsts: Vec<String> = Vec::with_capacity(minuses.len());
    let mut seconds: Vec<String> = Vec::with_capacity(minuses.len());
    let mut init_texts: Vec<String> = Vec::with_capacity(minuses.len());
    // First-site element nodes for byte-identical uniform spellings
    // (the `site_spelling` posture shared with the let cores above).
    let mut first_elems: Option<(syn::Pat, syn::Pat)> = None;
    let mut first_init: Option<syn::Expr> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (m_pat, m_init) = parse_untyped_let_full(m)?;
        let (p_pat, p_init) = parse_untyped_let_full(p)?;
        if tok(&m_init) != tok(&p_init) {
            return None;
        }
        // Macro/unsupported inits veto (WI-8, same posture as the
        // untyped core above): nested-macro classes belong to the
        // macro-interior detector.
        if !init_ok(&m_init) {
            return None;
        }
        let syn::Pat::Tuple(mt) = m_pat else {
            return None;
        };
        let syn::Pat::Tuple(pt) = p_pat else {
            return None;
        };
        if mt.elems.len() != 2 || pt.elems.len() != 2 {
            return None;
        }
        let m0 = tok(&mt.elems[0]);
        let m1 = tok(&mt.elems[1]);
        let p0 = tok(&pt.elems[0]);
        let p1 = tok(&pt.elems[1]);
        if m0 == m1 || m0 != p1 || m1 != p0 {
            return None;
        }
        if first_elems.is_none() {
            first_elems = Some((mt.elems[0].clone(), mt.elems[1].clone()));
            first_init = Some(m_init.clone());
        }
        firsts.push(m0);
        seconds.push(m1);
        init_texts.push(tok(&m_init));
    }
    let first_uniform = firsts.iter().all(|v| *v == firsts[0]);
    let second_uniform = seconds.iter().all(|v| *v == seconds[0]);
    let (e0, e1) = first_elems.expect("swap core holds N >= 2");
    let (h0, h1) = if first_uniform && second_uniform {
        (
            site_spelling(minuses[0], &e0, &firsts[0]),
            site_spelling(minuses[0], &e1, &seconds[0]),
        )
    } else if first_uniform {
        (site_spelling(minuses[0], &e0, &firsts[0]), "P1".to_string())
    } else if second_uniform {
        (
            "P0".to_string(),
            site_spelling(minuses[0], &e1, &seconds[0]),
        )
    } else {
        ("P0".to_string(), "P1".to_string())
    };
    let init_tpl = if init_texts.iter().all(|v| *v == init_texts[0]) {
        site_spelling(
            minuses[0],
            &first_init.expect("swap core holds N >= 2"),
            &init_texts[0],
        )
    } else {
        "E0".to_string()
    };
    let minus_tpl = format!("let ({h0}, {h1}) = {init_tpl};");
    let plus_tpl = format!("let ({h1}, {h0}) = {init_tpl};");
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
    fn parenthesized_fn_boxed_output_delta_generalizes() {
        // WI-6d: `Box<dyn Fn(..)>` nests Fn inputs at depth 3 (Box angle
        // → trait bound → path args); the parenthesized arm grants one
        // extra level so the hetero inputs hole instead of vetoing.
        // The delta is concrete (uniform `u8` → `String` output) with
        // the hole preserved across sides, per the correspondence law.
        let out = antiunify_strs(
            &[
                "Box<dyn Fn(u8) -> u8>",
                "Box<dyn Fn(u16) -> u8>",
                "Box<dyn Fn(u32) -> u8>",
                "Box<dyn Fn(bool) -> u8>",
            ],
            &[
                "Box<dyn Fn(u8) -> String>",
                "Box<dyn Fn(u16) -> String>",
                "Box<dyn Fn(u32) -> String>",
                "Box<dyn Fn(bool) -> String>",
            ],
        );
        assert_eq!(
            out,
            Some((
                "Box<dyn Fn(T0) -> u8>".to_string(),
                "Box<dyn Fn(T0) -> String>".to_string()
            )),
            "got: {out:?}"
        );
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

    fn untyped(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
        antiunify_untyped_let(minuses, pluses)
    }

    #[test]
    fn untyped_some_unwrap_fires() {
        // WI-8 slice 1 canonical shape: hetero `Some` patterns ride
        // `P0`, hetero inits ride the `E0` method-receiver hole.
        let out = untyped(
            &["let Some(v) = o;", "let Some(w) = p;"],
            &[
                "let Some(v) = o.unwrap_or(0);",
                "let Some(w) = p.unwrap_or(0);",
            ],
        );
        assert_eq!(
            out,
            Some((
                "let Some(P0) = E0;".to_string(),
                "let Some(P0) = E0.unwrap_or(0);".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn untyped_const_block_init_fires() {
        // WI-8 slice 1: `const` blocks are structural inits (shape
        // parity gate), so a uniform concrete delta anti-unifies.
        let out = untyped(
            &["let v = const { 1 + 2 };", "let w = const { 1 + 2 };"],
            &["let v = const { 1 + 3 };", "let w = const { 1 + 3 };"],
        );
        assert_eq!(
            out,
            Some((
                "let P0 = const { 1 + 2 };".to_string(),
                "let P0 = const { 1 + 3 };".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn untyped_infer_init_fires() {
        // WI-8 slice 1: `_` is a structural init (matches anything),
        // so a uniform `_` → concrete delta anti-unifies.
        let out = untyped(&["let v = _;", "let w = _;"], &["let v = 0;", "let w = 0;"]);
        assert_eq!(
            out,
            Some(("let P0 = _;".to_string(), "let P0 = 0;".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn untyped_macro_init_vetoes() {
        // Nested-macro classes belong to the macro-interior detector:
        // a template inlining macro text would steal them.
        let out = untyped(
            &["let a = m!(x);", "let b = m!(y);"],
            &["let a = m!(z);", "let b = m!(w);"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn untyped_single_vetoes() {
        let out = untyped(&["let a = x;"], &["let a = x.foo();"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn untyped_all_identical_declines_for_verbatim() {
        let out = untyped(
            &["let a = x;", "let a = x;"],
            &["let a = x.foo();", "let a = x.foo();"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    fn swap(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
        antiunify_pat_swap(minuses, pluses)
    }

    #[test]
    fn pat_swap_hetero_fires() {
        let out = swap(
            &["let (x, y) = t;", "let (m, n) = u;"],
            &["let (y, x) = t;", "let (n, m) = u;"],
        );
        assert_eq!(
            out,
            Some((
                "let (P0, P1) = E0;".to_string(),
                "let (P1, P0) = E0;".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn pat_swap_repeated_identical_declines_for_verbatim() {
        // Same swap at both sites: the concrete literal hunk is
        // tighter than any template (T422).
        let out = swap(
            &["let (x, y) = t;", "let (x, y) = t;"],
            &["let (y, x) = t;", "let (y, x) = t;"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn pat_swap_no_delta_vetoes() {
        let out = swap(
            &["let (x, y) = t;", "let (m, n) = u;"],
            &["let (x, y) = t;", "let (m, n) = u;"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn pat_swap_init_delta_vetoes() {
        // A per-site init delta is an untyped-let template, never a
        // swap (the init must be token-equal per site).
        let out = swap(
            &["let (x, y) = t;", "let (m, n) = u;"],
            &["let (y, x) = t2;", "let (n, m) = u2;"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn pat_swap_arity_vetoes() {
        let out = swap(
            &["let (x, y, z) = t;", "let (m, n, o) = u;"],
            &["let (z, y, x) = t;", "let (o, n, m) = u;"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn pat_swap_non_swap_vetoes() {
        // Same order both sides is not a swap (no delta at all here).
        let out = swap(
            &["let (x, y) = t;", "let (m, n) = u;"],
            &["let (x, y) = t;", "let (m, n) = u;"],
        );
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn pat_swap_half_uniform_pins_positional_holes() {
        // Minus-first uniform: concrete `x` + `P1`; the plus side
        // replays them swapped.
        let out = swap(
            &["let (x, y) = t;", "let (x, n) = u;"],
            &["let (y, x) = t;", "let (n, x) = u;"],
        );
        assert_eq!(
            out,
            Some((
                "let (x, P1) = E0;".to_string(),
                "let (P1, x) = E0;".to_string()
            )),
            "got: {out:?}"
        );
    }

    #[test]
    fn pat_swap_single_vetoes() {
        let out = swap(&["let (x, y) = t;"], &["let (y, x) = t;"]);
        assert_eq!(out, None, "got: {out:?}");
    }
}
