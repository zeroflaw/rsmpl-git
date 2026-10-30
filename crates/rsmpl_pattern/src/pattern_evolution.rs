//! Evolution-paste template parsing (WI-8a `fn` + WI-8 slice 1
//! `struct` + WI-8 slice 2 `enum` + WI-8 slice 3 `union` + WI-8 slice 4
//! `type` single-line headers).
//!
//! Used by the `Insertion` classifier and the engine capture splice.

/// WI-8a evolution-paste template: a single-line `fn` header with
/// `(...)` params and a `...` body (`pub fn New(...) { ... }`).
/// Returns the verbatim prefix before `fn` (vis + qualifiers) and the
/// new ident. `None` when the line is not a template shape.
pub fn parse_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `fn` followed by whitespace (never the `fn` inside
    // another ident, e.g. `myfn`).
    let mut fn_pos = None;
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..].starts_with(b"fn")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 2).is_some_and(|c| c.is_ascii_whitespace())
        {
            fn_pos = Some(i);
            break;
        }
        i += 1;
    }
    let fn_pos = fn_pos?;
    let prefix = &line[..fn_pos];
    if !valid_evolution_prefix(prefix) {
        return None;
    }
    let rest = line[fn_pos + 2..].trim_start();
    let mut ident_len = 0;
    for c in rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            ident_len += c.len_utf8();
        } else {
            break;
        }
    }
    if ident_len == 0 {
        return None;
    }
    let tail = &rest[ident_len..];
    if !tail.contains("(...)") || !tail.contains("...") {
        return None;
    }
    Some((prefix.to_string(), rest[..ident_len].to_string()))
}

/// WI-8 slice 1 struct evolution-paste template: a single-line
/// `struct` header with a `...` body marker
/// (`pub struct New { ... }`). Returns the verbatim prefix before
/// `struct` (visibility) and the new ident. The `{ ... }` tail is a
/// shape marker only — the splice keeps the captured item's suffix
/// (generics/where/fields) byte-verbatim, same as the fn `(...)`
/// abstraction. `None` when the line is not a struct-template shape.
pub fn parse_struct_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `struct` followed by whitespace.
    let mut struct_pos = None;
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..].starts_with(b"struct")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            struct_pos = Some(i);
            break;
        }
        i += 1;
    }
    let struct_pos = struct_pos?;
    let prefix = &line[..struct_pos];
    if !valid_struct_evolution_prefix(prefix) {
        return None;
    }
    let rest = line[struct_pos + 6..].trim_start();
    let mut ident_len = 0;
    for c in rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            ident_len += c.len_utf8();
        } else {
            break;
        }
    }
    if ident_len == 0 {
        return None;
    }
    let tail = &rest[ident_len..];
    if !tail.contains("...") {
        return None;
    }
    Some((prefix.to_string(), rest[..ident_len].to_string()))
}
/// The verbatim text before `struct` in a struct evolution-paste
/// template: a Rust visibility (`pub`, `pub(crate)`, `pub(in path)`,
/// or empty for a private struct). Validated by parsing, not by
/// character vetoes.
fn valid_struct_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemStruct>(&format!("{pre} struct __RsmplDummy__ {{}}")).is_ok()
}

/// WI-8 slice 2 enum evolution-paste template: a single-line
/// `enum` header with a `...` body marker
/// (`pub enum New { ... }`). Returns the verbatim prefix before
/// `enum` (visibility) and the new ident. The `{ ... }` tail is a
/// shape marker only — the splice keeps the captured item's suffix
/// (generics/where/variants) byte-verbatim, same as the `struct`
/// abstraction. `None` when the line is not an enum-template shape.
pub fn parse_enum_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `enum` followed by whitespace.
    let mut enum_pos = None;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"enum")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            enum_pos = Some(i);
            break;
        }
        i += 1;
    }
    let enum_pos = enum_pos?;
    let prefix = &line[..enum_pos];
    if !valid_enum_evolution_prefix(prefix) {
        return None;
    }
    let rest = line[enum_pos + 4..].trim_start();
    let mut ident_len = 0;
    for c in rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            ident_len += c.len_utf8();
        } else {
            break;
        }
    }
    if ident_len == 0 {
        return None;
    }
    let tail = &rest[ident_len..];
    if !tail.contains("...") {
        return None;
    }
    Some((prefix.to_string(), rest[..ident_len].to_string()))
}
/// The verbatim text before `enum` in an enum evolution-paste
/// template: a Rust visibility (`pub`, `pub(crate)`, `pub(in path)`,
/// or empty for a private enum). Validated by parsing, not by
/// character vetoes.
fn valid_enum_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemEnum>(&format!("{pre} enum __RsmplDummy__ {{}}")).is_ok()
}

