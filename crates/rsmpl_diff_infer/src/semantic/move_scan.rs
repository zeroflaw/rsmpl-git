//! Whole-item scan helpers for move inference (D5 split): parsing pure
//! `-U0` hunk bodies into named items plus the run/ident utilities the
//! sweep consumes. Bodies byte-identical to `moves.rs`; the four
//! parent-used helpers are `pub(crate)`, the rest stay private.

use super::move_parts::{line_text, MoveItem};
use syn::spanned::Spanned;

/// Parse a pure-hunk body as a `syn::File`; `None` when any non-blank
/// line is not whole-item content. Returns the named items with
/// body-line spans (fn + struct/enum/trait/type/mod/union/const/static).
pub(crate) fn minus_items_in(body: &[String]) -> Option<Vec<MoveItem>> {
    file_items_in(body, false)
}

/// Lenient plus-side parse (T430): `//`-comment lines (`//`, `///`,
/// `//!` — all trivia outside item spans, e.g. file-header docs above
/// the moved items) are coverable leftovers, never vetoes. Block
/// `/* */` interiors still veto (rare; ordinary fallback).
pub(crate) fn plus_items_in(body: &[String]) -> Option<Vec<MoveItem>> {
    file_items_in(body, true)
}

fn file_items_in(body: &[String], comments_ok: bool) -> Option<Vec<MoveItem>> {
    let texts: Vec<&str> = body.iter().map(|l| line_text(l)).collect();
    let fragment = texts.join("\n");
    let file: syn::File = syn::parse_str(&fragment).ok()?;
    let mut covered = vec![false; body.len()];
    for it in &file.items {
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > body.len() {
            return None;
        }
        covered[(s - 1)..e].fill(true);
    }
    for (i, t) in texts.iter().enumerate() {
        let comment = comments_ok && t.trim_start().starts_with("//");
        if !covered[i] && !t.trim().is_empty() && !comment {
            return None;
        }
    }
    let mut items = Vec::new();
    for it in &file.items {
        // WI-mod-content-cut: the scalar `- I` cut cannot isolate an item
        // with nested items (it would match them too — `expected one match,
        // found N`). Content-bearing inline `mod` (≥1 nested item) and
        // method-bearing `trait` (any trait item) stay ordinary; empty
        // `mod {}` / `mod;` / empty trait still move. Mirrors the
        // empty-only evolved `mod`/`trait` narrowing (WI-8 slices 7-8).
        if !move_cut_eligible(it) {
            continue;
        }
        let Some(ident) = item_ident(it) else {
            continue;
        };
        let s = it.span().start().line;
        let e = it.span().end().line;
        let lines: Vec<String> = texts[(s - 1)..e].iter().map(|s| s.to_string()).collect();
        items.push(MoveItem {
            ident,
            text: lines.join("\n"),
            lines,
            first: s - 1,
            last: e - 1,
        });
    }
    Some(items)
}

/// WI-mod-content-cut: scalar-cut eligibility for byte-identical moves.
///
/// The engine's `- I` cut matches nested items too (`collect_fns` recurses
/// into inline `mod` content; `collect_impl_method_spans` collects trait
/// methods), so cutting a content-bearing `mod` or a method-bearing
/// `trait` always fails with `expected one match, found N`. Such items
/// stay ordinary. Empty inline `mod {}` (`Some` empty), outline
/// `mod;` (`None`), and empty traits stay eligible.
pub(crate) fn move_cut_eligible(it: &syn::Item) -> bool {
    match it {
        syn::Item::Mod(m) => match &m.content {
            None => true,
            Some((_, inner)) => inner.is_empty(),
        },
        syn::Item::Trait(t) => t.items.is_empty(),
        _ => true,
    }
}

/// Single plain ident for named items; `None` for ident-less items
/// (use decls, macros) which stay ordinary. Inherent `impl` blocks
/// contribute their self-ty last segment (`impl Old` → `Old`);
/// trait-impls (`impl Tr for Ty`) stay `None` (WI-3 owns that header).
pub(crate) fn item_ident(it: &syn::Item) -> Option<String> {
    match it {
        syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
        syn::Item::Struct(s) => Some(s.ident.to_string()),
        syn::Item::Enum(e) => Some(e.ident.to_string()),
        syn::Item::Trait(t) => Some(t.ident.to_string()),
        syn::Item::Type(t) => Some(t.ident.to_string()),
        syn::Item::Const(c) => Some(c.ident.to_string()),
        syn::Item::Static(s) => Some(s.ident.to_string()),
        syn::Item::Mod(m) => Some(m.ident.to_string()),
        syn::Item::Union(u) => Some(u.ident.to_string()),
        syn::Item::Impl(imp) if imp.trait_.is_none() => self_ty_ident(&imp.self_ty),
        _ => None,
    }
}

/// Last segment of a plain-path self-ty (`Old`, `Old<T>`, `crate::Old`).
/// Anything else (references, tuples, `dyn`, qself) is not a moveable
/// self-ty.
fn self_ty_ident(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(tp) = ty {
        if tp.qself.is_none() {
            return tp.path.segments.last().map(|s| s.ident.to_string());
        }
    }
    None
}

/// All occurrences of the contiguous `needle` line run in `body`
/// (0-based spans); callers require exactly one.
pub(crate) fn find_runs(body: &[String], needle: &[String]) -> Vec<(usize, usize)> {
    if needle.is_empty() || needle.len() > body.len() {
        return Vec::new();
    }
    let texts: Vec<&str> = body.iter().map(|l| line_text(l)).collect();
    let needles: Vec<&str> = needle.iter().map(|s| s.as_str()).collect();
    let mut out = Vec::new();
    for i in 0..=body.len() - needle.len() {
        if texts[i..i + needle.len()] == needles {
            out.push((i, i + needle.len() - 1));
        }
    }
    out
}

/// The plus run must coincide with a parsed item span (never a
/// sub-range of a larger item) — or, under WI-14 slices 1-3, with an
/// item span minus exactly one leading single-line `#[...]` / `///`
/// line (the attr/doc line becomes a T432b plus-prefix
/// ordinary leftover while the cut + bare paste carry the move).
pub(crate) fn plus_run_is_item(body: &[String], first: usize, last: usize) -> bool {
    let Some(items) = plus_items_in(body) else {
        return false;
    };
    if items.iter().any(|it| it.first == first && it.last == last) {
        return true;
    }
    super::move_attr::plus_run_is_attr_added_item(body, first, last)
}
