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
//! `static` (plus `mut`), `mod`, `impl` (self-ty only), `macro_rules!`.
//! An optional `...`
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
    MacroRules,
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
            Self::MacroRules => "macro_rules!",
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
            "macro_rules" | "macro_rules!" => Some(Self::MacroRules),
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

/// Split leading attribute lines (`#[...]`, plus `///`/`//!`/single-line
/// `/** */` doc lines normalized to `#[doc = "..."]` via the shared
/// helper — T155 parity for the item front-end, T156); returns
/// (attrs, rest).
fn split_attrs(text: &str) -> (Vec<String>, String) {
    let lines: Vec<&str> = text.lines().collect();
    let mut attrs = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if !lines[idx].trim_start().starts_with("#[") {
            match crate::fn_pattern::doc_comment_to_attr(lines[idx].trim()) {
                Some(attr) => {
                    attrs.push(attr);
                    idx += 1;
                    continue;
                }
                None => break,
            }
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

/// Consume `pub` / `pub(...)` visibility prefix; returns the consumed prefix
/// (un-normalized) plus the rest. Only consumes `pub` when followed by a
/// delimiter so `public` (an ident) is never mistaken for visibility.
fn split_visibility(s: &str) -> (&str, &str) {
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
                        let tail = skip_ws(&after[i + 1..]);
                        let consumed = &s[..s.len() - tail.len()];
                        return (consumed, tail);
                    }
                }
            }
            return ("", s); // unbalanced: don't consume
        }
        // Lone `pub` or `pub` + whitespace (`pub struct`): consume. `public`
        // (an ident) must not be mistaken for visibility.
        if after_ws.is_empty() || rest.starts_with(char::is_whitespace) {
            let consumed = &s[..s.len() - after_ws.len()];
            return (consumed, after_ws);
        }
    }
    ("", s)
}

/// Strip all whitespace so equivalent prefixes compare equal
/// (`pub (crate)` ≡ `pub(crate)`). Applied per prefix part; parts are
/// joined with a single space by the caller.
fn normalize_prefix(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_whitespace()).collect()
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

/// One parsed item-rename side: the item kind, the rename name, any leading
/// `#[...]` attribute lines, and the normalized `pub…` / `unsafe` / `mut`
/// prefix spelling (`""` for the bare form).
#[derive(Debug)]
pub struct ItemRenameSide {
    /// Item kind (`struct`, `enum`, …).
    pub kind: ItemKind,
    /// Rename name ident.
    pub name: String,
    /// Leading attribute lines.
    pub attrs: Vec<String>,
    /// Normalized vis/qualifier prefix (`""` when bare).
    pub prefix: String,
}