/// WI-8 slice 3 union evolution-paste template: a single-line
/// `union` header with a `...` body marker
/// (`pub union New { ... }`). Returns the verbatim prefix before
/// `union` (visibility) and the new ident. The `{ ... }` tail is a
/// shape marker only — the splice keeps the captured item's suffix
/// (generics/where/fields) byte-verbatim, same as the `struct`
/// abstraction. `None` when the line is not a union-template shape.
pub fn parse_union_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `union` followed by whitespace.
    let mut union_pos = None;
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"union")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            union_pos = Some(i);
            break;
        }
        i += 1;
    }
    let union_pos = union_pos?;
    let prefix = &line[..union_pos];
    if !valid_union_evolution_prefix(prefix) {
        return None;
    }
    let rest = line[union_pos + 5..].trim_start();
    let mut ident_len = 0;
    for c in rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            ident_len += c.len_utf8();
        } else {
            break;
        }
    }
    if ident_len == 0 {
        return None;
    }
    let tail = &rest[ident_len..];
    if !tail.contains("...") {
        return None;
    }
    Some((prefix.to_string(), rest[..ident_len].to_string()))
}
/// The verbatim text before `union` in a union evolution-paste
/// template: a Rust visibility (`pub`, `pub(crate)`, `pub(in path)`,
/// or empty for a private union). Validated by parsing, not by
/// character vetoes.
fn valid_union_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemUnion>(&format!("{pre} union __RsmplDummy__ {{ x: i32 }}")).is_ok()
}

/// WI-8 slice 4 type-alias evolution-paste template: a single-line
/// `type` header with an `= ...;` body marker
/// (`pub type New = ...;`). Returns the verbatim prefix before
/// `type` (visibility) and the new ident. The `= ...;` tail is a
/// shape marker only — the splice keeps the captured item's suffix
/// (generics/`= ty;`) byte-verbatim, same as the `struct`
/// abstraction. `None` when the line is not a type-template shape.
pub fn parse_type_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `type` followed by whitespace.
    let mut type_pos = None;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"type")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            type_pos = Some(i);
            break;
        }
        i += 1;
    }
    let type_pos = type_pos?;
    let prefix = &line[..type_pos];
    if !valid_type_evolution_prefix(prefix) {
        return None;
    }
    let rest = line[type_pos + 4..].trim_start();
    let mut ident_len = 0;
    for c in rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            ident_len += c.len_utf8();
        } else {
            break;
        }
    }
    if ident_len == 0 {
        return None;
    }
    let tail = &rest[ident_len..];
    if !tail.contains('=') || !tail.contains("...") {
        return None;
    }
    Some((prefix.to_string(), rest[..ident_len].to_string()))
}
/// The verbatim text before `type` in a type-alias evolution-paste
/// template: a Rust visibility (`pub`, `pub(crate)`, `pub(in path)`,
/// or empty for a private alias). Validated by parsing, not by
/// character vetoes.
fn valid_type_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemType>(&format!("{pre} type __RsmplDummy__ = i32;")).is_ok()
}

/// The verbatim text before `fn` in an evolution-paste template: a Rust
/// visibility + qualifier prefix (`pub`, `pub(crate)`, `pub(in path)`,
/// `const`/`async`/`unsafe`/`extern "C"` combos, or empty for a bare
/// `fn`). Validated by parsing, not by character vetoes, so restricted
/// `fn`). Validated by parsing, not by character vetoes, so restricted
/// visibilities (which contain parens) are accepted while garbage
/// (`foo(`, `a;b`, `"x"`) still fails to parse as a header prefix.
fn valid_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    // Fast veto: these bytes never appear in a vis/qualifier prefix.
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemFn>(&format!("{pre} fn __rsmpl_dummy__() {{}}")).is_ok()
}
