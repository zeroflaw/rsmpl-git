//! WI-19 slice 4: container (`struct`/`enum`/`union`) body-evolving
//! `as` move pairing.
//!
//! A `struct`/`enum`/`union` move whose body delta is exactly the
//! container rename (`Old` → `New` inside field types / variant
//! payloads) pairs like the `fn` body-evolving shape (WI-6 slice 3b):
//! vis, generics, and attrs stay token-equal (name-only scope — vis or
//! signature deltas belong to the abstract or sig paths, or stay
//! ordinary), the name differs, and the full-text agreement (the
//! [`rsmpl_pattern::rename_ident_in_code`] scanner-renamed minus item
//! == the plus run, raw string equality) plus the WI-2
//! [`rsmpl_pattern::item_body_evolution_veto`] boundary proves the
//! pairing — so pairing and splice agree by construction. Unit structs
//! (`;`-terminated) never fire (their fields are token-equal, so the
//! abstract path owns them).
//!
//! D5: this lives in a sibling module because `move_evolve.rs` (1426
//! lines) must not grow.

use super::move_evolve::{enum_header_prefix, union_header_prefix};
use super::move_evolve::{struct_header_prefix, tok};
use syn::spanned::Spanned;

/// Body-evolving `struct` pair check: same shape modulo the header
/// ident, with a token-different field set the full-text agreement in
/// [`find_struct_body_evolved_hits`] proves is exactly the old→new
/// rename. Name-only scope: vis, generics, and attrs must be
/// token-equal (any wider delta vetoes downstream anyway). Returns the
/// plus-side ident when the pair evolves.
pub(crate) fn struct_body_evolved_match(
    minus: &syn::ItemStruct,
    plus: &syn::ItemStruct,
) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    if tok(&minus.vis) != tok(&plus.vis) {
        return None;
    }
    if minus.ident == plus.ident {
        return None; // vacuous — the abstract path owns renames without body delta
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if minus.semi_token.is_some() != plus.semi_token.is_some() {
        return None;
    }
    if tok(&minus.fields) == tok(&plus.fields) {
        return None; // no body delta — the abstract path owns it
    }
    Some(plus.ident.to_string())
}

/// Body-evolving `enum` pair check: same shape modulo the header
/// ident, with a token-different variant set the full-text agreement
/// proves is exactly the old→new rename. Name-only scope like above.
pub(crate) fn enum_body_evolved_match(
    minus: &syn::ItemEnum,
    plus: &syn::ItemEnum,
) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    if tok(&minus.vis) != tok(&plus.vis) {
        return None;
    }
    if minus.ident == plus.ident {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.variants) == tok(&plus.variants) {
        return None;
    }
    Some(plus.ident.to_string())
}

/// Body-evolving `union` pair check: same shape modulo the header
/// ident, with a token-different field set the full-text agreement
/// proves is exactly the old→new rename. Name-only scope like above.
pub(crate) fn union_body_evolved_match(
    minus: &syn::ItemUnion,
    plus: &syn::ItemUnion,
) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    if tok(&minus.vis) != tok(&plus.vis) {
        return None;
    }
    if minus.ident == plus.ident {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.fields) == tok(&plus.fields) {
        return None;
    }
    Some(plus.ident.to_string())
}

/// All body-evolving `struct` hits of one minus item text in a
/// pure-plus hunk body. Mirrors
/// [`super::move_evolve::find_fn_body_evolved_hits`]; spans coincide
/// with parsed `struct` item spans by construction. Returns
/// `(plus_first, plus_last, plus_prefix, old_name, new_name)`: the
/// paste spells `+ <prefix>struct <old> as <new> { ... }` and the
/// engine renames the captured suffix/body through the shared scanner
/// — the same scanner proves the pairing here, so pairing and splice
/// agree by construction.
pub(crate) fn find_struct_body_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemStruct>(minus_text) else {
        return Vec::new();
    };
    let old = minus.ident.to_string();
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Struct(plus) = it else {
            continue;
        };
        let Some(new) = struct_body_evolved_match(&minus, plus) else {
            continue;
        };
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let Some(prefix) = struct_header_prefix(&plus_body[(s - 1)..e], &new) else {
            continue;
        };
        // Full-text agreement: the scanner-renamed minus item must equal
        // the plus run verbatim. Any non-rename delta (vis, generics,
        // attrs, extra field changes, unrenamed occurrences) vetoes.
        let plus_text: String = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        if rsmpl_pattern::rename_ident_in_code(minus_text, &old, &new) != plus_text {
            continue;
        }
        // WI-19 slice 4 / WI-2: the same binding boundary as the
        // authored splice — a field ident, variant ident, macro name,
        // or attribute occurrence stays ordinary even when the text
        // scan agrees.
        if rsmpl_pattern::item_body_evolution_veto(minus_text, &old).is_some() {
            continue;
        }
        out.push((s - 1, e - 1, prefix, old.clone(), new));
    }
    out
}

/// All body-evolving `enum` hits of one minus item text in a
/// pure-plus hunk body. Mirrors `find_struct_body_evolved_hits`.
pub(crate) fn find_enum_body_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemEnum>(minus_text) else {
        return Vec::new();
    };
    let old = minus.ident.to_string();
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Enum(plus) = it else {
            continue;
        };
        let Some(new) = enum_body_evolved_match(&minus, plus) else {
            continue;
        };
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let Some(prefix) = enum_header_prefix(&plus_body[(s - 1)..e], &new) else {
            continue;
        };
        let plus_text: String = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        if rsmpl_pattern::rename_ident_in_code(minus_text, &old, &new) != plus_text {
            continue;
        }
        if rsmpl_pattern::item_body_evolution_veto(minus_text, &old).is_some() {
            continue;
        }
        out.push((s - 1, e - 1, prefix, old.clone(), new));
    }
    out
}

