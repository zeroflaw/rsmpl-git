//! Non-function item rename patterns (SPEC 14.8, T12).
//!
//! Rename-only, token-efficient shapes: the author writes a bare
//! `<kind> <Name>` line pair and the engine replaces only the name ident,
//! preserving visibility, generics, bodies, attributes and formatting.
//!
//! ```diff
//! - struct Old
//! + struct New
//! ```
//!
//! Supported kinds: `struct`, `enum`, `trait`, `union`, `type`, `const`,
//! `static` (plus `mut`), `mod`, `impl` (self-ty only). An optional `...`
//! body suffix (`;`, `{ ... }`, `( ... )`, `= ...;`, `: ... = ...;`) is
//! accepted and ignored; any other concrete text (generics, fields, types,
//! values, `where`) is a `PatternConflict` directing the author to the bare
//! form. `$Meta` abstracted names are unsupported in v1 (fn patterns already
//! cover `$Meta` for functions).

use rsmpl_patch::{ErrorCode, StructuredError};

/// Item kinds that support ident-span rename (SPEC 14.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Struct,
    Enum,
    Trait,
    Union,
    Type,
    Const,
    Static,
    Mod,
    Impl,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Union => "union",
            Self::Type => "type",
            Self::Const => "const",
            Self::Static => "static",
            Self::Mod => "mod",
            Self::Impl => "impl",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "struct" => Some(Self::Struct),
            "enum" => Some(Self::Enum),
            "trait" => Some(Self::Trait),
            "union" => Some(Self::Union),
            "type" => Some(Self::Type),
            "const" => Some(Self::Const),
            "static" => Some(Self::Static),
            "mod" => Some(Self::Mod),
            "impl" => Some(Self::Impl),
            _ => None,
        }
    }
}

/// Rust strict + reserved keywords: never a rename name. Hitting one in name
/// position means the text is a different shape (e.g. `const fn`), so the
/// caller must fall through to the other classifiers instead of erroring.
fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    )
}

/// A plain (unquoted, non-raw) Rust identifier.
fn is_ident(word: &str) -> bool {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Raw identifier (`r#type`): the `r#` prefix plus a plain identifier.
fn is_raw_ident(word: &str) -> bool {
    word.strip_prefix("r#").is_some_and(is_ident)
}

/// Split leading `#[...]` attribute lines; returns (attrs, rest).
fn split_attrs(text: &str) -> (Vec<String>, String) {
    let lines: Vec<&str> = text.lines().collect();
    let mut attrs = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if !lines[idx].trim_start().starts_with("#[") {
            break;
        }
        let mut depth = 0i32;
        let start = idx;
        while idx < lines.len() {
            for c in lines[idx].chars() {
                if c == '[' {
                    depth += 1;
                } else if c == ']' {
                    depth -= 1;
                }
            }
            idx += 1;
            if depth <= 0 {
                break;
            }
        }
        attrs.extend(lines[start..idx].iter().map(ToString::to_string));
        if depth > 0 {
            break;
        }
    }
    (attrs, lines[idx..].join("\n"))
}

/// Consume ASCII whitespace (spaces, tabs, newlines) and return the rest.
fn skip_ws(s: &str) -> &str {
    let n = s.len() - s.trim_start().len();
    &s[n..]
}

/// Consume `pub` / `pub(...)` visibility prefix; returns the rest.
/// Only consumes `pub` when followed by a delimiter so `public` (an ident)
/// is never mistaken for visibility.
fn skip_visibility(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("pub") {
        let after_ws = skip_ws(rest);
        if let Some(after) = after_ws.strip_prefix('(') {
            // `pub(...)` (whitespace before the paren is legal): consume to
            // the matching close paren.
            let mut depth = 1i32;
            for (i, c) in after.char_indices() {
                if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        return skip_ws(&after[i + 1..]);
                    }
                }
            }
            return s; // unbalanced: don't consume
        }
        // Lone `pub` or `pub` + whitespace (`pub struct`): consume. `public`
        // (an ident) must not be mistaken for visibility.
        if after_ws.is_empty() || rest.starts_with(char::is_whitespace) {
            return after_ws;
        }
    }
    s
}

