//! `use`-path prefix rename patterns (SPEC 14.9, T19a).
//!
//! Propagation for item renames (T12): after `mod old` -> `mod new`, every
//! `use crate::old::...` import must follow. One semantic hunk rewrites the
//! matched *prefix span* per `use` item, so plain, grouped (`::{A, B}`),
//! glob (`::*`), `as`-renamed, and `pub use` items are all fixed by a single
//! minus/plus prefix pair:
//!
//! ```diff
//! - use crate::old
//! + use crate::new
//! ```
//!
//! Matching is segment-wise on the target's leading path segments (never a
//! string prefix: minus `old` never matches target segment `old2`). The
//! pattern side is a bare `use <path>` prefix only; braces, globs, `as`,
//! `...`, `$Meta`, backticks, and declared metavariables in the pattern are
//! `PatternConflict` (write the concrete prefix; the engine preserves the
//! target's tail verbatim).

use rsmpl_patch::{ErrorCode, StructuredError};

use crate::metavar::MetavarTable;

/// A `use`-path prefix: normalized segments (leading `::` dropped).
pub type UsePrefix = Vec<String>;

/// Rust strict + reserved keywords: never a path segment in a `use` prefix
/// pattern, except the path-root keywords `crate`/`self`/`super` (and `Self`,
/// which names the parent module). Anything else keyword-shaped means the
/// text is a different shape, so reject instead of matching literally.
fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "break"
            | "const"
            | "continue"
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
            | "crate"
            | "super"
    )
}

/// Segments that may legally appear as path keywords in a `use` prefix.
fn is_path_keyword(word: &str) -> bool {
    matches!(word, "crate" | "self" | "super" | "Self")
}

fn is_ident(word: &str) -> bool {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        None | Some(_) => return false,
    }
    word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Consume ASCII whitespace and return the rest.
fn skip_ws(s: &str) -> &str {
    let n = s.len() - s.trim_start().len();
    &s[n..]
}

/// Consume one lowercase keyword when followed by a delimiter.
fn eat_keyword<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(kw)?;
    if rest.is_empty() || rest.starts_with(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        Some(skip_ws(rest))
    } else {
        None
    }
}