/// All body-evolving `union` hits of one minus item text in a
/// pure-plus hunk body. Mirrors `find_struct_body_evolved_hits`.
pub(crate) fn find_union_body_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemUnion>(minus_text) else {
        return Vec::new();
    };
    let old = minus.ident.to_string();
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Union(plus) = it else {
            continue;
        };
        let Some(new) = union_body_evolved_match(&minus, plus) else {
            continue;
        };
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let Some(prefix) = union_header_prefix(&plus_body[(s - 1)..e], &new) else {
            continue;
        };
        let plus_text: String = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        if rsmpl_pattern::rename_ident_in_code(minus_text, &old, &new) != plus_text {
            continue;
        }
        if rsmpl_pattern::item_body_evolution_veto(minus_text, &old).is_some() {
            continue;
        }
        out.push((s - 1, e - 1, prefix, old.clone(), new));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plus_body(text: &str) -> Vec<String> {
        text.lines().map(|l| format!("+{l}")).collect()
    }

    #[test]
    fn struct_body_evolve_fires() {
        // The 140-shape: move+rename whose field type names the
        // container — pairs through the body-evolving finder (the
        // abstract finder vetoes field deltas, pinned by shape).
        let minus = "struct Old {\n    next: Option<Box<Old>>,\n}";
        let body = plus_body("struct New {\n    next: Option<Box<New>>,\n}");
        let hits = find_struct_body_evolved_hits(minus, &body);
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        let (first, last, prefix, old, new) = &hits[0];
        assert_eq!((*first, *last), (0, 2));
        assert_eq!(prefix, "");
        assert_eq!((old.as_str(), new.as_str()), ("Old", "New"));
    }

    #[test]
    fn enum_body_evolve_fires() {
        let minus = "enum Old {\n    A(Box<Old>),\n    B,\n}";
        let body = plus_body("enum New {\n    A(Box<New>),\n    B,\n}");
        let hits = find_enum_body_evolved_hits(minus, &body);
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!((hits[0].3.as_str(), hits[0].4.as_str()), ("Old", "New"));
    }

    #[test]
    fn union_body_evolve_fires() {
        let minus = "union Old {\n    x: Old,\n    y: u32,\n}";
        let body = plus_body("union New {\n    x: New,\n    y: u32,\n}");
        let hits = find_union_body_evolved_hits(minus, &body);
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!((hits[0].3.as_str(), hits[0].4.as_str()), ("Old", "New"));
    }

    #[test]
    fn body_matcher_gates() {
        // Identical bodies belong to the abstract path, never here.
        let a: syn::ItemStruct = syn::parse_str("struct Old {\n    x: u32,\n}").unwrap();
        let b: syn::ItemStruct = syn::parse_str("struct New {\n    x: u32,\n}").unwrap();
        assert_eq!(struct_body_evolved_match(&a, &b), None);
        // Same name is vacuous.
        let c: syn::ItemStruct = syn::parse_str("struct Old {\n    x: Old,\n}").unwrap();
        let d: syn::ItemStruct = syn::parse_str("struct Old {\n    x: New,\n}").unwrap();
        assert_eq!(struct_body_evolved_match(&c, &d), None);
        // Vis / generic / attr deltas stay out (name-only scope).
        let e: syn::ItemStruct = syn::parse_str("pub struct New {\n    x: New,\n}").unwrap();
        assert_eq!(struct_body_evolved_match(&a, &e), None);
        let f: syn::ItemStruct = syn::parse_str("struct New<T> {\n    x: T,\n}").unwrap();
        let g: syn::ItemStruct = syn::parse_str("struct Old<T> {\n    x: T,\n}").unwrap();
        assert_eq!(struct_body_evolved_match(&g, &f), None);
        // Unit structs never fire (no body to rename).
        let u1: syn::ItemStruct = syn::parse_str("struct Old;").unwrap();
        let u2: syn::ItemStruct = syn::parse_str("struct New;").unwrap();
        assert_eq!(struct_body_evolved_match(&u1, &u2), None);
    }

    #[test]
    fn container_finder_declines_binding_risks() {
        // WI-19 slice 4 / WI-2: even when the text scan agrees, a field
        // ident, macro name, or attribute occurrence stays ordinary.
        for minus in [
            "struct Old {\n    Old: u32,\n    x: Old,\n}",
            "struct Old {\n    x: Old!(),\n}",
        ] {
            let renamed = rsmpl_pattern::rename_ident_in_code(minus, "Old", "New");
            let body = plus_body(&renamed);
            assert!(
                find_struct_body_evolved_hits(minus, &body).is_empty(),
                "binding risk stays ordinary: {minus}"
            );
        }
        // A variant named like the container strands the enum finder.
        let minus_v = "enum Old {\n    Old,\n    B,\n}";
        let renamed = rsmpl_pattern::rename_ident_in_code(minus_v, "Old", "New");
        let body = plus_body(&renamed);
        assert!(find_enum_body_evolved_hits(minus_v, &body).is_empty());
    }

    #[test]
    fn container_finder_vetoes_non_rename_body_delta() {
        // One extra field change beyond the rename: ordinary.
        let minus = "struct Old {\n    next: Box<Old>,\n    n: u32,\n}";
        let body = plus_body("struct New {\n    next: Box<New>,\n    n: u64,\n}");
        assert!(find_struct_body_evolved_hits(minus, &body).is_empty());
        // Stale self-ref: token-equal bodies belong to the abstract path.
        let stale = plus_body("struct New {\n    next: Box<Old>,\n}");
        assert!(find_struct_body_evolved_hits(minus, &stale).is_empty());
    }
}