/// Consume one lowercase keyword when followed by a delimiter.
/// Returns the rest after the keyword, or `None` when absent.
fn eat_keyword<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(kw)?;
    if rest.is_empty() || rest.starts_with(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        Some(skip_ws(rest))
    } else {
        None
    }
}

/// Parse one item-rename side.
///
/// - `Ok(None)` — not an item-rename shape; the caller must try the next
///   classifier (fn, macro, call, ...). No diagnostic is produced.
/// - `Ok(Some((kind, name, attrs)))` — valid rename side.
/// - `Err(_)` — looks like an item rename but is malformed (concrete body,
///   `$Meta`, kind problems); fail fast, never fall through.
#[allow(clippy::too_many_lines)]
pub fn parse_side(text: &str) -> Result<Option<(ItemKind, String, Vec<String>)>, StructuredError> {
    let (attrs, rest) = split_attrs(text);
    let mut s = skip_ws(&rest);
    if s.is_empty() {
        return Ok(None);
    }
    s = skip_visibility(s);
    // `unsafe` precedes `trait`/`impl` (`unsafe trait`, `unsafe impl`);
    // consume it when present so those shapes route here. `unsafe fn`
    // falls through below because `fn` is not an item kind.
    if let Some(after) = eat_keyword(s, "unsafe") {
        s = after;
    }
    // Kind keyword.
    let mut kind: Option<ItemKind> = None;
    for kw in [
        "struct", "enum", "trait", "union", "type", "const", "static", "mod", "impl",
    ] {
        if let Some(after) = eat_keyword(s, kw) {
            kind = ItemKind::parse(kw);
            s = after;
            break;
        }
    }
    let Some(kind) = kind else {
        return Ok(None);
    };
    // `static mut NAME`.
    if kind == ItemKind::Static {
        if let Some(after) = eat_keyword(s, "mut") {
            s = after;
        }
    }
    // A `!` here is a macro invocation (`impl!(...)`, ...), not an item.
    if s.starts_with('!') {
        return Ok(None);
    }
    // Name: backticked literal, `$Meta` (rejected), raw ident, or ident.
    let (name, after) = if let Some(rest) = s.strip_prefix('`') {
        let Some(end) = rest.find('`') else {
            return err("unbalanced backtick quoting in item name (use `` `Name` ``)");
        };
        let inner = &rest[..end];
        if !is_ident(inner) && !is_raw_ident(inner) {
            return err(format!(
                "invalid backticked item name `` `{inner}` `` (expected one identifier)"
            ));
        }
        (inner.to_string(), skip_ws(&rest[end + 1..]))
    } else if s.starts_with('$') {
        return err(
            "`$Meta` abstracted names are not supported for item renames in v1 (write concrete names)",
        );
    } else if let Some(after) = s.strip_prefix("r#") {
        let end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map(|i| i + 2)
            .unwrap_or(s.len());
        let word = &s[..end];
        if !is_raw_ident(word) {
            return err(format!(
                "invalid item name `{word}` (expected an identifier)"
            ));
        }
        (word.to_string(), skip_ws(&s[end..]))
    } else {
        let end = s
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(s.len());
        let word = &s[..end];
        if word.is_empty() {
            return err(format!(
                "missing item name after `{}` (write a bare `<kind> <Name>` pattern)",
                kind.as_str()
            ));
        }
        // A keyword in name position means a different shape (`const fn`):
        // fall through to the fn classifier.
        if is_keyword(word) {
            return Ok(None);
        }
        (word.to_string(), skip_ws(&s[end..]))
    };
    validate_suffix(after, kind)?;
    Ok(Some((kind, name, attrs)))
}

