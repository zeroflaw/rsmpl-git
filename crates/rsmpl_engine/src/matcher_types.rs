//! Type and path matching for the general matcher (WI-2a split, D5).
//!
//! Split from `matcher_general.rs` (which sat at 1997 lines, at the
//! 2000-line gate): path-segment, generic-arg, turbofish, and `Type`
//! matching live here. `match_expr` stays in `matcher_general` (with the
//! WI-2a everyday-expression arms); the two modules call each other via
//! `crate::` paths, mirroring `matcher_general_ctrl` /
//! `matcher_general_async`.

use crate::matcher_general::{as_placeholder, render, render_eq, MatchCx};
use syn::spanned::Spanned;

/// Match pattern path segments against target segments. A whole-segment
/// placeholder binds the target segment text; concrete segments must equal.
/// (`qself` equality lives in the `matcher_spelling` sibling, WI-7d.)
pub(crate) fn match_path(pat: &syn::Path, tgt: &syn::Path, cx: &mut MatchCx) -> bool {
    if pat.leading_colon.is_some() != tgt.leading_colon.is_some() {
        return false;
    }
    if pat.segments.len() != tgt.segments.len() {
        return false;
    }
    for (ps, ts) in pat.segments.iter().zip(tgt.segments.iter()) {
        let id = ps.ident.to_string();
        if let Some(name) = as_placeholder(&cx.reverse, &id).cloned() {
            if !cx.bind(&name, &ts.ident.to_string()) {
                return false;
            }
            // Placeholder segments may carry generic arguments (T144):
            // `P<u8>`, `P::<T>` match structurally via `match_path_args`.
            // Bare `P` (no args) keeps the wildcard behavior below since
            // `match_path_args(None, <anything>)` only accepts `None`.
            // To preserve the historical bare-`P` wildcard (matches any
            // target args), skip the args check when the pattern has none.
            if !matches!(ps.arguments, syn::PathArguments::None)
                && !match_path_args(&ps.arguments, &ts.arguments, cx)
            {
                return false;
            }
            continue;
        }
        if ps.ident != ts.ident {
            return false;
        }
        if !match_path_args(&ps.arguments, &ts.arguments, cx) {
            return false;
        }
    }
    true
}

pub(crate) fn match_path_args(
    pat: &syn::PathArguments,
    tgt: &syn::PathArguments,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        (syn::PathArguments::None, syn::PathArguments::None) => true,
        (syn::PathArguments::AngleBracketed(a), syn::PathArguments::AngleBracketed(b)) => {
            if a.args.len() != b.args.len() {
                return false;
            }
            if a.colon2_token.is_some() != b.colon2_token.is_some() {
                return false;
            }
            for (pa, ta) in a.args.iter().zip(b.args.iter()) {
                if !match_generic_arg(pa, ta, cx) {
                    return false;
                }
            }
            true
        }
        // WI-6c: parenthesized `Fn(A) -> B` args match structurally —
        // inputs pairwise via `match_type` (holes bind), output via
        // `match_return_type`. Cross-kind pairs (e.g. `P(T)` vs
        // `Vec<u8>`) stay `render_eq` below (fail safe, never equal).
        (syn::PathArguments::Parenthesized(a), syn::PathArguments::Parenthesized(b)) => {
            a.inputs.len() == b.inputs.len()
                && a.inputs
                    .iter()
                    .zip(b.inputs.iter())
                    .all(|(x, y)| match_type(x, y, cx))
                && match_return_type(&a.output, &b.output, cx)
        }
        _ => render_eq(pat, tgt),
    }
}

pub(crate) fn match_generic_arg(
    pat: &syn::GenericArgument,
    tgt: &syn::GenericArgument,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        // Lifetimes are wildcards in v1 (documented).
        (syn::GenericArgument::Lifetime(_), syn::GenericArgument::Lifetime(_)) => true,
        (syn::GenericArgument::Type(pt), syn::GenericArgument::Type(tt)) => {
            if let Some(name) = single_placeholder_type(pt, cx) {
                return cx.bind_span(&name, tt.span());
            }
            match_type(pt, tt, cx)
        }
        (syn::GenericArgument::Const(a), syn::GenericArgument::Const(b)) => {
            crate::matcher_general::match_expr(a, b, cx)
        }
        _ => render_eq(pat, tgt),
    }
}

