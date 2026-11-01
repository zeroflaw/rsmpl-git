//! Evolution-paste template parsing (WI-8a `fn` + WI-8 slice 1
//! `struct` + WI-8 slice 2 `enum` + WI-8 slice 3 `union` + WI-8 slice 4
//! `type` + WI-8 slice 5 `const` + WI-8 slice 6 `static` + WI-8 slice 7
//! `mod` + WI-8 slice 8 `trait` single-line headers).
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
    // WI-6 slice 3b: the `fn <old> as <new>(...) { ... }` body-evolving
    // form belongs to `parse_fn_body_evolution_template`, never here —
    // without this guard the abstract parser would mis-name it `old`.
    if follows_as_rename(tail) {
        return None;
    }
    Some((prefix.to_string(), rest[..ident_len].to_string()))
}

/// WI-6 slice 3b: true when `tail` (text after the `fn <ident>`) opens a
/// word-bounded `as <ident>` rename (`fn fact as factorial(...)`).
/// Single-word helper shared by the abstract-form guard above.
fn follows_as_rename(tail: &str) -> bool {
    let bytes = tail.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if tail.get(i..i + 2) != Some("as") {
        return false;
    }
    let after = i + 2;
    if after < bytes.len() && is_ident(bytes[after]) {
        return false;
    }
    if i > 0 && is_ident(bytes[i - 1]) {
        return false;
    }
    let mut j = after;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    j < bytes.len() && is_ident(bytes[j])
}

/// WI-6 slice 3b body-evolving `fn` evolution-paste template:
/// `fn <old> as <new>(...) { ... }` (prefix rides verbatim:
/// `pub fn fact as factorial(...) { ... }`). Returns
/// `(prefix, old, new)`. The engine splices the header to the new name
/// AND renames word-bounded `old` → `new` in the captured body through
/// [`rename_ident_in_code`]; the abstract `(...)` form without `as` keeps
/// its verbatim-body splice. `None` for every other shape (the concrete
/// signature form never carries `as`).
pub fn parse_fn_body_evolution_template(text: &str) -> Option<(String, String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
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
    let mut old_len = 0;
    for c in rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            old_len += c.len_utf8();
        } else {
            break;
        }
    }
    if old_len == 0 {
        return None;
    }
    let old = &rest[..old_len];
    let after_old = &rest[old_len..];
    // Word-bounded `as` then the new ident.
    let mut j = 0;
    while after_old
        .as_bytes()
        .get(j)
        .is_some_and(|c| c.is_ascii_whitespace())
    {
        j += 1;
    }
    if after_old.get(j..j + 2) != Some("as") {
        return None;
    }
    let k = j + 2;
    if after_old.as_bytes().get(k).is_some_and(|c| is_ident(*c)) {
        return None;
    }
    if j > 0 && after_old.as_bytes()[j - 1].is_ascii_alphanumeric() {
        return None;
    }
    let after_as = &after_old[k..];
    let mut s = 0;
    while after_as
        .as_bytes()
        .get(s)
        .is_some_and(|c| c.is_ascii_whitespace())
    {
        s += 1;
    }
    let new_rest = &after_as[s..];
    let mut new_len = 0;
    for c in new_rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            new_len += c.len_utf8();
        } else {
            break;
        }
    }
    if new_len == 0 || old == &new_rest[..new_len] {
        return None;
    }
    let tail = &new_rest[new_len..];
    if !tail.contains("(...)") || !tail.contains("...") {
        return None;
    }
    if tail.contains(" as ") || follows_as_rename(tail) {
        return None;
    }
    Some((
        prefix.to_string(),
        old.to_string(),
        new_rest[..new_len].to_string(),
    ))
}

