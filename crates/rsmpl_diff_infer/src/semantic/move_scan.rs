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
        // WI-mod-content-cut + E2 slice 1 + WI-1a: the scalar `- I` cut
        // cannot isolate a content-bearing inline `mod` (it would match
        // nested items too — `expected one match, found N`), so those
        // stay ordinary. Outline `mod X;` declarations are never cut
        // items either (WI-1a: the F1 `535425f` shape cut
        // `mod sweep_tests;` while its `#[path = …]` line left as
        // ordinary, and the engine cannot match the bare cut at that
        // range — decl renames raise via the `mod` intent, decl
        // deletions stay ordinary). Method-bearing traits ARE eligible:
        // the engine's `outermost_spans` keeps the outermost (trait)
        // span for scalar cuts (probed `verified` for bare +
        // `trait New { ... }` evolution pastes), so the cut isolates the
        // whole trait. Empty `mod {}` still moves. Mirrors the
        // empty-only evolved `mod` narrowing (WI-8 slice 7); trait
        // evolution now admits token-equal methods (E2 slice 1).
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

/// WI-mod-content-cut + E2 slice 1 + WI-1a: scalar-cut eligibility for
/// moves.
///
/// The engine's `- I` cut matches nested items too (`collect_fns`
/// recurses into inline `mod` content; `collect_impl_method_spans`
/// collects trait methods), but scalar cuts keep the outermost span
/// (`outermost_spans` in `plan_cut.rs`), so a method-bearing trait
/// cuts as one capture (probed `verified`). Content-bearing `mod`
/// still stays ordinary (nested `fn` items would also pair as
/// independent moves, unlike trait methods which never move alone).
/// Outline `mod X;` declarations (`None` content) are never cut items
/// (WI-1a): their attributes split across hunks, so the cited span
/// cannot carry them whole, and the engine rejects the bare cut at
/// that range (F1 `535425f`). Empty inline `mod {}` (`Some` empty)
/// and all traits (empty or method-bearing) stay eligible.
pub(crate) fn move_cut_eligible(it: &syn::Item) -> bool {
    match it {
        syn::Item::Mod(m) => match &m.content {
            // WI-1a: outline declarations never move via bind cuts —
            // decl renames raise via the `mod` intent, decl deletions
            // stay ordinary.
            None => false,
            Some((_, inner)) => inner.is_empty(),
        },
        syn::Item::Trait(_) => true,
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
/// sub-range of a larger item) — or, under WI-14 slices 1-3 + slice 5,
/// with an item span minus exactly one leading added block: one
/// single-line `///` doc line or one balanced single `#[...]` attr
/// (single-line or multi-line, up to the slice-5 cap). The attr/doc
/// lines become a T432b plus-prefix ordinary leftover while the cut +
/// bare paste carry the move.
pub(crate) fn plus_run_is_item(body: &[String], first: usize, last: usize) -> bool {
    let Some(items) = plus_items_in(body) else {
        return false;
    };
    if items.iter().any(|it| it.first == first && it.last == last) {
        return true;
    }
    super::move_attr::plus_run_is_attr_added_item(body, first, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn outline_mod_decl_is_not_cut_eligible() {
        // WI-1a (F1 `535425f`): `mod sweep_tests;` parses but yields no
        // eligible move items — the raiser never cuts outline decls.
        let items = minus_items_in(&body(&["-mod sweep_tests;"])).expect("parses");
        assert_eq!(items.len(), 0);
        let items = plus_items_in(&body(&["+mod sweep_tests;"])).expect("parses");
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn empty_inline_mod_stays_eligible() {
        // Guards over-correction: `mod m {}` keeps its move eligibility.
        let items = minus_items_in(&body(&["-mod m {}"])).expect("parses");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ident, "m");
    }

    #[test]
    fn attributed_outline_mod_parses_but_declines() {
        // The exact F1 minus shape: attr + decl parse as one item whose
        // span covers both lines, but the outline decl is not cuttable.
        let items = minus_items_in(&body(&[
            "-#[path = \"sweep_tests.rs\"]",
            "-mod sweep_tests;",
        ]))
        .expect("parses");
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn plain_fn_stays_eligible() {
        // The gate narrows only outline mods; ordinary items unaffected.
        let items = minus_items_in(&body(&["-fn f() {}"])).expect("parses");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ident, "f");
    }
}