/// Structural method-call turbofish comparison (T144): `None/None`
/// matches, `Some/Some` compares `::` presence + each generic argument
/// via `match_generic_arg` (so `collect::<T>` binds `T`). Anything else
/// (including `None` vs `Some`) fails.
pub(crate) fn match_turbofish(
    pat: &Option<syn::AngleBracketedGenericArguments>,
    tgt: &Option<syn::AngleBracketedGenericArguments>,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if a.colon2_token.is_some() != b.colon2_token.is_some() {
                return false;
            }
            if a.args.len() != b.args.len() {
                return false;
            }
            for (pa, ta) in a.args.iter().zip(b.args.iter()) {
                if !match_generic_arg(pa, ta, cx) {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

/// Declared name if `t` is exactly one placeholder path (`__RSMPL_T`).
pub(crate) fn single_placeholder_type(t: &syn::Type, cx: &MatchCx) -> Option<String> {
    if let syn::Type::Path(tp) = t {
        if tp.qself.is_none() && tp.path.segments.len() == 1 {
            let seg = &tp.path.segments[0];
            if matches!(seg.arguments, syn::PathArguments::None) {
                return as_placeholder(&cx.reverse, &seg.ident.to_string()).cloned();
            }
        }
    }
    None
}

pub(crate) fn match_type(pat: &syn::Type, tgt: &syn::Type, cx: &mut MatchCx) -> bool {
    if let Some(name) = single_placeholder_type(pat, cx) {
        return cx.bind_span(&name, tgt.span());
    }
    match (pat, tgt) {
        (syn::Type::Path(a), syn::Type::Path(b)) => {
            // WI-7d-follow: type-position `qself` parity mirrors the
            // `Expr::Path` arm — `<T as Tr>::Assoc` never matches bare
            // `Assoc` and vice versa; trait prefix compares via
            // `match_path`, `ty` half fails closed on placeholders.
            crate::matcher_spelling::qself_syn_eq(&a.qself, &b.qself)
                && match_path(&a.path, &b.path, cx)
        }
        (syn::Type::Reference(a), syn::Type::Reference(b)) => {
            // Lifetimes are wildcards in v1 (documented).
            render(&a.mutability) == render(&b.mutability) && match_type(&a.elem, &b.elem, cx)
        }
        (syn::Type::Slice(a), syn::Type::Slice(b)) => match_type(&a.elem, &b.elem, cx),
        (syn::Type::Array(a), syn::Type::Array(b)) => {
            match_type(&a.elem, &b.elem, cx) && render_eq(&a.len, &b.len)
        }
        (syn::Type::Tuple(a), syn::Type::Tuple(b)) => {
            a.elems.len() == b.elems.len()
                && a.elems
                    .iter()
                    .zip(b.elems.iter())
                    .all(|(x, y)| match_type(x, y, cx))
        }
        (syn::Type::Paren(a), syn::Type::Paren(b)) => match_type(&a.elem, &b.elem, cx),
        (syn::Type::Group(a), syn::Type::Group(b)) => match_type(&a.elem, &b.elem, cx),
        (syn::Type::Ptr(a), syn::Type::Ptr(b)) => {
            // `*const T` never matches `*mut T`; the pointee recurses so a
            // `# type T` placeholder binds.
            render(&a.const_token) == render(&b.const_token)
                && render(&a.mutability) == render(&b.mutability)
                && match_type(&a.elem, &b.elem, cx)
        }
        (syn::Type::BareFn(a), syn::Type::BareFn(b)) => {
            // `for<'a>` binders are ignored (lifetime-wildcard spirit);
            // unsafety/ABI compare strictly; variadics never match (C1
            // rejects `...` in patterns, so a variadic target fails safe).
            a.variadic.is_none()
                && b.variadic.is_none()
                && render(&a.unsafety) == render(&b.unsafety)
                && render(&a.abi) == render(&b.abi)
                && a.inputs.len() == b.inputs.len()
                && a.inputs
                    .iter()
                    .zip(b.inputs.iter())
                    .all(|(x, y)| match_type(&x.ty, &y.ty, cx))
                && match_return_type(&a.output, &b.output, cx)
        }
        (syn::Type::ImplTrait(a), syn::Type::ImplTrait(b)) => {
            match_bounds(&a.bounds, &b.bounds, cx)
        }
        (syn::Type::TraitObject(a), syn::Type::TraitObject(b)) => {
            // `dyn` presence compares strictly; a bare path is a different
            // variant (`Type::Path`) and never reaches this arm.
            a.dyn_token.is_some() == b.dyn_token.is_some() && match_bounds(&a.bounds, &b.bounds, cx)
        }
        // `_` matches any type.
        (syn::Type::Infer(_), _) => true,
        (syn::Type::Never(_), syn::Type::Never(_)) => true,
        // WI-6c: macro types are verbatim-only — token-equal, never
        // holed. A substituted `__RSMPL_*` hole can never render-equal
        // a target, so holes inside macros reject by construction.
        (syn::Type::Macro(a), syn::Type::Macro(b)) => render_eq(a, b),
        _ => false,
    }
}

/// Match `impl Trait` / `dyn Trait` bound lists structurally (T149): same
/// length, each bound matches; plain lifetime bounds are wildcards per the
/// SPEC deliberate-wildcards rule, `for<'a>` binders and `?` modifiers
/// compare strictly.
fn match_bounds(
    pat: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    tgt: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    cx: &mut MatchCx,
) -> bool {
    pat.len() == tgt.len()
        && pat
            .iter()
            .zip(tgt.iter())
            .all(|(a, b)| match_type_param_bound(a, b, cx))
}

fn match_type_param_bound(
    pat: &syn::TypeParamBound,
    tgt: &syn::TypeParamBound,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        // Lifetimes are wildcards in v1 (documented).
        (syn::TypeParamBound::Lifetime(_), syn::TypeParamBound::Lifetime(_)) => true,
        (syn::TypeParamBound::Trait(a), syn::TypeParamBound::Trait(b)) => {
            render(&a.modifier) == render(&b.modifier)
                && render(&a.lifetimes) == render(&b.lifetimes)
                && match_path(&a.path, &b.path, cx)
        }
        _ => render_eq(pat, tgt),
    }
}

/// Match `fn` return types structurally (T149): absent/absent matches, and
/// a whole-type placeholder (`T`) binds the target span like elsewhere.
/// `pub(crate)` for `matcher_general_ctrl` closure arms (WI-5b).
pub(crate) fn match_return_type(
    pat: &syn::ReturnType,
    tgt: &syn::ReturnType,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        (syn::ReturnType::Default, syn::ReturnType::Default) => true,
        (syn::ReturnType::Type(_, a), syn::ReturnType::Type(_, b)) => {
            if let Some(name) = single_placeholder_type(a, cx) {
                return cx.bind_span(&name, b.span());
            }
            match_type(a, b, cx)
        }
        _ => false,
    }
}
