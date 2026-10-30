//! Evolution-paste template parsing (WI-8a): single-line `fn` headers.
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

/// The verbatim text before `fn` in an evolution-paste template: a Rust
/// visibility + qualifier prefix (`pub`, `pub(crate)`, `pub(in path)`,
/// `const`/`async`/`unsafe`/`extern "C"` combos, or empty for a bare
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
