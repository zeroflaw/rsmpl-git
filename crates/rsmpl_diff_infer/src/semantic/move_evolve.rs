//! WI-8a + WI-8 slices 1-3: `fn`, `struct`, `enum`, and `union` move +
//! header evolution pairing (`type`/`const`/`static`/`mod`/`trait`
//! slices 4-8 live in `move_evolve_tail`, re-exported below).
//!
//! Byte-identical pairing (`moves::try_move_sweep`) misses moves that
//! also rename the item or change its header — the common case
//! (`fn foo` → `pub async fn bar`, `struct Old` → `pub struct New`,
//! `enum Old` → `pub enum New`, `union Old` → `pub union New` in a new
//! module file). This module pairs `fn`/`struct`/`enum`/`union` items
//! whose bodies (and signatures/fields/variants/aliased types/const
//! type+expr/static type+expr/empty-or-absent mod content/generics+supertraits+trait items) are token-equal but
//! whose header prefix (vis + qualifiers, verbatim) and/or name evolve,
//! and builds the single-line header template the engine's evolution
//! paste (`# use=N` + `+ <prefix>fn <New>(...) { ... }` /
//! `+ <prefix>struct <New> { ... }` / `+ <prefix>enum <New> { ... }` /
//! `+ <prefix>union <New> { ... }` / `+ <prefix>type <New> = ...;` /
//! `+ <prefix>const <New> = ...;` / `+ <prefix>mod <New> { ... }` /
//! `+ <prefix>trait <New> { ... }` /
//! `+ <prefix>static [mut ]<New> = ...;`) splices the captured body under.
//!
//! Scope is deliberately narrow (WI-8a + slices 1-3): `fn`, `struct`,
//! `enum`, and `union` items only (slices 4-8 live in `move_evolve_tail`); generics, params/fields/variants/
//! aliased type/const type+expr/static type+expr/empty-or-absent mod content/empty trait items, return type, `where` clause, attrs, and bodies must be token-equal.
//! The evolving prefix is verbatim plus-side text, never an inferred
//! hole, so qualifier evolution (`fn` → `pub async fn`) rides the same
//! safe mechanism as vis/name. `static mut` mutability rides verbatim
//! in the template the same way (a `static` vs `static mut` delta is an
//! evolution, not a veto). `unsafe mod`/`unsafe trait` ride the same way (an
//! `unsafe` delta is part of the verbatim prefix before `mod`/`trait`).
//! `auto trait` (and `impl Trait for` restrictions) never evolve —
//! they veto to ordinary, mirroring the authoring-side `auto trait`
//! rejection. Everything else (other item kinds,
//! attr/signature/field/variant/aliased-type/body deltas) stays ordinary — the
//! byte-exact path owns identical items (an evolved pair requires a
//! prefix, name, or (for `static`) mutability delta, or (for `trait`)
//! unsafety delta).

use quote::ToTokens;
use syn::spanned::Spanned;

pub(crate) use super::move_evolve_tail::*;

pub(crate) fn tok<T: ToTokens>(node: &T) -> String {
    node.to_token_stream().to_string()
}

pub(crate) fn opt_tok<T: ToTokens>(node: &Option<T>) -> String {
    node.as_ref().map(tok).unwrap_or_default()
}

/// Evolved `fn` pair check: same shape modulo the header prefix
/// (vis + qualifiers) and the ident. Returns the plus-side ident
/// when the pair evolves (prefix extraction needs the plus-side
/// *text*, so the caller pulls it from the item span lines).
pub(crate) fn fn_evolved_match(minus: &syn::ItemFn, plus: &syn::ItemFn) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.sig.ident == plus.sig.ident;
    if vis_same && name_same {
        return None; // identical — the byte-exact path owns it
    }
    let (a, b) = (&minus.sig, &plus.sig);
    // Qualifiers (`const`/`async`/`unsafe`/`extern`) live in the
    // evolving prefix and ride along verbatim — only the signature
    // slots below must be token-equal.
    let sig_same = tok(&a.generics.params) == tok(&b.generics.params)
        && opt_tok(&a.generics.where_clause) == opt_tok(&b.generics.where_clause)
        && tok(&a.inputs) == tok(&b.inputs)
        && a.variadic.is_some() == b.variadic.is_some()
        && opt_tok(&a.variadic) == opt_tok(&b.variadic)
        && tok(&a.output) == tok(&b.output);
    if !sig_same {
        return None;
    }
    if tok(&minus.block) != tok(&plus.block) {
        return None;
    }
    Some(plus.sig.ident.to_string())
}