/// Parse one item-rename side.
///
/// - `Ok(None)` — not an item-rename shape; the caller must try the next
///   classifier (fn, macro, call, ...). No diagnostic is produced.
/// - `Ok(Some(_))` — valid rename side; the caller rejects minus/plus
///   prefix differences so a vis change is never silently ignored.
/// - `Err(_)` — looks like an item rename but is malformed (concrete body,
///   `$Meta`, kind problems); fail fast, never fall through.
#[allow(clippy::too_many_lines)]
pub fn parse_side(text: &str) -> Result<Option<ItemRenameSide>, StructuredError> {
    let (attrs, rest) = split_attrs(text);
    let mut s = skip_ws(&rest);
    if s.is_empty() {
        return Ok(None);
    }
    let mut prefix = String::new();
    let mut saw_vis = false;
    let mut saw_unsafe = false;
    let (vis, after_vis) = split_visibility(s);
    if !vis.is_empty() {
        saw_vis = true;
        prefix.push_str(&normalize_prefix(vis));
    }
    s = after_vis;
    // `unsafe` precedes `trait`/`impl` (`unsafe trait`, `unsafe impl`);
    // consume it when present so those shapes route here. `unsafe fn`
    // falls through below because `fn` is not an item kind.
    if let Some(after) = eat_keyword(s, "unsafe") {
        saw_unsafe = true;
        if !prefix.is_empty() {
            prefix.push(' ');
        }
        prefix.push_str("unsafe");
        s = after;
    }
    // `auto trait` headers (T23): fail fast instead of falling through to
    // the call/general routers as a confusing no-match. `auto` is a
    // reserved keyword, so no other shape starts this way; anything else
    // `auto`-led keeps today's fallthrough below.
    if let Some(after) = eat_keyword(s, "auto") {
        if eat_keyword(after, "trait").is_some() {
            return err("`auto trait` headers are not supported in v1 (use an ordinary hunk)");
        }
    }
    // Kind keyword.
    let mut kind: Option<ItemKind> = None;
    for kw in [
        "struct",
        "enum",
        "trait",
        "union",
        "type",
        "const",
        "static",
        "mod",
        "impl",
        "macro_rules",
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
    // `macro_rules!` definitions take the `!` between kind and name
    // (`macro_rules! Old`); without it the text is a macro invocation
    // shape, not an item rename — fall through to the macro router.
    if kind == ItemKind::MacroRules {
        let t = skip_ws(s);
        let Some(rest) = t.strip_prefix('!') else {
            return Ok(None);
        };
        s = skip_ws(rest);
    }
    // Prefix–kind legality (T23a review): `unsafe` is only legal on
    // `trait`/`impl` headers, and `impl` blocks take no visibility.
    // Anything else is invalid Rust — fail fast rather than rename under
    // a meaningless spelling. Kind is `Some` here, so the `unsafe fn` /
    // `const fn` fallthrough above is unaffected. `macro_rules!`
    // definitions take no visibility, `unsafe`, or `mut`.
    if saw_unsafe && kind != ItemKind::Trait && kind != ItemKind::Impl {
        return err(format!(
            "`unsafe` is only legal on `trait`/`impl` headers, not `{}` (write the bare `<{}> <Name>` form)",
            kind.as_str(),
            kind.as_str()
        ));
    }
    if saw_vis && (kind == ItemKind::Impl || kind == ItemKind::MacroRules) {
        if kind == ItemKind::Impl {
            return err("`impl` blocks take no visibility (write the bare `impl <Name>` form)");
        }
        return err(
            "`macro_rules!` definitions take no visibility (write the bare `macro_rules! <Name>` form)",
        );
    }
    // `static mut NAME`.
    if kind == ItemKind::Static {
        if let Some(after) = eat_keyword(s, "mut") {
            if !prefix.is_empty() {
                prefix.push(' ');
            }
            prefix.push_str("mut");
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
    Ok(Some(ItemRenameSide {
        kind,
        name,
        attrs,
        prefix,
    }))
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
    Err(StructuredError::new(ErrorCode::PatternConflict, msg.into()).with_repair(
        StructuredError::fix_pattern_repair(
            "pattern",
            "write the bare `<kind> <Name>` form (e.g. `- struct Old` / `+ struct New`) or use an ordinary hunk",
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (ItemKind, String) {
        let side = parse_side(text).unwrap().unwrap();
        (side.kind, side.name)
    }

    fn prefix_of(text: &str) -> String {
        parse_side(text).unwrap().unwrap().prefix
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
        assert_eq!(parse("macro_rules! Old").0, ItemKind::MacroRules);
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
    fn prefixes_captured_and_normalized() {
        assert_eq!(prefix_of("struct Old"), "");
        assert_eq!(prefix_of("pub struct Old"), "pub");
        assert_eq!(prefix_of("pub(crate) struct Old"), "pub(crate)");
        assert_eq!(prefix_of("pub (crate) struct Old"), "pub(crate)");
        assert_eq!(prefix_of("static mut Old"), "mut");
        assert_eq!(prefix_of("pub static mut Old"), "pub mut");
        assert_eq!(prefix_of("unsafe trait Old"), "unsafe");
        assert_eq!(prefix_of("pub unsafe trait Old"), "pub unsafe");
    }
    #[test]
    fn rejects_kind_illegal_prefixes() {
        // `unsafe` is only legal on `trait`/`impl`; `impl` takes no vis.
        assert!(parse_side("unsafe struct Old").is_err());
        assert!(parse_side("unsafe static Old").is_err());
        assert!(parse_side("pub impl Old").is_err());
        // Legal combos still parse; fn fallthrough unaffected.
        assert!(parse_side("unsafe trait Old").unwrap().is_some());
        assert!(parse_side("unsafe impl Old").unwrap().is_some());
        assert!(parse_side("unsafe fn foo").unwrap().is_none());
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

    #[test]
    fn peels_doc_comment_lines_like_other_front_ends() {
        // T156: item parity with the T155 fn/general normalization —
        // `///`, `//!`, and single-line `/** */` peel to `#[doc]`.
        let side = parse_side("/// Doc.\nstruct Old").unwrap().unwrap();
        assert_eq!(side.attrs, vec!["#[doc = \" Doc.\"]"]);
        let side = parse_side("//! Inner.\nstruct Old").unwrap().unwrap();
        assert_eq!(side.attrs, vec!["#[doc = \" Inner.\"]"]);
        let side = parse_side("/** Block doc. */\nstruct Old")
            .unwrap()
            .unwrap();
        assert_eq!(side.attrs, vec!["#[doc = \" Block doc. \"]"]);
        // Mixed `#[...]` + doc lines peel in order.
        let side = parse_side("#[inline]\n/// Doc.\nstruct Old")
            .unwrap()
            .unwrap();
        assert_eq!(side.attrs, vec!["#[inline]", "#[doc = \" Doc.\"]"]);
        // Ordinary comments still stop the peel (fallthrough preserved).
        assert!(parse_side("//// Not doc.\nstruct Old").unwrap().is_none());
        assert!(parse_side("/** Unclosed.\nstruct Old").unwrap().is_none());
    }

    #[test]
    fn rejects_auto_trait_with_needle() {
        // T23: fail fast instead of falling through to a confusing no-match.
        for text in ["auto trait Old", "pub auto trait Old"] {
            let err = parse_side(text).unwrap_err().to_string();
            assert!(err.contains("`auto trait`"), "got: {err}");
        }
        // `auto` + non-`trait` keeps today's fallthrough (no valid Rust
        // shape starts that way).
        assert!(parse_side("auto foo").unwrap().is_none());
    }

    #[test]
    fn parses_macro_rules_bang_forms() {
        // WI-7b: `macro_rules!` definitions route to item rename.
        assert_eq!(parse("macro_rules! old_widget").1, "old_widget");
        assert_eq!(parse("macro_rules!old_widget").1, "old_widget");
        assert_eq!(parse("macro_rules ! old_widget").1, "old_widget");
        assert_eq!(parse("macro_rules! old_widget { ... }").1, "old_widget");
        // Missing `!` is not an item shape (falls through to macro).
        assert!(parse_side("macro_rules old_widget").unwrap().is_none());
    }

    #[test]
    fn rejects_macro_rules_prefixes() {
        // `macro_rules!` takes no visibility or `unsafe`.
        assert!(parse_side("pub macro_rules! Old").is_err());
        assert!(parse_side("unsafe macro_rules! Old").is_err());
    }
}