/// WI-6 slice 3b: rename word-bounded `old` → `new` in code text, leaving
/// string literals, char literals, line comments, and (nested) block
/// comments byte-verbatim. Pairing (`move_evolve`) and splicing
/// (`capture`) share this scanner, so they agree by construction: a shape
/// the author did not rename inside a string vetoes at pairing instead of
/// mis-firing. Limits (conservative = ordinary, never a guess): byte
/// strings ride on the `"` arm, raw strings (`r"…"`, `r#"…"#`, `br"…"`
/// via the `r` arm) skip to their closing delimiter; lifetimes (`'a`)
/// stay in-code and rename consistently (a signature holding a bare
/// `old` lifetime vetoes upstream at pairing).
pub fn rename_ident_in_code(text: &str, old: &str, new: &str) -> String {
    if old.is_empty() || old == new || !text.contains(old) {
        return text.to_string();
    }
    // Char-window scan (byte slicing would split multi-byte text; every
    // advance below is whole `char`s, so unicode comments/strings/literals
    // ride byte-verbatim).
    let cs: Vec<char> = text.chars().collect();
    let old_cs: Vec<char> = old.chars().collect();
    let is_ident = |c: char| c == '_' || c.is_ascii_alphanumeric();
    let at = |pos: usize, s: &str| -> bool {
        let v: Vec<char> = s.chars().collect();
        pos + v.len() <= cs.len() && cs[pos..pos + v.len()] == v[..]
    };
    let word_old_at = |pos: usize| -> bool {
        pos + old_cs.len() <= cs.len()
            && cs[pos..pos + old_cs.len()] == old_cs[..]
            && (pos == 0 || !is_ident(cs[pos - 1]))
            && cs.get(pos + old_cs.len()).is_none_or(|c| !is_ident(*c))
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut block_depth = 0usize;
    while i < cs.len() {
        // Inside a block comment: only nesting and the closer matter.
        if block_depth > 0 {
            if at(i, "/*") {
                block_depth += 1;
                out.push_str("/*");
                i += 2;
            } else if at(i, "*/") {
                block_depth -= 1;
                out.push_str("*/");
                i += 2;
            } else {
                out.push(cs[i]);
                i += 1;
            }
            continue;
        }
        if at(i, "//") {
            while i < cs.len() && cs[i] != '\n' {
                out.push(cs[i]);
                i += 1;
            }
            continue;
        }
        if at(i, "/*") {
            block_depth = 1;
            out.push_str("/*");
            i += 2;
            continue;
        }
        // Raw string: `r"…"`, `r#"…"#`, `br"…"` (a leading `b` falls
        // through as code; this arm catches the `r` itself).
        if cs[i] == 'r' && (i == 0 || !is_ident(cs[i - 1])) {
            let mut hashes = 0;
            let mut k = i + 1;
            while cs.get(k) == Some(&'#') {
                hashes += 1;
                k += 1;
            }
            if cs.get(k) == Some(&'"') {
                while i <= k {
                    out.push(cs[i]);
                    i += 1;
                }
                // Exact closer: `"` + exactly `hashes` × `#`.
                while i < cs.len() {
                    if cs[i] == '"' {
                        let mut h = 0;
                        while h < hashes && cs.get(i + 1 + h) == Some(&'#') {
                            h += 1;
                        }
                        if h == hashes {
                            out.push('"');
                            for _ in 0..hashes {
                                out.push('#');
                            }
                            i += 1 + hashes;
                            break;
                        }
                    }
                    out.push(cs[i]);
                    i += 1;
                }
                continue;
            }
        }
        if cs[i] == '"' {
            out.push('"');
            i += 1;
            while i < cs.len() {
                if cs[i] == '\\' && i + 1 < cs.len() {
                    out.push('\\');
                    out.push(cs[i + 1]);
                    i += 2;
                } else if cs[i] == '"' {
                    out.push('"');
                    i += 1;
                    break;
                } else {
                    out.push(cs[i]);
                    i += 1;
                }
            }
            continue;
        }
        // Char literal `'x'` / `'\n'` (lifetimes like `'a` lack the closer
        // and fall through as code, renaming consistently).
        if cs[i] == '\'' {
            let mut k = i + 1;
            if cs.get(k) == Some(&'\\') {
                k += 2;
                while k < cs.len() && cs[k] != '\'' && !cs[k].is_whitespace() {
                    k += 1;
                }
            } else if cs.get(k).is_some_and(|c| !c.is_whitespace() && *c != '\'') {
                k += 1;
            }
            if cs.get(k) == Some(&'\'') {
                while i <= k {
                    out.push(cs[i]);
                    i += 1;
                }
                continue;
            }
            out.push('\'');
            i += 1;
            continue;
        }
        if word_old_at(i) {
            out.push_str(new);
            i += old_cs.len();
            continue;
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// WI-8 slice 9 concrete-signature `fn` evolution-paste template: a
/// single-line `fn` header with a fully spelled signature and a `...`
/// body marker (`pub fn New(x: i64) -> i64 { ... }`). Returns the
/// verbatim prefix before `fn` (vis + qualifiers) and the new ident.
/// The signature between the ident and the `{ ... }` marker rides
/// verbatim from the plus side — the engine's full-header splice
/// replaces the captured header line with it, so signature-evolving
/// moves (`fn old(x: i32) -> i32` → `pub fn new(x: i64) -> i64`) apply
/// byte-identical. `None` when the line is not a concrete template:
/// the abstract `(...)` form (owned by `parse_evolution_template`),
/// marker-less lines, multi-line text, and bad prefixes all decline.
pub fn parse_fn_full_header_template(text: &str) -> Option<(String, String)> {
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
    // The abstract `(...)` marker form belongs to
    // `parse_evolution_template`, never here.
    if tail.contains("(...)") {
        return None;
    }
    // A concrete signature plus the `{ ... }` body marker: parens for
    // params, a brace opening the marker, `...` inside it, `}` closing
    // the line. Marker-less concrete lines (e.g. `pub fn n(x: i32)`)
    // decline — they stay loud downstream, as before.
    if !(tail.contains('(') && tail.contains(')') && tail.contains('{') && tail.contains("...")) {
        return None;
    }
    if !line.trim_end().ends_with('}') {
        return None;
    }
    // The header part (everything before the first `{` — signatures
    // never contain braces) must parse as an `fn` item once closed.
    // This validates the vis/qualifier prefix, generics, params,
    // return type, and `where` clause in one probe, so the engine can
    // splice it without re-validating.
    let brace = line.find('{')?;
    let header_part = line[..brace].trim_end();
    if syn::parse_str::<syn::ItemFn>(&format!("{header_part} {{}}")).is_err() {
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

/// WI-8 slice 5 const evolution-paste template: a single-line
/// `const` header with an `= ...;` body marker
/// (`pub const New = ...;`). Returns the verbatim prefix before
/// `const` (visibility) and the new ident. The `= ...;` tail is a
/// shape marker only — the splice keeps the captured item's suffix
/// (generics/`: ty = expr;`) byte-verbatim, same as the `type`
/// abstraction. `None` when the line is not a const-template shape.
pub fn parse_const_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `const` followed by whitespace.
    let mut const_pos = None;
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"const")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            const_pos = Some(i);
            break;
        }
        i += 1;
    }
    let const_pos = const_pos?;
    let prefix = &line[..const_pos];
    if !valid_const_evolution_prefix(prefix) {
        return None;
    }
    let rest = line[const_pos + 5..].trim_start();
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
/// The verbatim text before `const` in a const evolution-paste
/// template: a Rust visibility (`pub`, `pub(crate)`, `pub(in path)`,
/// or empty for a private const). Validated by parsing, not by
/// character vetoes.
fn valid_const_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemConst>(&format!("{pre} const __RsmplDummy__: i32 = 0;")).is_ok()
}

/// WI-8 slice 6 static evolution-paste template: a single-line
/// `static` header with an `= ...;` body marker
/// (`pub static mut New = ...;`). Returns the verbatim prefix before
/// `static` (visibility), the new ident, and whether the template
/// carries `mut`. The `= ...;` tail is a shape marker only — the splice
/// keeps the captured item's suffix (`: ty = expr;`) verbatim, same as
/// the `const` abstraction. `None` when the line is not a
/// static-template shape.
pub fn parse_static_evolution_template(text: &str) -> Option<(String, String, bool)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `static` followed by whitespace.
    let mut static_pos = None;
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..].starts_with(b"static")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            static_pos = Some(i);
            break;
        }
        i += 1;
    }
    let static_pos = static_pos?;
    let prefix = &line[..static_pos];
    if !valid_static_evolution_prefix(prefix) {
        return None;
    }
    let mut rest = line[static_pos + 6..].trim_start();
    // Optional word-bounded `mut` qualifier before the ident (the
    // `static mut` form).
    let mut is_mut = false;
    if let Some(after_mut) = rest.strip_prefix("mut") {
        if let Some(c) = after_mut.chars().next() {
            if c.is_whitespace() {
                let tail = after_mut.trim_start();
                if let Some(d) = tail.chars().next() {
                    if d == '_' || d.is_ascii_alphanumeric() {
                        is_mut = true;
                        rest = tail;
                    }
                }
            }
        }
    }
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
    Some((prefix.to_string(), rest[..ident_len].to_string(), is_mut))
}
/// The verbatim text before `static` in a static evolution-paste
/// template: a Rust visibility (`pub`, `pub(crate)`, `pub(in path)`,
/// or empty for a private static). Validated by parsing, not by
/// character vetoes. The optional `mut` lives after the keyword, so
/// the bare-`static` probe validates vis-only prefixes for both forms.
fn valid_static_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemStatic>(&format!("{pre} static __RsmplDummy__: i32 = 0;")).is_ok()
}

