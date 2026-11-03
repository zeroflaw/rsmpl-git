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
use super::move_evolve_tail::trait_header_prefix;
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

/// E2 slice 2a: body-evolving `trait` pair check — same shape modulo
/// the header ident, with a token-different item set the full-text
/// agreement in [`find_trait_body_evolved_hits`] proves is exactly the
/// old→new self-reference rename. Name-only scope: attrs, vis,
/// generics, and supertraits must be token-equal (any wider delta
/// vetoes); the names must differ. Returns the plus-side ident.
pub(crate) fn trait_body_evolved_match(
    minus: &syn::ItemTrait,
    plus: &syn::ItemTrait,
) -> Option<String> {
    if minus.auto_token.is_some() || plus.auto_token.is_some() {
        return None;
    }
    if minus.restriction.is_some() || plus.restriction.is_some() {
        return None;
    }
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
    if tok(&minus.supertraits) != tok(&plus.supertraits) {
        return None;
    }
    if minus.items.len() == plus.items.len()
        && minus
            .items
            .iter()
            .zip(plus.items.iter())
            .all(|(a, b)| tok(a) == tok(b))
    {
        return None; // no body delta — the abstract path owns it
    }
    Some(plus.ident.to_string())
}

/// E2 slice 2a: all body-evolving `trait` hits of one minus item text
/// in a pure-plus hunk body. Mirrors `find_struct_body_evolved_hits`;
/// spans coincide with parsed `trait` item spans by construction.
/// Returns `(plus_first, plus_last, plus_prefix, old_name, new_name)`:
/// the paste spells `+ <prefix>trait <old> as <new> { ... }` and the
/// engine renames the captured suffix/body through the shared scanner
/// — the same scanner proves the pairing here, so pairing and splice
/// agree by construction.
pub(crate) fn find_trait_body_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemTrait>(minus_text) else {
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
        let syn::Item::Trait(plus) = it else {
            continue;
        };
        let Some(new) = trait_body_evolved_match(&minus, plus) else {
            continue;
        };
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let Some(prefix) = trait_header_prefix(&plus_body[(s - 1)..e], &new) else {
            continue;
        };
        // Full-text agreement: the scanner-renamed minus item must equal
        // the plus run verbatim. Any non-rename delta (vis, generics,
        // supertraits, extra item changes, unrenamed occurrences) vetoes.
        let plus_text: String = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        if rsmpl_pattern::rename_ident_in_code(minus_text, &old, &new) != plus_text {
            continue;
        }
        // E2 slice 2a / WI-2: the same binding boundary as the
        // authored splice — a method ident, macro name, or attribute
        // occurrence stays ordinary even when the text scan agrees.
        if rsmpl_pattern::item_body_evolution_veto(minus_text, &old).is_some() {
            continue;
        }
        out.push((s - 1, e - 1, prefix, old.clone(), new));
    }
    out
}

/// E2 slice 2b: body-joint `trait` pair check — same shape modulo the
/// header ident, with exactly one method renamed plus the trait-rename
/// self-reference. Name-only scope: attrs, vis, generics, and supertraits
/// must be token-equal (any wider delta vetoes); the trait names must
/// differ. Returns `(plus_ident, m_old, m_new)` when exactly one item
/// index differs and that pair are both `TraitItemFn` with different
/// idents whose remaining tokens agree modulo both renames. The
/// finder below owns exactness through the full-text double scan; this
/// gate only classifies.
pub(crate) fn trait_body_joint_match(
    minus: &syn::ItemTrait,
    plus: &syn::ItemTrait,
) -> Option<(String, String, String)> {
    if minus.auto_token.is_some() || plus.auto_token.is_some() {
        return None;
    }
    if minus.restriction.is_some() || plus.restriction.is_some() {
        return None;
    }
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
        return None; // vacuous — identical headers never joint-fire
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.supertraits) != tok(&plus.supertraits) {
        return None;
    }
    if minus.items.len() != plus.items.len() {
        return None;
    }
    let mut joint: Option<(String, String)> = None;
    for (a, b) in minus.items.iter().zip(plus.items.iter()) {
        if tok(a) == tok(b) {
            continue;
        }
        let (syn::TraitItem::Fn(af), syn::TraitItem::Fn(bf)) = (a, b) else {
            return None;
        };
        if af.sig.ident == bf.sig.ident {
            return None; // same-name delta is a signature/body change, not a rename
        }
        if joint.is_some() {
            return None; // two method renames belong to no joint shape
        }
        // The pair must differ only by the method ident plus the two
        // renames (trait self-reference, method self-calls in a default
        // body): normalize all three and compare.
        let mut norm = af.clone();
        norm.sig.ident = bf.sig.ident.clone();
        let norm_text = tok(&syn::TraitItem::Fn(norm));
        let old = minus.ident.to_string();
        let new = plus.ident.to_string();
        let m_old = af.sig.ident.to_string();
        let m_new = bf.sig.ident.to_string();
        let once = rsmpl_pattern::rename_ident_in_code(&norm_text, &old, &new);
        if rsmpl_pattern::rename_ident_in_code(&once, &m_old, &m_new) != tok(b) {
            return None;
        }
        joint = Some((m_old, m_new));
    }
    let (m_old, m_new) = joint?;
    Some((plus.ident.to_string(), m_old, m_new))
}