/// Consume `pub` / `pub(...)` visibility prefix; `public` is an ident, never
/// visibility (same rule as item renames).
fn skip_visibility(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("pub") {
        let after_ws = skip_ws(rest);
        if let Some(after) = after_ws.strip_prefix('(') {
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
        if after_ws.is_empty() || rest.starts_with(char::is_whitespace) {
            return after_ws;
        }
    }
    s
}

fn err<T>(msg: impl Into<String>) -> Result<T, StructuredError> {
    Err(StructuredError::new(ErrorCode::PatternConflict, msg.into()).with_repair(
        StructuredError::fix_pattern_repair(
            "pattern",
            "write a bare `use <path>` prefix pair (e.g. `- use crate::old` / `+ use crate::new`) or use an ordinary hunk",
        ),
    ))
}

/// Parse one `use`-prefix pattern side.
///
/// - `Ok(None)` — not a `use` shape; the caller must try the next
///   classifier. No diagnostic is produced.
/// - `Ok(Some(segments))` — valid prefix pattern.
/// - `Err(_)` — looks like a `use` prefix but is malformed (groups, globs,
///   `as`, wildcards, metavars, identical-shape problems are diagnosed at
///   the `classify` level); fail fast, never fall through.
pub fn parse_use_prefix(
    text: &str,
    table: &MetavarTable,
) -> Result<Option<UsePrefix>, StructuredError> {
    let mut s = skip_visibility(skip_ws(text));
    if s.is_empty() {
        return Ok(None);
    }
    // Mirrored attributes before a `use` shape are not supported in v1
    // (docs are preserved by default) — fail fast instead of a silent
    // no-match downstream (T156). Only a later `use`-led line triggers;
    // `#[...]`-led fn/item shapes keep their fallthrough.
    let non_empty: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.len() > 1
        && {
            let first = non_empty[0].trim_start();
            first.starts_with("#[")
                || first.starts_with("///")
                || first.starts_with("//!")
                || first.starts_with("/*")
        }
        && non_empty[1..].iter().any(|l| {
            let t = skip_visibility(skip_ws(l));
            eat_keyword(t, "use").is_some()
        })
    {
        return err(
            "mirrored attributes are not supported in `use`-path patterns in v1 (docs are preserved by default; drop the attribute lines or use an ordinary hunk)",
        );
    }
    let Some(after_use) = eat_keyword(s, "use") else {
        return Ok(None);
    };
    s = after_use;
    if s.is_empty() {
        return err("missing path after `use` (write a bare `use <path>` prefix)");
    }
    // Fail-fast triggers for shapes that need an ordinary hunk instead.
    if s.contains("...") {
        return err("v1 `use`-path patterns do not support `...` (write the concrete prefix path)");
    }
    if s.contains('$') {
        return err(
            "`$Meta` abstracted names are not supported for `use`-path renames in v1 (write concrete paths)",
        );
    }
    if s.contains('`') {
        return err(
            "backtick literal quoting is not supported in `use`-path patterns in v1 (write the concrete path)",
        );
    }
    if s.contains('{') || s.contains('}') || s.contains('*') {
        return err(
            "grouped (`::{...}`) and glob (`::*`) tails are not supported in the `use` *pattern* in v1 (write the bare shared prefix; target items may still contain groups/globs)",
        );
    }
    if s.contains('!') {
        return err(
            "`use!` is a macro invocation, not a `use`-path rename (remove the `!` or use a macro pattern)",
        );
    }
    // ` as ` rename in the pattern: the engine preserves target `as` tails,
    // so the pattern must not name one.
    let words: Vec<&str> = s
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '#')
        .collect();
    if words.contains(&"as") {
        return err(
            "`as` renames are not supported in the `use` *pattern* in v1 (write the bare prefix; target `as` tails are preserved)",
        );
    }
    // Only path characters may remain: strip one trailing `;`, then every
    // `::`-separated segment must be an identifier.
    let mut body = s.trim();
    if let Some(stripped) = body.strip_suffix(';') {
        body = stripped.trim();
    }
    if body.is_empty() {
        return err("missing path after `use` (write a bare `use <path>` prefix)");
    }
    let no_colons: String = body.chars().filter(|c| *c != ':').collect();
    if no_colons.is_empty() {
        return err(format!(
            "invalid `use` path `{body}` (expected `::`-separated identifiers)"
        ));
    }
    let mut segs = Vec::new();
    for part in body.split("::") {
        let seg = part.trim();
        if seg.is_empty() {
            // Leading `::` yields one empty first part: the global path
            // marker, normalized away. Any other empty part is `a::::b`.
            if segs.is_empty() && part.is_empty() {
                continue;
            }
            return err(format!(
                "invalid `use` path `{body}` (expected `::`-separated identifiers)"
            ));
        }
        if !is_ident(seg) || (is_keyword(seg) && !is_path_keyword(seg)) {
            return err(format!(
                "invalid `use` path segment `{seg}` (expected an identifier)"
            ));
        }
        if table.is_meta(seg) {
            return err(format!(
                "metavariable `{seg}` is not supported in `use`-path patterns in v1 (write the concrete path)"
            ));
        }
        segs.push(seg.to_string());
    }
    if segs.is_empty() {
        return err("missing path after `use` (write a bare `use <path>` prefix)");
    }
    Ok(Some(segs))
}