/// WI-8 slice 7 mod evolution-paste template: a single-line
/// `mod` header with a `...` body marker
/// (`pub mod New { ... }`, `pub unsafe mod New { ... }`). Returns
/// the verbatim prefix before `mod` (visibility + optional `unsafe`)
/// and the new ident. The `{ ... }` tail is a shape marker only —
/// the splice keeps the captured suffix (`;` or `{}` — declaration or
/// empty inline)
/// verbatim, same as the `struct` abstraction. `None` when the line
/// is not a mod-template shape.
pub fn parse_mod_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `mod` followed by whitespace.
    let mut mod_pos = None;
    let mut i = 0;
    while i + 3 <= bytes.len() {
        if bytes[i..].starts_with(b"mod")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 3).is_some_and(|c| c.is_ascii_whitespace())
        {
            mod_pos = Some(i);
            break;
        }
        i += 1;
    }
    let mod_pos = mod_pos?;
    let prefix = &line[..mod_pos];
    if !valid_mod_evolution_prefix(prefix) {
        return None;
    }
    let rest = line[mod_pos + 3..].trim_start();
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
/// The verbatim text before `mod` in a mod evolution-paste
/// template: a Rust visibility with optional `unsafe` (`pub`,
/// `pub(crate)`, `pub unsafe`, or empty for a private inline mod).
/// Validated by parsing, not by character vetoes.
fn valid_mod_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemMod>(&format!("{pre} mod __RsmplDummy__ {{}}")).is_ok()
}