/// E2 slice 2b: all body-joint `trait` hits of one minus item text in a
/// pure-plus hunk body. Mirrors `find_trait_body_evolved_hits`, with the
/// double scan (`Old→New` then `m_old→m_new`) proving the delta is exactly
/// the trait rename plus one method rename. Returns
/// `(plus_first, plus_last, plus_prefix, old_name, new_name, m_old,
/// m_new)`: the paste spells
/// `+ <prefix>trait <old> as <new> { fn <m_old> as <m_new>; ... }` and the
/// engine renames through the same double scan — pairing and splice agree
/// by construction.
pub(crate) fn find_trait_body_joint_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String, String, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemTrait>(minus_text) else {
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
        let syn::Item::Trait(plus) = it else {
            continue;
        };
        let Some((new, m_old, m_new)) = trait_body_joint_match(&minus, plus) else {
            continue;
        };
        // Pairwise distinctness: the double scan below would corrupt on
        // any cross-talk between the two renames.
        if m_old == m_new || old == m_old || old == m_new || new == m_old || new == m_new {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let Some(prefix) = trait_header_prefix(&plus_body[(s - 1)..e], &new) else {
            continue;
        };
        // Full-text agreement: the double-scanner-renamed minus item
        // must equal the plus run verbatim. Any non-rename delta vetoes.
        let plus_text: String = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        let renamed_once = rsmpl_pattern::rename_ident_in_code(minus_text, &old, &new);
        if rsmpl_pattern::rename_ident_in_code(&renamed_once, &m_old, &m_new) != plus_text {
            continue;
        }
        // E2 slice 2a / WI-2: the binding boundary for `Old`, plus the
        // joint boundary for `m_old` — a macro name or an attribute
        // interior spelling `m_old` stays ordinary even when the scan
        // agrees.
        if rsmpl_pattern::item_body_evolution_veto(minus_text, &old).is_some() {
            continue;
        }
        if rsmpl_pattern::trait_joint_method_veto(minus_text, &m_old).is_some() {
            continue;
        }
        out.push((s - 1, e - 1, prefix, old.clone(), new, m_old, m_new));
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

    #[test]
    fn trait_body_evolve_fires() {
        // E2 slice 2a: move+rename whose method signature names the
        // trait — pairs through the body-evolving finder (the abstract
        // finder vetoes item deltas).
        let minus = "trait Old {\n    fn make() -> Old;\n    fn use_it(&self);\n}";
        let body = plus_body("trait New {\n    fn make() -> New;\n    fn use_it(&self);\n}");
        let hits = find_trait_body_evolved_hits(minus, &body);
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!((hits[0].3.as_str(), hits[0].4.as_str()), ("Old", "New"));
    }

    #[test]
    fn trait_body_matcher_gates() {
        // Identical items belong to the abstract path, never here.
        let a: syn::ItemTrait = syn::parse_str("trait Old {\n    fn f(&self);\n}").unwrap();
        let b: syn::ItemTrait = syn::parse_str("trait New {\n    fn f(&self);\n}").unwrap();
        assert_eq!(trait_body_evolved_match(&a, &b), None);
        // Method rename passes the matcher (any item delta) but the
        // finder declines it: the scanner-renamed minus never equals a
        // method-renamed plus (slice 2b territory, pinned ordinary).
        let c: syn::ItemTrait = syn::parse_str("trait New {\n    fn g(&self);\n}").unwrap();
        assert_eq!(trait_body_evolved_match(&a, &c), Some("New".to_string()));
        let a_text = "trait Old {\n    fn f(&self);\n}";
        let c_body = plus_body("trait New {\n    fn g(&self);\n}");
        assert!(find_trait_body_evolved_hits(a_text, &c_body).is_empty());
        // Vis delta stays out (name-only scope).
        let d: syn::ItemTrait =
            syn::parse_str("pub trait New {\n    fn make() -> New;\n}").unwrap();
        let e: syn::ItemTrait = syn::parse_str("trait Old {\n    fn make() -> Old;\n}").unwrap();
        assert_eq!(trait_body_evolved_match(&e, &d), None);
        // Stale self-ref: renamed scan disagrees, finder declines.
        let stale = plus_body("trait New {\n    fn make() -> Old;\n    fn use_it(&self);\n}");
        let e_text = "trait Old {\n    fn make() -> Old;\n    fn use_it(&self);\n}";
        assert!(find_trait_body_evolved_hits(e_text, &stale).is_empty());
        // Method named like the trait: binding risk stays ordinary.
        let m = "trait Old {\n    fn Old(&self);\n}";
        let renamed = rsmpl_pattern::rename_ident_in_code(m, "Old", "New");
        assert!(find_trait_body_evolved_hits(m, &plus_body(&renamed)).is_empty());
    }

    #[test]
    fn trait_body_joint_fires() {
        // E2 slice 2b: move+rename plus one method rename pairs through
        // the body-joint finder (the abstract finder vetoes item deltas,
        // the self-ref finder fails the single scan).
        let minus = "trait Old {\n    fn m_old(&self);\n    fn keep(&self);\n}";
        let body = plus_body("trait New {\n    fn m_new(&self);\n    fn keep(&self);\n}");
        let hits = find_trait_body_joint_hits(minus, &body);
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        let (first, last, prefix, old, new, m_old, m_new) = &hits[0];
        assert_eq!((*first, *last), (0, 3));
        assert_eq!(prefix, "");
        assert_eq!(old.as_str(), "Old");
        assert_eq!(new.as_str(), "New");
        assert_eq!(m_old.as_str(), "m_old");
        assert_eq!(m_new.as_str(), "m_new");
    }

    #[test]
    fn trait_body_joint_self_ref_combo_fires() {
        // Method rename riding with a trait-rename self-reference in the
        // renamed method's signature: the matcher normalizes both.
        let minus = "trait Old {\n    fn m_old() -> Old;\n}";
        let body = plus_body("trait New {\n    fn m_new() -> New;\n}");
        let hits = find_trait_body_joint_hits(minus, &body);
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!((hits[0].5.as_str(), hits[0].6.as_str()), ("m_old", "m_new"));
    }

    #[test]
    fn trait_body_joint_self_call_fires() {
        // A default body calling the renamed method renames along: the
        // matcher and the finder normalize both renames.
        let minus = "trait Old {\n    fn m_old(&self) {\n        self.m_old();\n    }\n}";
        let body = plus_body("trait New {\n    fn m_new(&self) {\n        self.m_new();\n    }\n}");
        let hits = find_trait_body_joint_hits(minus, &body);
        assert_eq!(hits.len(), 1, "got: {hits:?}");
    }

    #[test]
    fn trait_body_joint_matcher_gates() {
        // Method-equal pairs belong to the abstract/self-ref arms, never here.
        let a: syn::ItemTrait = syn::parse_str("trait Old {\n    fn m_old(&self);\n}").unwrap();
        let same: syn::ItemTrait = syn::parse_str("trait New {\n    fn m_old(&self);\n}").unwrap();
        assert_eq!(trait_body_joint_match(&a, &same), None);
        // Same name on both sides is vacuous.
        let b: syn::ItemTrait = syn::parse_str("trait Old {\n    fn m_new(&self);\n}").unwrap();
        assert_eq!(trait_body_joint_match(&a, &b), None);
        // Vis delta stays out (name-only scope).
        let vis_plus: syn::ItemTrait =
            syn::parse_str("pub trait New {\n    fn m_new(&self);\n}").unwrap();
        assert_eq!(trait_body_joint_match(&a, &vis_plus), None);
        // Two method renames belong to no joint shape.
        let two_minus: syn::ItemTrait =
            syn::parse_str("trait Old {\n    fn a(&self);\n    fn b(&self);\n}").unwrap();
        let two_plus: syn::ItemTrait =
            syn::parse_str("trait New {\n    fn c(&self);\n    fn d(&self);\n}").unwrap();
        assert_eq!(trait_body_joint_match(&two_minus, &two_plus), None);
        let two_body = plus_body("trait New {\n    fn c(&self);\n    fn d(&self);\n}");
        assert!(find_trait_body_joint_hits(
            "trait Old {\n    fn a(&self);\n    fn b(&self);\n}",
            &two_body
        )
        .is_empty());
        // Same-name signature delta is a signature change, not a rename.
        let sig_minus: syn::ItemTrait = syn::parse_str("trait Old {\n    fn m(&self);\n}").unwrap();
        let sig_plus: syn::ItemTrait =
            syn::parse_str("trait New {\n    fn m(&self, x: u32);\n}").unwrap();
        assert_eq!(trait_body_joint_match(&sig_minus, &sig_plus), None);
        // Non-fn item delta vetoes (const renamed alongside — no shape).
        let const_minus: syn::ItemTrait =
            syn::parse_str("trait Old {\n    const C: u32;\n}").unwrap();
        let const_plus: syn::ItemTrait =
            syn::parse_str("trait New {\n    const D: u32;\n}").unwrap();
        assert_eq!(trait_body_joint_match(&const_minus, &const_plus), None);
        // Cross-talk idents decline in the finder, not the matcher.
        let x_minus = "trait Old {\n    fn New(&self);\n}";
        let x_body = plus_body("trait New {\n    fn m_new(&self);\n}");
        assert!(find_trait_body_joint_hits(x_minus, &x_body).is_empty());
        // A method named like the trait: binding risk stays ordinary.
        let m = "trait Old {\n    fn Old(&self);\n}";
        let m_body = plus_body("trait New {\n    fn m_new(&self);\n}");
        assert!(find_trait_body_joint_hits(m, &m_body).is_empty());
        // Macro/attr occurrences of the method name stay ordinary.
        let mac = "trait Old {\n    fn m_old(&self);\n    m_old!();\n}";
        let mac_body = plus_body("trait New {\n    fn m_new(&self);\n    m_new!();\n}");
        assert!(find_trait_body_joint_hits(mac, &mac_body).is_empty());
    }
}