/// Evolved `struct` pair check (WI-8 slice 1): same shape modulo the
/// visibility prefix and the ident. Generics, where-clauses, attrs,
/// and fields must be token-equal. Returns the plus-side ident when
/// the pair evolves (a vis or name delta is required — the byte-exact
/// path owns identical items).
pub(crate) fn struct_evolved_match(
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
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.ident == plus.ident;
    if vis_same && name_same {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.fields) != tok(&plus.fields) {
        return None;
    }
    if minus.semi_token.is_some() != plus.semi_token.is_some() {
        return None;
    }
    Some(plus.ident.to_string())
}
/// All evolved hits of one minus item text in a pure-plus hunk body.
/// Returns `(plus_first, plus_last, plus_prefix, plus_name)` with
/// 0-based hunk-body spans; the spans coincide with parsed `fn` item
/// spans by construction (never sub-ranges).
pub(crate) fn find_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemFn>(minus_text) else {
        return Vec::new();
    };
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
        let syn::Item::Fn(plus) = it else {
            continue;
        };
        if fn_evolved_match(&minus, plus).is_none() {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let name = plus.sig.ident.to_string();
        let Some(prefix) = header_prefix(&plus_body[(s - 1)..e], &name) else {
            continue;
        };
        out.push((s - 1, e - 1, prefix, name));
    }
    out
}

/// All evolved `struct` hits of one minus item text in a pure-plus
/// hunk body (WI-8 slice 1). Mirrors `find_evolved_hits`; spans
/// coincide with parsed `struct` item spans by construction.
pub(crate) fn find_struct_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemStruct>(minus_text) else {
        return Vec::new();
    };
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
        if struct_evolved_match(&minus, plus).is_none() {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let name = plus.ident.to_string();
        let Some(prefix) = struct_header_prefix(&plus_body[(s - 1)..e], &name) else {
            continue;
        };
        out.push((s - 1, e - 1, prefix, name));
    }
    out
}