/// WI-8 slice 8 trait evolution-paste template: a single-line
/// `trait` header with a `...` body marker
/// (`pub trait New { ... }`, `pub unsafe trait New { ... }`). Returns
/// the verbatim prefix before `trait` (visibility + optional `unsafe`)
/// and the new ident. The `{ ... }` tail is a shape marker only —
/// the splice keeps the captured suffix
/// (generics/supertraits/`{...items}`)
/// verbatim, same as the `mod` abstraction. `None` when the line
/// is not a trait-template shape.
pub fn parse_trait_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `trait` followed by whitespace.
    let mut trait_pos = None;
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"trait")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            trait_pos = Some(i);
            break;
        }
        i += 1;
    }
    let trait_pos = trait_pos?;
    let prefix = &line[..trait_pos];
    if !valid_trait_evolution_prefix(prefix) {
        return None;
    }
    let rest = line[trait_pos + 5..].trim_start();
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
/// The verbatim text before `trait` in a trait evolution-paste
/// template: a Rust visibility with optional `unsafe` (`pub`,
/// `pub(crate)`, `pub unsafe`, or empty for a private trait).
/// Validated by parsing, not by character vetoes. An `auto` prefix
/// fails the probe (`auto trait` is rejected authoring-side), so
/// `auto`-led templates never parse as templates.
fn valid_trait_evolution_prefix(prefix: &str) -> bool {
    let pre = prefix.trim();
    if pre.is_empty() {
        return true;
    }
    if pre.contains(['{', ';']) {
        return false;
    }
    syn::parse_str::<syn::ItemTrait>(&format!("{pre} trait __RsmplDummy__ {{}}")).is_ok()
}

