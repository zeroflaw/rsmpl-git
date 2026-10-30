//! Call-path spelling helpers (WI-7d). Split from `matcher_expr` per the
//! D5 hygiene gate (`matcher_expr.rs` sits near the 2000-line cap).
//!
//! Leading-`::` and `qself` qualifiers compare with parity (never
//! wildcard): a `::a::b` pattern matches only `::a::b` sites, and a
//! `<T as Tr>::old` pattern only identical-qualifier sites.
//! `syn::QSelf` carries only the `<T as …>` half — the trait path is the
//! first `position` segments of the sibling `Path` — so both render
//! together here. Placeholders never bind inside `qself` (fail closed).

use crate::edits::fragments_equal;

/// Render `<T>` / `<T as Trait>` for `q` (trait path = first
/// `q.position` segments of `path`). Token-normalized at comparison
/// time (`fragments_equal`), so spacing here is irrelevant.
pub(crate) fn qualifier_render(q: &syn::QSelf, path: &syn::Path) -> String {
    let mut out = String::from("<");
    let ty: &syn::Type = &q.ty;
    out.push_str(&quote::quote!(#ty).to_string());
    if q.as_token.is_some() {
        out.push_str(" as ");
        let trait_part: Vec<String> = path
            .segments
            .iter()
            .take(q.position)
            .map(|s| quote::quote!(#s).to_string())
            .collect();
        out.push_str(&trait_part.join("::"));
    }
    out.push('>');
    out
}

/// Token-normalized equality for `qself` qualifier keys
/// (`None` never equals `Some`; positions must agree; v1 has no holes
/// inside `qself`).
pub(crate) fn qself_key_eq(a: (&Option<String>, usize), b: (&Option<String>, usize)) -> bool {
    if a.1 != b.1 {
        return false;
    }
    match (&a.0, &b.0) {
        (None, None) => true,
        (Some(x), Some(y)) => fragments_equal(x, y),
        _ => false,
    }
}

/// `syn`-level `qself` equality for the general matcher: presence,
/// `position`, `as`, and written type compare token-equal (the trait
/// path itself is the first `position` segments of the sibling `Path`,
/// compared by `match_path`). Placeholders never bind inside `qself`.
pub(crate) fn qself_syn_eq(a: &Option<syn::QSelf>, b: &Option<syn::QSelf>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            let xt: &syn::Type = &x.ty;
            let yt: &syn::Type = &y.ty;
            x.position == y.position
                && x.as_token.is_some() == y.as_token.is_some()
                && fragments_equal(
                    &quote::quote!(#xt).to_string(),
                    &quote::quote!(#yt).to_string(),
                )
        }
        _ => false,
    }
}

/// A parsed concrete callee: leading-`::` flag, optional `qself`
/// qualifier text + position, and per-segment ident texts.
pub(crate) struct CalleeParts {
    pub(crate) leading: bool,
    pub(crate) qself: Option<String>,
    pub(crate) qself_pos: usize,
    pub(crate) segs: Vec<String>,
}

/// Parse a concrete callee (`::a::b`, `<T as Tr>::old`,
/// `<T as Tr>::a::b`) via a dummy call parse, so `::` inside `<…>`
/// never splits. Returns `None` for shapes that do not parse
/// (`...` path ellipsis, placeholders are ordinary idents and parse).
pub(crate) fn parse_callee_parts(callee: &str) -> Option<CalleeParts> {
    let probe = format!("{callee}( )");
    let syn::Expr::Call(call) = syn::parse_str::<syn::Expr>(&probe).ok()? else {
        return None;
    };
    let syn::Expr::Path(p) = call.func.as_ref() else {
        return None;
    };
    let (qself, qself_pos) = match &p.qself {
        Some(q) => (Some(qualifier_render(q, &p.path)), q.position),
        None => (None, 0),
    };
    Some(CalleeParts {
        leading: p.path.leading_colon.is_some(),
        qself,
        qself_pos,
        segs: p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect(),
    })
}

/// True when the callee carries a real segment turbofish (`::<…>` on a
/// path segment). Parsed structurally so a `qself` qualifier (`<T as
/// Tr>::old`) is not mistaken for one.
pub(crate) fn callee_has_turbo(callee: &str) -> bool {
    let probe = format!("{callee}( )");
    if let Ok(syn::Expr::Call(call)) = syn::parse_str::<syn::Expr>(&probe) {
        if let syn::Expr::Path(p) = call.func.as_ref() {
            return p.path.segments.iter().any(|s| {
                matches!(&s.arguments, syn::PathArguments::AngleBracketed(a) if a.colon2_token.is_some())
            });
        }
    }
    callee.contains('<')
}

/// Full written path of a call candidate (`::` + `qself::` + segments).
pub(crate) fn cand_full_path(leading: bool, qself: &Option<String>, segs: &[String]) -> String {
    let mut out = String::new();
    if leading {
        out.push_str("::");
    }
    if let Some(q) = qself {
        out.push_str(q);
        out.push_str("::");
    }
    out.push_str(&segs.join("::"));
    out
}

/// T145: structural function-call turbofish comparison (mirror of T144's
/// `MethodCall` arm). Moved from `matcher_expr` per the D5 size gate
/// (byte-identical move plus WI-7d parity). The pattern callee parses
/// as a call func (`syn::ExprPath`, so leading-`::` and `qself` spell);
/// target segments pair with their source-sliced generic args from
/// collection. Leading and `qself` compare with parity (never
/// wildcard): a `::a::b::<u8>` pattern matches only `::a::b::<u8>`
/// sites, and a `<T as Tr>::old::<u8>` pattern only identical-qself
/// sites.
pub(crate) fn match_callee_turbofish(
    pat_callee: &str,
    cand: &crate::matcher_expr::Candidate<'_>,
    reverse_map: &std::collections::HashMap<String, String>,
    symbols: &std::collections::HashMap<String, String>,
    bindings: &mut std::collections::HashMap<String, String>,
) -> bool {
    let probe = format!("{pat_callee}( )");
    let pat_expr: syn::Expr = match syn::parse_str(&probe) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let syn::Expr::Call(pat_call) = &pat_expr else {
        return false;
    };
    let syn::Expr::Path(pat_path) = pat_call.func.as_ref() else {
        return false;
    };
    // WI-7d parity: leading-`::` and `qself` must agree exactly.
    if pat_path.path.leading_colon.is_some() != cand.leading {
        return false;
    }
    let (pat_qself, pat_pos) = match &pat_path.qself {
        Some(q) => (Some(qualifier_render(q, &pat_path.path)), q.position),
        None => (None, 0),
    };
    if !qself_key_eq((&pat_qself, pat_pos), (&cand.qself, cand.qself_pos)) {
        return false;
    }
    let mut segs: Vec<syn::PathSegment> = pat_path.path.segments.iter().cloned().collect();
    // T199: a sole symbol ident with turbofish (`Old::<u8>` with
    // `# symbol Old=crate::old_api`) expands to its seeded path
    // segments; seeded idents compare by full-path equality (SPEC 19.3,
    // same rule as the bare-symbol arm) while the turbofish args compare
    // structurally below. Symbols are pre-bound, so no binding is added.
    // A placeholder-mapped ident keeps placeholder behavior (the bare
    // arm checks the reverse map first, too).
    if segs.len() == 1 {
        let ident = segs[0].ident.to_string();
        // A symbol name is also placeholder-substituted, so resolve
        // through the reverse map first (mirrors the bare-symbol arm).
        // A plain placeholder has no symbols entry and keeps its
        // behavior.
        if let Some(orig) = reverse_map.get(&ident) {
            if let Some(seed) = symbols.get(orig) {
                let seed_probe = format!("{seed}( )");
                let Ok(syn::Expr::Call(seed_call)) = syn::parse_str::<syn::Expr>(&seed_probe)
                else {
                    return false;
                };
                let syn::Expr::Path(seed_path) = seed_call.func.as_ref() else {
                    return false;
                };
                // WI-7d parity: a seeded path matches only same-leading,
                // qself-free sites (symbols are plain-path spellings).
                if seed_path.path.leading_colon.is_some() != cand.leading
                    || seed_path.qself.is_some()
                    || cand.qself.is_some()
                {
                    return false;
                }
                let mut seed_segs: Vec<_> = seed_path.path.segments.iter().cloned().collect();
                if let Some(last) = seed_segs.last_mut() {
                    last.arguments = segs[0].arguments.clone();
                }
                segs = seed_segs;
            }
        }
    }
    if segs.len() != cand.callee_segments.len() {
        return false;
    }
    for ((pat_seg, tgt_ident), tgt_args) in segs
        .iter()
        .zip(cand.callee_segments.iter())
        .zip(cand.callee_arg_texts.iter())
    {
        if pat_seg.ident == "..." {
            return false;
        }
        if let Some(orig) = reverse_map.get(&pat_seg.ident.to_string()) {
            let prev_ok = match bindings.get(orig) {
                Some(prev) => prev == tgt_ident,
                None => {
                    bindings.insert(orig.clone(), tgt_ident.clone());
                    true
                }
            };
            if !prev_ok {
                return false;
            }
            // T144 bare-placeholder parity: no-arg pattern skips the
            // args check; `P::<T>` compares structurally.
            if matches!(pat_seg.arguments, syn::PathArguments::None) {
                continue;
            }
        } else {
            if pat_seg.ident != *tgt_ident {
                return false;
            }
            if matches!(pat_seg.arguments, syn::PathArguments::None) && tgt_args.is_empty() {
                continue;
            }
        }
        let syn::PathArguments::AngleBracketed(pat_bracketed) = &pat_seg.arguments else {
            return false;
        };
        // A bare `id<T>` pattern never matches a real `id::<u8>` call
        // (it never did — a non-turbofish call site cannot exist).
        if pat_bracketed.colon2_token.is_none() {
            return false;
        }
        if pat_bracketed.args.len() != tgt_args.len() {
            return false;
        }
        for (pat_arg, tgt_text) in pat_bracketed.args.iter().zip(tgt_args.iter()) {
            if !match_callee_generic_arg(pat_arg, tgt_text, reverse_map, bindings) {
                return false;
            }
        }
    }
    true
}

/// One generic argument of a pattern turbofish against target source
/// text (moved from `matcher_expr` per the D5 size gate): single-
/// placeholder types bind; lifetimes wildcard; everything else
/// (concrete types, consts) compares token-normalized.
pub(crate) fn match_callee_generic_arg(
    pat_arg: &syn::GenericArgument,
    tgt_text: &str,
    reverse_map: &std::collections::HashMap<String, String>,
    bindings: &mut std::collections::HashMap<String, String>,
) -> bool {
    if let Some(orig) = single_placeholder_arg_type(pat_arg, reverse_map) {
        return match bindings.get(&orig) {
            Some(prev) => fragments_equal(prev, tgt_text),
            None => {
                bindings.insert(orig, tgt_text.to_string());
                true
            }
        };
    }
    if let syn::GenericArgument::Lifetime(_) = pat_arg {
        if syn::parse_str::<syn::Lifetime>(tgt_text).is_ok() {
            return true;
        }
    }
    fragments_equal(&quote::quote!(#pat_arg).to_string(), tgt_text)
}

/// Declared name if `arg` is a single-placeholder type (`T` for
/// `# type T`; moved from `matcher_expr` per the D5 size gate).
fn single_placeholder_arg_type(
    arg: &syn::GenericArgument,
    reverse_map: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let syn::GenericArgument::Type(syn::Type::Path(tp)) = arg else {
        return None;
    };
    if tp.qself.is_some()
        || tp.path.segments.len() != 1
        || !matches!(tp.path.segments[0].arguments, syn::PathArguments::None)
    {
        return None;
    }
    reverse_map
        .get(&tp.path.segments[0].ident.to_string())
        .cloned()
}
