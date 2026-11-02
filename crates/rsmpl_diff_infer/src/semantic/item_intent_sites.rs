//! WI-5 slice-1 split (D5): item-intent use-site classifiers.
//!
//! The fn-arm call-callee / use-tail gates plus the dedicated-position
//! (call-callee / macro-head) carve-out, moved verbatim from
//! item_intent.rs so the pre-pass file stays under the 2000-line gate.
//! Tests stay inline in item_intent.rs (WI-22); this module holds only
//! the pure classifiers.

/// True when the use line's changed subterm sits in a dedicated
/// detector's position: a call callee or macro head naming `old`
/// (`Old(1)`, `m::Old::<T>(x)`, `old!(…)` incl. `let`-wrapped). Those
/// classes keep their pinned shapes; everything else (impl/fn headers,
/// type positions, struct literals/patterns, `use` lines, bare value
/// paths) stays claimable for the probe.
pub(crate) fn is_dedicated_use_site(minus: &str, old: &str) -> bool {
    let mut text = minus.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return false;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if is_dedicated_expr(&expr, old) {
            return true;
        }
    }
    // `let`-wrapped sites (`let b = old!(…);`) parse as statements.
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(minus.trim()) {
        if let Some(init) = local.init {
            if is_dedicated_expr(&init.expr, old) {
                return true;
            }
        }
    }
    false
}

/// WI-9: true when the use line's changed subterm is a *call callee*
/// naming `old` — the plain-`fn` follow domain (bare, qualified,
/// turbofish; `let`-wrapped included). Method calls, fields, bare
/// value paths, macro heads, and `use` lines all return false and
/// veto the `fn` intent. Arguments never count — only the head.
pub(crate) fn is_fn_call_use_site(minus: &str, old: &str) -> bool {
    let mut text = minus.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return false;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if is_fn_call_expr(&expr, old) {
            return true;
        }
    }
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(minus.trim()) {
        if let Some(init) = local.init {
            if is_fn_call_expr(&init.expr, old) {
                return true;
            }
        }
    }
    false
}

/// WI-2: true when the pair is a `use`-tail delta renaming exactly one
/// leaf ident `old` → `new` at a tail-or-direct-group-member position:
/// plain tails (`use crate::a::foo;`), grouped members
/// (`use crate::a::{foo, Other};`), `as`-preserving
/// (`use crate::a::foo as f;`), and `pub use` re-export forms. Both
/// lines must parse as `syn::ItemUse` with equal vis/attrs; glob trees,
/// nested member paths (`crate::{x::foo}` suffix), intermediate-segment
/// changes (`crate::foo::Bar`), and vis/attr deltas all return false.
/// The workspace scratch-apply probe below stays authoritative
/// (second-hop re-exports and off-diff same-tail uses veto there).
pub(crate) fn is_fn_use_tail_site(minus: &str, plus: &str, old: &str, new: &str) -> bool {
    let minus_item: syn::ItemUse = match syn::parse_str(minus.trim()) {
        Ok(item) => item,
        Err(_) => return false,
    };
    let plus_item: syn::ItemUse = match syn::parse_str(plus.trim()) {
        Ok(item) => item,
        Err(_) => return false,
    };
    let minus_vis = &minus_item.vis;
    let plus_vis = &plus_item.vis;
    if quote::quote!(#minus_vis).to_string() != quote::quote!(#plus_vis).to_string() {
        return false;
    }
    if minus_item.leading_colon != plus_item.leading_colon {
        return false;
    }
    if minus_item.attrs.len() != plus_item.attrs.len() {
        return false;
    }
    for (a, b) in minus_item.attrs.iter().zip(plus_item.attrs.iter()) {
        if quote::quote!(#a).to_string() != quote::quote!(#b).to_string() {
            return false;
        }
    }
    match use_tree_single_tail_delta(&minus_item.tree, &plus_item.tree, old, new, false) {
        Some(count) => count == 1,
        None => false,
    }
}

/// Single-tail delta walk over two `use` trees: `Some(n)` when the trees
/// are structurally identical except for `n` leaf `old` → `new` swaps at
/// allowed tail positions, `None` on any other divergence (glob, vis
/// already checked above, mismatched variants, intermediate changes).
/// `in_group` is true inside a `{…}` member list: a change under a
/// nested `Path` there (`{x::foo}`) is a suffix and vetoes.
pub(crate) fn use_tree_single_tail_delta(
    minus: &syn::UseTree,
    plus: &syn::UseTree,
    old: &str,
    new: &str,
    in_group: bool,
) -> Option<u32> {
    match (minus, plus) {
        (syn::UseTree::Path(m), syn::UseTree::Path(p)) => {
            if m.ident != p.ident {
                return None;
            }
            let inner = use_tree_single_tail_delta(&m.tree, &p.tree, old, new, in_group)?;
            if inner > 0 && in_group {
                return None;
            }
            Some(inner)
        }
        (syn::UseTree::Name(m), syn::UseTree::Name(p)) => {
            if m.ident == p.ident {
                Some(0)
            } else if m.ident == old && p.ident == new {
                Some(1)
            } else {
                None
            }
        }
        (syn::UseTree::Rename(m), syn::UseTree::Rename(p)) => {
            if m.rename != p.rename {
                return None;
            }
            if m.ident == p.ident {
                Some(0)
            } else if m.ident == old && p.ident == new {
                Some(1)
            } else {
                None
            }
        }
        (syn::UseTree::Group(m), syn::UseTree::Group(p)) => {
            if m.items.len() != p.items.len() {
                return None;
            }
            let mut total = 0u32;
            for (mm, pp) in m.items.iter().zip(p.items.iter()) {
                total += use_tree_single_tail_delta(mm, pp, old, new, true)?;
            }
            Some(total)
        }
        _ => None,
    }
}

/// True for call callees naming `old` (qualified and turbofish
/// spellings included — the engine's scalar follow rewrites them;
/// method calls never count: different namespace).
pub(crate) fn is_fn_call_expr(expr: &syn::Expr, old: &str) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                path.qself.is_none() && path.path.segments.last().is_some_and(|s| s.ident == old)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// True for call callees and macro heads naming `old` (qualified and
/// turbofish spellings included; arguments never count — only the
/// head position).
pub(crate) fn is_dedicated_expr(expr: &syn::Expr, old: &str) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                if path.qself.is_none() && path.path.segments.last().is_some_and(|s| s.ident == old)
                {
                    return true;
                }
            }
            false
        }
        syn::Expr::Macro(m) => m.mac.path.segments.last().is_some_and(|s| s.ident == old),
        _ => false,
    }
}