/// WI-8 slice 9: whether single-line `text` is any evolution-paste
/// template (the nine abstract shapes plus the concrete-signature `fn`
/// form). Shared by the classifier's Insertion gate and the
/// `unbound_plus` exemption — an evolution paste is verbatim plus-side
/// text spliced under a capture, never a match pattern, so its idents
/// (`T`, `Vec`, ...) are never metavariable holes.
pub fn is_evolution_template(text: &str) -> bool {
    let t = text.trim();
    parse_evolution_template(t).is_some()
        || parse_fn_body_evolution_template(t).is_some()
        || parse_fn_full_header_template(t).is_some()
        || parse_struct_evolution_template(t).is_some()
        || parse_enum_evolution_template(t).is_some()
        || parse_union_evolution_template(t).is_some()
        || parse_type_evolution_template(t).is_some()
        || parse_const_evolution_template(t).is_some()
        || parse_static_evolution_template(t).is_some()
        || parse_mod_evolution_template(t).is_some()
        || parse_trait_evolution_template(t).is_some()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_evolution_template_accepts() {
        assert_eq!(
            parse_fn_body_evolution_template("fn fact as factorial(...) { ... }"),
            Some(("".to_string(), "fact".to_string(), "factorial".to_string()))
        );
        assert_eq!(
            parse_fn_body_evolution_template("pub fn fact as factorial(...) { ... }"),
            Some((
                "pub ".to_string(),
                "fact".to_string(),
                "factorial".to_string()
            ))
        );
        assert_eq!(
            parse_fn_body_evolution_template("pub(crate) async fn f as g(...) { ... }"),
            Some((
                "pub(crate) async ".to_string(),
                "f".to_string(),
                "g".to_string()
            ))
        );
    }

    #[test]
    fn body_evolution_template_rejects() {
        // Plain abstract form is not the `as` form.
        assert_eq!(
            parse_fn_body_evolution_template("fn factorial(...) { ... }"),
            None
        );
        // Same name on both sides is vacuous.
        assert_eq!(
            parse_fn_body_evolution_template("fn fact as fact(...) { ... }"),
            None
        );
        // Concrete signatures never carry `as`.
        assert_eq!(
            parse_fn_body_evolution_template("fn fact as factorial(x: i32) { ... }"),
            None
        );
        // Multi-line text is never a template.
        assert_eq!(
            parse_fn_body_evolution_template("fn fact as factorial(...) {\n ... \n}"),
            None
        );
        // Bad prefixes decline.
        assert_eq!(
            parse_fn_body_evolution_template("foo( fn fact as factorial(...) { ... }"),
            None
        );
    }

    #[test]
    fn abstract_template_rejects_as_form() {
        // Ownership: the abstract parser must not mis-name the `as` form.
        assert_eq!(
            parse_evolution_template("fn fact as factorial(...) { ... }"),
            None
        );
        assert_eq!(
            parse_evolution_template("pub fn fact as factorial(...) { ... }"),
            None
        );
        // Untouched shapes still parse.
        assert_eq!(
            parse_evolution_template("fn factorial(...) { ... }"),
            Some(("".to_string(), "factorial".to_string()))
        );
        assert!(is_evolution_template("fn fact as factorial(...) { ... }"));
    }

    #[test]
    fn rename_skips_strings_comments_chars_raw() {
        let body = "{\n    let s = \"fact(1)\";\n    // fact(2)\n    /* fact(3) */\n    let c = 'x';\n    n * fact(n - 1)\n}";
        let out = rename_ident_in_code(body, "fact", "factorial");
        assert!(out.contains("\"fact(1)\""), "string kept, got: {out}");
        assert!(out.contains("// fact(2)"), "line comment kept, got: {out}");
        assert!(
            out.contains("/* fact(3) */"),
            "block comment kept, got: {out}"
        );
        assert!(
            out.contains("n * factorial(n - 1)"),
            "call renamed, got: {out}"
        );
        assert!(
            !out.contains("n * fact(n - 1)"),
            "old call gone, got: {out}"
        );
    }

    #[test]
    fn rename_word_bounded_and_raw_strings() {
        assert_eq!(
            rename_ident_in_code("factory fact fact2", "fact", "g"),
            "factory g fact2"
        );
        let raw = "let s = r#\"fact\"#;\n    fact(1)";
        let out = rename_ident_in_code(raw, "fact", "g");
        assert!(out.contains("r#\"fact\"#"), "raw string kept, got: {out}");
        assert!(out.contains("g(1)"), "call renamed, got: {out}");
    }

    #[test]
    fn rename_nested_block_comments_and_unicode() {
        let body = "/* outer /* fact */ still fact */\n    // héllo fact\n    fact(1)";
        let out = rename_ident_in_code(body, "fact", "g");
        assert!(
            out.contains("/* outer /* fact */ still fact */"),
            "nested comment kept, got: {out}"
        );
        assert!(
            out.contains("// héllo fact"),
            "unicode comment kept, got: {out}"
        );
        assert!(out.contains("g(1)"), "call renamed, got: {out}");
    }
}
