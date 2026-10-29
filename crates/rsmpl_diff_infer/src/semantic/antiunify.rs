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
//! This module generalizes two shapes: **let type-addition** — minus
//! `let PAT = INIT;` (no ascription) → plus `let PAT: T = INIT;`
//! (uniform concrete `T`, per-site preserved `PAT`/`INIT`; a varying
//! type would need an unbound plus-side `T0` (T442-illegal), so it
//! vetoes; a per-site init/pattern delta is not this shape (verbatim
//! and the call sweeps own those)) — and **type rewrite** (WI-4b):
//! `antiunify_types` over `syn::Type` pairs with hole correspondence.
//! Signature evolution (WI-4c) reuses this module next.

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

fn tok<T: ToTokens>(node: &T) -> String {
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
/// - macro/verbatype types veto (engine-opaque); rendered templates
///   must differ and re-parse as `syn::Type`.
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
/// agreeing `::` and arity and per-position generalization; `Fn(..)`
/// parenthesized args must agree (no holes inside — rare, safe).
fn gen_path_args(
    args: &[&syn::PathArguments],
    depth: usize,
    holes: &mut Vec<Hole>,
) -> Option<syn::PathArguments> {
    if args.iter().all(|a| matches!(a, syn::PathArguments::None)) {
        return Some(syn::PathArguments::None);
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
        // Macro types are engine-opaque.
        assert!(syn::parse_str::<syn::Type>("m!(x)").is_ok());
        let out = antiunify_strs(&["m!(x)", "m!(y)"], &["Box<m!(x)>", "Box<m!(y)>"]);
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
}