/// The text after `<kind> <Name>` must be empty or punctuation/`...` only.
/// Anything with identifier characters is concrete (generics, fields,
/// types, values) and must be removed: the bare form preserves everything.
fn validate_suffix(after: &str, kind: ItemKind) -> Result<(), StructuredError> {
    // Strip every `...` wildcard first; what remains must be punctuation.
    let stripped = after.replace("...", "");
    let bad = stripped.chars().any(|c| {
        c.is_ascii_alphanumeric()
            || c == '_'
            || c == '$'
            || c == '`'
            || c == '#'
            || c == '\''
            || c == '"'
    });
    if bad {
        return err(format!(
            "concrete item shape after `<{}> <name>` is not supported in v1 (write the bare `<{}> <Name>` form; visibility, generics and bodies are preserved)",
            kind.as_str(),
            kind.as_str()
        ));
    }
    Ok(())
}

fn err<T>(msg: impl Into<String>) -> Result<T, StructuredError> {
    Err(StructuredError::new(ErrorCode::PatternConflict, msg.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (ItemKind, String) {
        let (k, n, _) = parse_side(text).unwrap().unwrap();
        (k, n)
    }

    #[test]
    fn parses_bare_kinds() {
        assert_eq!(parse("struct Old").0, ItemKind::Struct);
        assert_eq!(parse("enum Old").0, ItemKind::Enum);
        assert_eq!(parse("trait Old").0, ItemKind::Trait);
        assert_eq!(parse("union Old").0, ItemKind::Union);
        assert_eq!(parse("type Old").0, ItemKind::Type);
        assert_eq!(parse("const Old").0, ItemKind::Const);
        assert_eq!(parse("static Old").0, ItemKind::Static);
        assert_eq!(parse("mod old").0, ItemKind::Mod);
        assert_eq!(parse("impl Old").0, ItemKind::Impl);
    }

    #[test]
    fn parses_names_and_vis() {
        assert_eq!(parse("struct Old").1, "Old");
        assert_eq!(parse("pub struct Old").1, "Old");
        assert_eq!(parse("pub(crate) struct Old").1, "Old");
        assert_eq!(parse("pub (crate) struct Old").1, "Old");
        assert_eq!(parse("static mut Old").1, "Old");
        assert_eq!(parse("unsafe trait Old").1, "Old");
        assert_eq!(parse("mod old").1, "old");
        // `public` is an identifier, not visibility: not an item shape.
        assert!(parse_side("public struct Old").unwrap().is_none());
    }

    #[test]
    fn accepts_ellipsis_suffixes() {
        assert!(parse_side("struct Old;").unwrap().is_some());
        assert!(parse_side("struct Old { ... }").unwrap().is_some());
        assert!(parse_side("type Old = ...;").unwrap().is_some());
        assert!(parse_side("const Old: ... = ...;").unwrap().is_some());
        assert!(parse_side("mod old { ... }").unwrap().is_some());
    }

    #[test]
    fn not_applicable_falls_through() {
        assert!(parse_side("fn foo").unwrap().is_none());
        assert!(parse_side("pub fn foo").unwrap().is_none());
        assert!(parse_side("unsafe fn foo").unwrap().is_none());
        assert!(parse_side("const fn foo").unwrap().is_none());
        assert!(parse_side("foo(E)").unwrap().is_none());
        assert!(parse_side("impl!(...)").unwrap().is_none());
        assert!(parse_side("let x = E;").unwrap().is_none());
    }

    #[test]
    fn rejects_concrete_shapes() {
        assert!(parse_side("struct Old<T>").is_err());
        assert!(parse_side("struct Old { x: i32 }").is_err());
        assert!(parse_side("type Old = Foo;").is_err());
        assert!(parse_side("const Old: i32 = 1;").is_err());
    }

    #[test]
    fn rejects_meta_names() {
        assert!(parse_side("struct $S").is_err());
    }
}