/// Classify a minus/plus side pair as a `use`-path prefix rename.
///
/// Returns the normalized segment lists; the engine compares segment-wise
/// and renders the plus side joined with `::`.
pub fn classify_use_rename(
    minus_raw: &str,
    plus_raw: &str,
    table: &MetavarTable,
) -> Result<Option<(UsePrefix, UsePrefix)>, StructuredError> {
    let minus = parse_use_prefix(minus_raw, table)?;
    let plus = parse_use_prefix(plus_raw, table)?;
    match (minus, plus) {
        (Some(m), Some(p)) => {
            if m == p {
                return err(format!(
                    "`use`-path rename minus and plus prefixes are identical (`{}`); write the new path on the `+` side",
                    m.join("::")
                ));
            }
            Ok(Some((m, p)))
        }
        (Some(_), None) => err(
            "`use`-path rename plus side must be a bare `use <path>` prefix matching the minus side",
        ),
        (None, Some(_)) => err(
            "`use`-path rename minus side must be a bare `use <path>` prefix matching the plus side",
        ),
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metavar::MetavarTable;

    fn table() -> MetavarTable {
        MetavarTable::from_directives(&[]).unwrap()
    }

    fn parse(text: &str) -> Vec<String> {
        parse_use_prefix(text, &table()).unwrap().unwrap()
    }

    #[test]
    fn parses_plain_prefixes() {
        assert_eq!(parse("use crate::old"), vec!["crate", "old"]);
        assert_eq!(parse("use crate::old;"), vec!["crate", "old"]);
        assert_eq!(parse("use old"), vec!["old"]);
        assert_eq!(parse("use ::crate::old"), vec!["crate", "old"]);
        assert_eq!(parse("use self::old"), vec!["self", "old"]);
        assert_eq!(parse("use super::old"), vec!["super", "old"]);
        assert_eq!(parse("use crate :: old"), vec!["crate", "old"]);
        assert_eq!(parse("pub use crate::old"), vec!["crate", "old"]);
        assert_eq!(parse("pub(crate) use crate::old"), vec!["crate", "old"]);
    }

    #[test]
    fn not_applicable_falls_through() {
        assert!(parse_use_prefix("fn foo", &table()).unwrap().is_none());
        assert!(parse_use_prefix("foo(E)", &table()).unwrap().is_none());
        assert!(parse_use_prefix("struct Old", &table()).unwrap().is_none());
        assert!(parse_use_prefix("use_foo(E)", &table()).unwrap().is_none());
        assert!(parse_use_prefix("", &table()).unwrap().is_none());
        // `public` is an identifier, not visibility: not a use shape.
        assert!(parse_use_prefix("public use crate::old", &table())
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_pattern_tails() {
        for bad in [
            "use crate::old::{A, B}",
            "use crate::old::*",
            "use crate::old::Thing as T",
            "use crate::...",
            "use crate::$M",
            "use `crate`::old",
            "use crate::::old",
            "use",
            "use;",
            "use fn",
        ] {
            assert!(
                parse_use_prefix(bad, &table()).is_err(),
                "must reject: {bad}"
            );
        }
    }

    #[test]
    fn mirrored_attributes_fail_fast_with_needle() {
        // T156: leading attr-ish lines before a `use` shape fail fast
        // instead of a silent no-match downstream; docs stay preserved
        // by default.
        for bad in [
            "/// Doc.\nuse crate::old",
            "//! Doc.\nuse crate::old",
            "/** Doc. */\nuse crate::old",
            "#[doc = \" Doc. \"]\nuse crate::old",
            "/* plain */\nuse crate::old",
            "#[inline]\npub use crate::old",
        ] {
            let err = parse_use_prefix(bad, &table()).unwrap_err();
            assert!(
                err.render().contains("mirrored attributes"),
                "must needle: {bad} got: {}",
                err.render()
            );
        }
        // Non-use shapes with leading attrs keep their fallthrough.
        assert!(parse_use_prefix("#[inline]\nfn foo", &table())
            .unwrap()
            .is_none());
        assert!(parse_use_prefix("#[inline]\npub fn foo", &table())
            .unwrap()
            .is_none());
        assert!(parse_use_prefix("/// Doc.\nstruct Old", &table())
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_declared_metavariable_segments() {
        use rsmpl_patch::{Directive, MetavarKind};
        let table = MetavarTable::from_directives(&[Directive::MetaVar {
            kind: MetavarKind::Type,
            names: vec!["T".to_string()],
        }])
        .unwrap();
        let err = parse_use_prefix("use crate::T", &table).unwrap_err();
        assert!(
            err.render().contains("metavariable"),
            "got: {}",
            err.render()
        );
    }

    #[test]
    fn identical_prefixes_rejected() {
        let t = table();
        let err = classify_use_rename("use crate::old", "use crate::old;", &t).unwrap_err();
        assert!(err.render().contains("identical"), "got: {}", err.render());
    }

    #[test]
    fn mismatched_sides_rejected() {
        let t = table();
        assert!(classify_use_rename("use crate::old", "struct New", &t).is_err());
        assert!(classify_use_rename("struct Old", "use crate::new", &t).is_err());
        assert!(classify_use_rename("foo(E)", "bar(E)", &t)
            .unwrap()
            .is_none());
    }

    #[test]
    fn classifies_valid_pair() {
        let t = table();
        let (m, p) = classify_use_rename("use crate::old", "use crate::new;", &t)
            .unwrap()
            .unwrap();
        assert_eq!(m, vec!["crate", "old"]);
        assert_eq!(p, vec!["crate", "new"]);
    }
}
