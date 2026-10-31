//! Type-level anti-unification core (D5 split).
//!
//! Split from `antiunify.rs` (which sat at 2000 lines, at the
//! 2000-line gate): `TypedSite`, `antiunify_types`, and the
//! `gen_ty`/`gen_path`/`gen_bounds` generalization machinery live
//! here, plus the shared `tok` renderer both halves use.
//! `antiunify.rs` keeps the let/sequence cores and re-exports the
//! shared names so existing `super::antiunify::{…}` paths work.

use quote::ToTokens;
use syn::spanned::Spanned as _;

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
        // Span columns are Unicode scalars (SPEC 31.3), not bytes.
        let (s, e) = (
            rsmpl_pattern::char_col_to_byte(first_text, span.start().column),
            rsmpl_pattern::char_col_to_byte(first_text, span.end().column),
        );
        repls.push((s, e, idx));
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
        // WI-6d: the `dyn`/`impl` wrapper plus bound plus path
        // triple-count a single bound position against the depth ≤ 2
        // doctrine, so `Box<dyn Fn(..)>` holes land at depth 3 and veto.
        // Fn inputs/outputs are granted one extra level
        // (`saturating_sub(1)`): the shapes stay totality-gated and the
        // allowance applies only inside parenthesized positions, never
        // to the general core.
        for (i, input) in out.inputs.iter_mut().enumerate() {
            let nth: Vec<syn::Type> = pars.iter().map(|p| p.inputs[i].clone()).collect();
            *input = hole_or_gen(&nth, depth.saturating_sub(1), holes)?;
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
                **out_ty = hole_or_gen(&nth, depth.saturating_sub(1), holes)?;
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