/// The verbatim text before the `fn <name>` keyword on the item's
/// header line (vis + qualifiers, leading whitespace stripped).
/// Attr/doc lines never match: the header is always a code line.
pub(crate) fn header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(fn_pos) = find_fn_name(text, name) {
            return Some(text[..fn_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `fn <name>` pair (`None` when the
/// line holds no such pair: comments, strings, or other idents).
fn find_fn_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..].starts_with(b"fn")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 2).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 2;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line evolution-paste template: the plus-side prefix
/// (vis + qualifiers) verbatim, `(...)` params, `...` body.
/// `prefix` is trimmed; an empty prefix yields a bare `fn` header.
pub(crate) fn evolution_template(prefix: &str, name: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("fn {name}(...) {{ ... }}")
    } else {
        format!("{pre} fn {name}(...) {{ ... }}")
    }
}

/// The verbatim text before the `struct <name>` keyword on the
/// item's header line (visibility, leading whitespace stripped).
/// Attr/doc lines never match: the header is always a code line.
pub(crate) fn struct_header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(s_pos) = find_struct_name(text, name) {
            return Some(text[..s_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `struct <name>` pair (`None` when
/// the line holds no such pair: comments, strings, other idents).
fn find_struct_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..].starts_with(b"struct")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 6;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line struct evolution-paste template: the plus-side
/// visibility verbatim, `{ ... }` body marker. `prefix` is trimmed;
/// an empty prefix yields a bare `struct` header. The `{ ... }` tail
/// is a shape marker only — the splice keeps the captured suffix
/// (generics/fields) verbatim, so tuple/unit structs round-trip.
pub(crate) fn struct_evolution_template(prefix: &str, name: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("struct {name} {{ ... }}")
    } else {
        format!("{pre} struct {name} {{ ... }}")
    }
}

/// Evolved `enum` pair check (WI-8 slice 2): same shape modulo the
/// visibility prefix and the ident. Generics (params + where),
/// attrs, and variants must be token-equal. Returns the plus-side
/// ident when the pair evolves (a vis or name delta is required —
/// the byte-exact path owns identical items).
pub(crate) fn enum_evolved_match(minus: &syn::ItemEnum, plus: &syn::ItemEnum) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.ident == plus.ident;
    if vis_same && name_same {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.variants) != tok(&plus.variants) {
        return None;
    }
    Some(plus.ident.to_string())
}

/// All evolved `enum` hits of one minus item text in a pure-plus
/// hunk body (WI-8 slice 2). Mirrors `find_struct_evolved_hits`;
/// spans coincide with parsed `enum` item spans by construction.
pub(crate) fn find_enum_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemEnum>(minus_text) else {
        return Vec::new();
    };
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
        if enum_evolved_match(&minus, plus).is_none() {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let name = plus.ident.to_string();
        let Some(prefix) = enum_header_prefix(&plus_body[(s - 1)..e], &name) else {
            continue;
        };
        out.push((s - 1, e - 1, prefix, name));
    }
    out
}

/// The verbatim text before the `enum <name>` keyword on the item's
/// header line (visibility, leading whitespace stripped).
/// Attr/doc lines never match: the header is always a code line.
pub(crate) fn enum_header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(e_pos) = find_enum_name(text, name) {
            return Some(text[..e_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `enum <name>` pair (`None` when
/// the line holds no such pair: comments, strings, or other idents).
fn find_enum_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"enum")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 4;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line enum evolution-paste template: the plus-side
/// visibility verbatim, `{ ... }` body marker. `prefix` is trimmed;
/// an empty prefix yields a bare `enum` header. The `{ ... }` tail
/// is a shape marker only — the splice keeps the captured suffix
/// (generics/variants) verbatim.
pub(crate) fn enum_evolution_template(prefix: &str, name: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("enum {name} {{ ... }}")
    } else {
        format!("{pre} enum {name} {{ ... }}")
    }
}

/// Evolved `union` pair check (WI-8 slice 3): same shape modulo the
/// visibility prefix and the ident. Generics (params + where),
/// attrs, and fields must be token-equal. Returns the plus-side
/// ident when the pair evolves (a vis or name delta is required —
/// the byte-exact path owns identical items).
pub(crate) fn union_evolved_match(minus: &syn::ItemUnion, plus: &syn::ItemUnion) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.ident == plus.ident;
    if vis_same && name_same {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.fields) != tok(&plus.fields) {
        return None;
    }
    Some(plus.ident.to_string())
}

/// All evolved `union` hits of one minus item text in a pure-plus
/// hunk body (WI-8 slice 3). Mirrors `find_enum_evolved_hits`;
/// spans coincide with parsed `union` item spans by construction.
pub(crate) fn find_union_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemUnion>(minus_text) else {
        return Vec::new();
    };
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
        if union_evolved_match(&minus, plus).is_none() {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let name = plus.ident.to_string();
        let Some(prefix) = union_header_prefix(&plus_body[(s - 1)..e], &name) else {
            continue;
        };
        out.push((s - 1, e - 1, prefix, name));
    }
    out
}

/// The verbatim text before the `union <name>` keyword on the item's
/// header line (visibility, leading whitespace stripped).
/// Attr/doc lines never match: the header is always a code line.
pub(crate) fn union_header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(u_pos) = find_union_name(text, name) {
            return Some(text[..u_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `union <name>` pair (`None` when
/// the line holds no such pair: comments, strings, or other idents).
fn find_union_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"union")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 5;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line union evolution-paste template: the plus-side
/// visibility verbatim, `{ ... }` body marker. `prefix` is trimmed;
/// an empty prefix yields a bare `union` header. The `{ ... }` tail
/// is a shape marker only — the splice keeps the captured suffix
/// (generics/where/fields) verbatim.
pub(crate) fn union_evolution_template(prefix: &str, name: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("union {name} {{ ... }}")
    } else {
        format!("{pre} union {name} {{ ... }}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_fn(text: &str) -> syn::ItemFn {
        syn::parse_str(text).expect("test fn must parse")
    }

    const BODY: &str = "{\n    let y = x + 1;\n    y * 2\n}";

    #[test]
    fn vis_and_name_evolve_fires() {
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("pub fn new(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_evolved_match(&minus, &plus), Some("new".to_string()));
    }

    #[test]
    fn name_only_evolve_fires() {
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("fn new(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_evolved_match(&minus, &plus), Some("new".to_string()));
    }

    #[test]
    fn vis_only_evolve_fires() {
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("pub(crate) fn old(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_evolved_match(&minus, &plus), Some("old".to_string()));
    }

    #[test]
    fn qualifier_evolution_rides_the_verbatim_prefix() {
        // Qualifiers live in the evolving prefix: `fn` → `pub async fn`
        // fires (the prefix is verbatim plus-side text, never a hole).
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("pub async fn new(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_evolved_match(&minus, &plus), Some("new".to_string()));
        // Same qualifiers on both sides fire.
        let minus = item_fn(&format!("pub async fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("pub async fn new(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_evolved_match(&minus, &plus), Some("new".to_string()));
    }

    #[test]
    fn identical_pair_is_not_evolved() {
        let a = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let b = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_evolved_match(&a, &b), None);
    }

    #[test]
    fn body_param_return_generic_attr_deltas_veto() {
        let base = format!("fn old(x: i32) -> i32 {BODY}");
        let minus = item_fn(&base);
        // Body delta.
        let plus = item_fn("fn new(x: i32) -> i32 {\n    x\n}");
        assert_eq!(fn_evolved_match(&minus, &plus), None);
        // Param delta.
        let plus = item_fn(&format!("pub fn new(x: i64) -> i32 {BODY}"));
        assert_eq!(fn_evolved_match(&minus, &plus), None);
        // Return delta.
        let plus = item_fn(&format!("pub fn new(x: i32) -> i64 {BODY}"));
        assert_eq!(fn_evolved_match(&minus, &plus), None);
        // Generic delta.
        let minus_g = item_fn(&format!("fn old<T>(x: T) -> T {BODY}"));
        let plus_g = item_fn(&format!("pub fn new<T: Clone>(x: T) -> T {BODY}"));
        assert_eq!(fn_evolved_match(&minus_g, &plus_g), None);
        // Attr delta.
        let plus_a = item_fn(&format!("#[inline]\npub fn new(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_evolved_match(&minus, &plus_a), None);
    }

    #[test]
    fn template_shapes() {
        assert_eq!(evolution_template("pub ", "new"), "pub fn new(...) { ... }");
        assert_eq!(evolution_template("", "new"), "fn new(...) { ... }");
        assert_eq!(
            evolution_template("pub(crate) async ", "new"),
            "pub(crate) async fn new(...) { ... }"
        );
    }

    #[test]
    fn header_prefix_skips_attrs_and_docs() {
        let lines = vec![
            "+#[inline]".to_string(),
            "+/// calls fn helper".to_string(),
            "+pub fn helper(x: i32) -> i32 {".to_string(),
            "+    x".to_string(),
            "+}".to_string(),
        ];
        assert_eq!(header_prefix(&lines, "helper"), Some("pub ".to_string()));
    }

    fn item_struct(text: &str) -> syn::ItemStruct {
        syn::parse_str(text).expect("test struct must parse")
    }

    #[test]
    fn struct_vis_and_name_evolve_fires() {
        let minus = item_struct("struct Old {\n    x: i32,\n}");
        let plus = item_struct("pub struct New {\n    x: i32,\n}");
        assert_eq!(struct_evolved_match(&minus, &plus), Some("New".to_string()));
    }

    #[test]
    fn struct_name_only_and_vis_only_fire() {
        let minus = item_struct("struct Old {\n    x: i32,\n}");
        let plus = item_struct("struct New {\n    x: i32,\n}");
        assert_eq!(struct_evolved_match(&minus, &plus), Some("New".to_string()));
        let plus = item_struct("pub(crate) struct Old {\n    x: i32,\n}");
        assert_eq!(struct_evolved_match(&minus, &plus), Some("Old".to_string()));
    }

    #[test]
    fn struct_identical_and_deltas_veto() {
        let base = item_struct("struct Old {\n    x: i32,\n}");
        let same = item_struct("struct Old {\n    x: i32,\n}");
        assert_eq!(struct_evolved_match(&base, &same), None);
        // Field delta.
        let plus = item_struct("pub struct New {\n    y: i64,\n}");
        assert_eq!(struct_evolved_match(&base, &plus), None);
        // Generic delta.
        let minus_g = item_struct("struct Old<T> {\n    x: T,\n}");
        let plus_g = item_struct("pub struct New<T: Clone> {\n    x: T,\n}");
        assert_eq!(struct_evolved_match(&minus_g, &plus_g), None);
        // Tuple-vs-braced shape delta (fields differ).
        let plus_t = item_struct("pub struct New(u8);");
        assert_eq!(struct_evolved_match(&base, &plus_t), None);
        // Attr delta.
        let plus_a = item_struct("#[derive(Clone)]\npub struct New {\n    x: i32,\n}");
        assert_eq!(struct_evolved_match(&base, &plus_a), None);
    }

    #[test]
    fn struct_template_shapes() {
        assert_eq!(
            struct_evolution_template("pub ", "New"),
            "pub struct New { ... }"
        );
        assert_eq!(struct_evolution_template("", "New"), "struct New { ... }");
    }

    fn item_enum(text: &str) -> syn::ItemEnum {
        syn::parse_str(text).expect("test enum must parse")
    }

    #[test]
    fn enum_vis_and_name_evolve_fires() {
        let minus = item_enum("enum Old {\n    A,\n    B(i32),\n}");
        let plus = item_enum("pub enum New {\n    A,\n    B(i32),\n}");
        assert_eq!(enum_evolved_match(&minus, &plus), Some("New".to_string()));
    }

    #[test]
    fn enum_name_only_and_vis_only_fire() {
        let minus = item_enum("enum Old {\n    A,\n    B,\n}");
        let plus = item_enum("enum New {\n    A,\n    B,\n}");
        assert_eq!(enum_evolved_match(&minus, &plus), Some("New".to_string()));
        let plus = item_enum("pub(crate) enum Old {\n    A,\n    B,\n}");
        assert_eq!(enum_evolved_match(&minus, &plus), Some("Old".to_string()));
    }

    #[test]
    fn enum_identical_and_deltas_veto() {
        let base = item_enum("enum Old {\n    A,\n    B(i32),\n}");
        let same = item_enum("enum Old {\n    A,\n    B(i32),\n}");
        assert_eq!(enum_evolved_match(&base, &same), None);
        // Variant delta.
        let plus = item_enum("pub enum New {\n    A,\n    C,\n}");
        assert_eq!(enum_evolved_match(&base, &plus), None);
        // Generic delta.
        let minus_g = item_enum("enum Old<T> {\n    A(T),\n}");
        let plus_g = item_enum("pub enum New<T: Clone> {\n    A(T),\n}");
        assert_eq!(enum_evolved_match(&minus_g, &plus_g), None);
        // Attr delta.
        let plus_a = item_enum("#[derive(Clone)]\npub enum New {\n    A,\n    B(i32),\n}");
        assert_eq!(enum_evolved_match(&base, &plus_a), None);
    }

    #[test]
    fn enum_template_shapes() {
        assert_eq!(
            enum_evolution_template("pub ", "New"),
            "pub enum New { ... }"
        );
        assert_eq!(enum_evolution_template("", "New"), "enum New { ... }");
    }

    fn item_union(text: &str) -> syn::ItemUnion {
        syn::parse_str(text).expect("test union must parse")
    }

    #[test]
    fn union_vis_and_name_evolve_fires() {
        let minus = item_union("union Old {\n    x: i32,\n}");
        let plus = item_union("pub union New {\n    x: i32,\n}");
        assert_eq!(union_evolved_match(&minus, &plus), Some("New".to_string()));
    }

    #[test]
    fn union_name_only_and_vis_only_fire() {
        let minus = item_union("union Old {\n    x: i32,\n}");
        let plus = item_union("union New {\n    x: i32,\n}");
        assert_eq!(union_evolved_match(&minus, &plus), Some("New".to_string()));
        let plus = item_union("pub(crate) union Old {\n    x: i32,\n}");
        assert_eq!(union_evolved_match(&minus, &plus), Some("Old".to_string()));
    }

    #[test]
    fn union_identical_and_deltas_veto() {
        let base = item_union("union Old {\n    x: i32,\n}");
        let same = item_union("union Old {\n    x: i32,\n}");
        assert_eq!(union_evolved_match(&base, &same), None);
        // Field delta.
        let plus = item_union("pub union New {\n    y: i64,\n}");
        assert_eq!(union_evolved_match(&base, &plus), None);
        // Generic delta.
        let minus_g = item_union("union Old<T> {\n    x: T,\n}");
        let plus_g = item_union("pub union New<T: Clone> {\n    x: T,\n}");
        assert_eq!(union_evolved_match(&minus_g, &plus_g), None);
        // Attr delta.
        let plus_a = item_union("#[derive(Clone)]\npub union New {\n    x: i32,\n}");
        assert_eq!(union_evolved_match(&base, &plus_a), None);
    }

    #[test]
    fn union_template_shapes() {
        assert_eq!(
            union_evolution_template("pub ", "New"),
            "pub union New { ... }"
        );
        assert_eq!(union_evolution_template("", "New"), "union New { ... }");
    }

    fn item_type(text: &str) -> syn::ItemType {
        syn::parse_str(text).expect("test type must parse")
    }

    #[test]
    fn type_vis_and_name_evolve_fires() {
        let minus = item_type("type Old = u32;");
        let plus = item_type("pub type New = u32;");
        assert_eq!(type_evolved_match(&minus, &plus), Some("New".to_string()));
    }

    #[test]
    fn type_name_only_and_vis_only_fire() {
        let minus = item_type("type Old = u32;");
        let plus = item_type("type New = u32;");
        assert_eq!(type_evolved_match(&minus, &plus), Some("New".to_string()));
        let plus = item_type("pub(crate) type Old = u32;");
        assert_eq!(type_evolved_match(&minus, &plus), Some("Old".to_string()));
    }

    #[test]
    fn type_identical_and_deltas_veto() {
        let base = item_type("type Old = u32;");
        let same = item_type("type Old = u32;");
        assert_eq!(type_evolved_match(&base, &same), None);
        // Aliased-type delta.
        let plus = item_type("pub type New = i64;");
        assert_eq!(type_evolved_match(&base, &plus), None);
        // Generic delta.
        let minus_g = item_type("type Old<T> = Vec<T>;");
        let plus_g = item_type("pub type New<T: Clone> = Vec<T>;");
        assert_eq!(type_evolved_match(&minus_g, &plus_g), None);
        // Attr delta.
        let plus_a = item_type("#[allow(dead_code)]\npub type New = u32;");
        assert_eq!(type_evolved_match(&base, &plus_a), None);
    }

    #[test]
    fn type_template_shapes() {
        assert_eq!(
            type_evolution_template("pub ", "New"),
            "pub type New = ...;"
        );
        assert_eq!(type_evolution_template("", "New"), "type New = ...;");
    }

    fn item_const(text: &str) -> syn::ItemConst {
        syn::parse_str(text).expect("test const must parse")
    }

    #[test]
    fn const_vis_and_name_evolve_fires() {
        let minus = item_const("const Old: u32 = 1;");
        let plus = item_const("pub const New: u32 = 1;");
        assert_eq!(const_evolved_match(&minus, &plus), Some("New".to_string()));
    }

    #[test]
    fn const_name_only_and_vis_only_fire() {
        let minus = item_const("const Old: u32 = 1;");
        let plus = item_const("const New: u32 = 1;");
        assert_eq!(const_evolved_match(&minus, &plus), Some("New".to_string()));
        let plus = item_const("pub(crate) const Old: u32 = 1;");
        assert_eq!(const_evolved_match(&minus, &plus), Some("Old".to_string()));
    }

    #[test]
    fn const_identical_and_deltas_veto() {
        let base = item_const("const Old: u32 = 1;");
        let same = item_const("const Old: u32 = 1;");
        assert_eq!(const_evolved_match(&base, &same), None);
        // Type delta.
        let plus = item_const("pub const New: i64 = 1;");
        assert_eq!(const_evolved_match(&base, &plus), None);
        // Expr delta.
        let plus = item_const("pub const New: u32 = 2;");
        assert_eq!(const_evolved_match(&base, &plus), None);
        // Attr delta.
        let plus_a = item_const("#[allow(dead_code)]\npub const New: u32 = 1;");
        assert_eq!(const_evolved_match(&base, &plus_a), None);
    }

    #[test]
    fn const_template_shapes() {
        assert_eq!(
            const_evolution_template("pub ", "New"),
            "pub const New = ...;"
        );
        assert_eq!(const_evolution_template("", "New"), "const New = ...;");
    }

    fn item_static(text: &str) -> syn::ItemStatic {
        syn::parse_str(text).expect("test static must parse")
    }

    #[test]
    fn static_vis_and_name_evolve_fires() {
        let minus = item_static("static Old: u32 = 1;");
        let plus = item_static("pub static New: u32 = 1;");
        assert_eq!(static_evolved_match(&minus, &plus), Some("New".to_string()));
    }

    #[test]
    fn static_name_only_and_vis_only_fire() {
        let minus = item_static("static Old: u32 = 1;");
        let plus = item_static("static New: u32 = 1;");
        assert_eq!(static_evolved_match(&minus, &plus), Some("New".to_string()));
        let plus = item_static("pub(crate) static Old: u32 = 1;");
        assert_eq!(static_evolved_match(&minus, &plus), Some("Old".to_string()));
    }

    #[test]
    fn static_mut_delta_rides_verbatim() {
        // `static` -> `static mut` is an evolution, not a veto (the
        // template carries the plus-side `mut` verbatim, like `fn`
        // qualifiers ride the prefix).
        let minus = item_static("static Old: u32 = 1;");
        let plus = item_static("pub static mut New: u32 = 1;");
        assert_eq!(static_evolved_match(&minus, &plus), Some("New".to_string()));
        let minus_m = item_static("static mut Old: u32 = 1;");
        let plus_m = item_static("static Old: u32 = 1;");
        assert_eq!(
            static_evolved_match(&minus_m, &plus_m),
            Some("Old".to_string())
        );
    }

    #[test]
    fn static_identical_and_deltas_veto() {
        let base = item_static("static Old: u32 = 1;");
        let same = item_static("static Old: u32 = 1;");
        assert_eq!(static_evolved_match(&base, &same), None);
        // Type delta.
        let plus = item_static("pub static New: i64 = 1;");
        assert_eq!(static_evolved_match(&base, &plus), None);
        // Expr delta.
        let plus = item_static("pub static New: u32 = 2;");
        assert_eq!(static_evolved_match(&base, &plus), None);
        // Attr delta.
        let plus_a = item_static("#[allow(dead_code)]\npub static New: u32 = 1;");
        assert_eq!(static_evolved_match(&base, &plus_a), None);
    }

    #[test]
    fn static_template_shapes() {
        assert_eq!(
            static_evolution_template("pub ", "New", false),
            "pub static New = ...;"
        );
        assert_eq!(
            static_evolution_template("", "New", false),
            "static New = ...;"
        );
        assert_eq!(
            static_evolution_template("pub ", "New", true),
            "pub static mut New = ...;"
        );
        assert_eq!(
            static_evolution_template("", "New", true),
            "static mut New = ...;"
        );
    }

    #[test]
    fn static_header_prefix_skips_mut_and_attrs() {
        let lines = vec![
            "+#[allow(dead_code)]".to_string(),
            "+pub static mut helper: u32 = 1;".to_string(),
        ];
        assert_eq!(
            static_header_prefix(&lines, "helper"),
            Some("pub ".to_string())
        );
    }

    fn item_mod(text: &str) -> syn::ItemMod {
        syn::parse_str(text).expect("test mod must parse")
    }

    #[test]
    fn mod_vis_and_name_evolve_fires() {
        let minus = item_mod("mod old {}");
        let plus = item_mod("pub mod new {}");
        assert_eq!(mod_evolved_match(&minus, &plus), Some("new".to_string()));
    }

    #[test]
    fn mod_name_only_and_vis_only_fire() {
        let minus = item_mod("mod old {}");
        let plus = item_mod("mod new {}");
        assert_eq!(mod_evolved_match(&minus, &plus), Some("new".to_string()));
        let plus = item_mod("pub(crate) mod old {}");
        assert_eq!(mod_evolved_match(&minus, &plus), Some("old".to_string()));
    }

    #[test]
    fn mod_decl_forms_fire() {
        let minus = item_mod("mod old;");
        let plus = item_mod("pub mod new;");
        assert_eq!(mod_evolved_match(&minus, &plus), Some("new".to_string()));
    }

    #[test]
    fn mod_unsafe_delta_rides_verbatim() {
        let minus = item_mod("mod old {}");
        let plus = item_mod("pub unsafe mod new {}");
        assert_eq!(mod_evolved_match(&minus, &plus), Some("new".to_string()));
        let minus_u = item_mod("unsafe mod old {}");
        let plus_u = item_mod("mod old {}");
        assert_eq!(
            mod_evolved_match(&minus_u, &plus_u),
            Some("old".to_string())
        );
    }

    #[test]
    fn mod_identical_and_deltas_veto() {
        let base = item_mod("mod old {}");
        let same = item_mod("mod old {}");
        assert_eq!(mod_evolved_match(&base, &same), None);
        // Any nested item vetoes, even identical content (the scalar
        // `- I` cut cannot isolate a mod with nested items).
        let plus = item_mod("pub mod new {\n    pub fn f() {}\n}");
        assert_eq!(mod_evolved_match(&base, &plus), None);
        let nested = item_mod("mod old {\n    pub fn f() {}\n}");
        let nested_same = item_mod("mod old {\n    pub fn f() {}\n}");
        assert_eq!(mod_evolved_match(&nested, &nested_same), None);
        let nested_plus = item_mod("pub mod new {\n    pub fn f() {}\n}");
        assert_eq!(mod_evolved_match(&nested, &nested_plus), None);
        // Semi-vs-braced delta.
        let minus_d = item_mod("mod old;");
        let plus_d = item_mod("pub mod new {\n}");
        assert_eq!(mod_evolved_match(&minus_d, &plus_d), None);
        // Attr delta.
        let plus_a = item_mod("#[allow(dead_code)]\npub mod new {}");
        assert_eq!(mod_evolved_match(&base, &plus_a), None);
    }

    #[test]
    fn mod_template_shapes() {
        assert_eq!(mod_evolution_template("pub ", "New"), "pub mod New { ... }");
        assert_eq!(mod_evolution_template("", "New"), "mod New { ... }");
        assert_eq!(
            mod_evolution_template("pub unsafe ", "New"),
            "pub unsafe mod New { ... }"
        );
    }

    fn item_trait(text: &str) -> syn::ItemTrait {
        syn::parse_str(text).expect("test trait must parse")
    }

    #[test]
    fn trait_vis_and_name_evolve_fires() {
        let minus = item_trait("trait Old {}");
        let plus = item_trait("pub trait New {}");
        assert_eq!(trait_evolved_match(&minus, &plus), Some("New".to_string()));
    }

    #[test]
    fn trait_name_only_and_vis_only_fire() {
        let minus = item_trait("trait Old {}");
        let plus = item_trait("trait New {}");
        assert_eq!(trait_evolved_match(&minus, &plus), Some("New".to_string()));
        let plus = item_trait("pub(crate) trait Old {}");
        assert_eq!(trait_evolved_match(&minus, &plus), Some("Old".to_string()));
    }

    #[test]
    fn trait_unsafe_delta_rides_verbatim() {
        let minus = item_trait("trait Old {}");
        let plus = item_trait("pub unsafe trait New {}");
        assert_eq!(trait_evolved_match(&minus, &plus), Some("New".to_string()));
        let minus_u = item_trait("unsafe trait Old {}");
        let plus_u = item_trait("trait Old {}");
        assert_eq!(
            trait_evolved_match(&minus_u, &plus_u),
            Some("Old".to_string())
        );
    }

    #[test]
    fn trait_identical_and_deltas_veto() {
        let base = item_trait("trait Old {}");
        let same = item_trait("trait Old {}");
        assert_eq!(trait_evolved_match(&base, &same), None);
        // Any trait item vetoes, even identical content (the scalar
        // `- I` cut cannot isolate a trait with methods, it would match
        // them too — same narrowing as empty-only `mod` evolution).
        let plus = item_trait("pub trait New {\n    fn f(&self);\n}");
        assert_eq!(trait_evolved_match(&base, &plus), None);
        let method = item_trait("trait Old {\n    fn f(&self);\n}");
        let method_same = item_trait("trait Old {\n    fn f(&self);\n}");
        assert_eq!(trait_evolved_match(&method, &method_same), None);
        let method_plus = item_trait("pub trait New {\n    fn f(&self);\n}");
        assert_eq!(trait_evolved_match(&method, &method_plus), None);
        // Generics delta.
        let plus = item_trait("pub trait New<T> {}");
        assert_eq!(trait_evolved_match(&base, &plus), None);
        // Supertrait delta.
        let plus = item_trait("pub trait New: Clone {}");
        assert_eq!(trait_evolved_match(&base, &plus), None);
        // Attr delta.
        let plus_a = item_trait("#[allow(dead_code)]\npub trait New {}");
        assert_eq!(trait_evolved_match(&base, &plus_a), None);
        // `auto` / restriction never evolve (mirror the authoring veto).
        let auto_minus: syn::ItemTrait =
            syn::parse_str("auto trait Old {}").expect("auto trait must parse");
        let auto_plus: syn::ItemTrait =
            syn::parse_str("pub auto trait New {}").expect("auto trait must parse");
        assert_eq!(trait_evolved_match(&auto_minus, &auto_plus), None);
        assert_eq!(trait_evolved_match(&auto_minus, &base), None);
    }

    #[test]
    fn trait_template_shapes() {
        assert_eq!(
            trait_evolution_template("pub ", "New"),
            "pub trait New { ... }"
        );
        assert_eq!(trait_evolution_template("", "New"), "trait New { ... }");
        assert_eq!(
            trait_evolution_template("pub unsafe ", "New"),
            "pub unsafe trait New { ... }"
        );
    }

    #[test]
    fn trait_header_prefix_skips_attrs_and_docs() {
        let lines = vec![
            "+#[allow(dead_code)]".to_string(),
            "+/// implements drawing".to_string(),
            "+pub trait Drawable {".to_string(),
            "+    fn draw(&self);".to_string(),
            "+}".to_string(),
        ];
        assert_eq!(
            trait_header_prefix(&lines, "Drawable"),
            Some("pub ".to_string())
        );
    }
}
