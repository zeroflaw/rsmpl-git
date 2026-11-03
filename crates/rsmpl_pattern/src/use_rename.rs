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
//! `...`, `$Meta`, backticks, and non-symbol declared metavariables in the pattern are
//! `PatternConflict` (write the concrete prefix; the engine preserves the
//! target's tail verbatim). A leading segment naming a declared `# symbol`
//! is the verified prefix head (WI-4 slice 2b, SPEC 19.3).

use rsmpl_patch::{ErrorCode, MetavarKind, StructuredError};

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
        if table.is_meta(seg) && table.kind_of(seg) != Some(MetavarKind::Symbol) {
            return err(format!(
                "metavariable `{seg}` is not supported in `use`-path patterns in v1 (write the concrete path)"
            ));
        }
        // WI-4 slice 2b: a segment naming a declared `# symbol` is the
        // verified prefix head (leading-only; the C14 position gate owns
        // the slot rule). It passes parsing here like a concrete ident.
        segs.push(seg.to_string());
    }
    if segs.is_empty() {
        return err("missing path after `use` (write a bare `use <path>` prefix)");
    }
    Ok(Some(segs))
}

/// A `use` regroup merge: N≥2 minus `use P::Ti;` lines plus one plus
/// `use P::{M1..Mn};` line stating their grouped form (SPEC 14.9).
/// Merge-only, order-preserving, plain-`Name` members, uniform
/// visibility, no attributes — anything else vetoes with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseRegroup {
    /// Shared prefix segments (leading `::` normalized away).
    pub prefix: UsePrefix,
    /// Member idents in minus order (== plus group order).
    pub members: Vec<String>,
    /// Uniform visibility text (`""` or e.g. `"pub(crate)"`).
    pub vis: String,
    /// Verbatim plus line (trimmed), used as the replacement text.
    pub plus_line: String,
}

fn regroup_err(msg: impl Into<String>) -> StructuredError {
    StructuredError::new(ErrorCode::PatternConflict, msg.into()).with_repair(
        StructuredError::fix_pattern_repair(
            "pattern",
            "write N≥2 `use <prefix>::<Tail>;` minus lines plus one `use <prefix>::{<Tails>};` plus line (merge-only, same order, plain members, uniform visibility) or use an ordinary hunk",
        ),
    )
}

/// Split `pub` / `pub(...)` visibility off a `use` line, returning the
/// raw visibility text plus the remainder. Mirrors `skip_visibility`
/// consumption exactly (so `public` stays an ident, never visibility).
fn split_use_vis(line: &str) -> (String, &str) {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix("pub") else {
        return (String::new(), trimmed);
    };
    if rest.is_empty() || rest.starts_with([' ', '\t']) {
        return ("pub".to_string(), rest.trim_start());
    }
    if rest.starts_with('(') {
        // Byte scan is safe: only ASCII `(`/`)` change the depth.
        let bytes = rest.as_bytes();
        let mut depth = 1i32;
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'(' {
                depth += 1;
            } else if bytes[i] == b')' {
                depth -= 1;
                if depth == 0 {
                    let vis_len = trimmed.len() - rest.len() + i + 1;
                    let vis = trimmed[..vis_len].trim().to_string();
                    return (vis, rest[i + 1..].trim_start());
                }
            }
            i += 1;
        }
        return (String::new(), trimmed);
    }
    (String::new(), trimmed)
}

/// Parse one regroup minus line as (`use` tree path segments, tail).
/// `None` when the line is not a plain single-`use` item (the caller
/// falls through so existing gates needle it instead).
fn parse_regroup_minus(line: &str) -> Option<(bool, Vec<String>)> {
    let mut text = line.trim().to_string();
    if text.is_empty() {
        return None;
    }
    if !text.trim_end().ends_with(';') {
        text.push(';');
    }
    let item: syn::Item = syn::parse_str(&text).ok()?;
    let syn::Item::Use(u) = item else {
        return None;
    };
    if !u.attrs.is_empty() {
        return None;
    }
    // Plain path + single `Name` leaf only (groups/globs/`as` in a
    // minus line are not a regroup — fall through to the existing
    // pattern-side gates, which needle them).
    let mut segs: Vec<String> = Vec::new();
    let mut tree = &u.tree;
    loop {
        match tree {
            syn::UseTree::Path(p) => {
                segs.push(p.ident.to_string());
                tree = &p.tree;
            }
            syn::UseTree::Name(n) => {
                segs.push(n.ident.to_string());
                break;
            }
            _ => return None,
        }
    }
    let leading = u.leading_colon.is_some();
    // `quote`-free ident check: `syn` already proved identifier
    // shape; reject keyword tails the prefix classifier also rejects
    // (except the path-root keywords, same allowlist).
    for s in &segs {
        if is_keyword(s) && !is_path_keyword(s) {
            return None;
        }
    }
    Some((leading, segs))
}

/// Parsed regroup plus side: leading-`::` flag, shared prefix
/// segments, grouped member idents.
type RegroupPlus = (bool, Vec<String>, Vec<String>);
/// Parse the regroup plus line as (prefix segments, member idents).
/// `Err` when the line is `use`-led but carries an unstatable merge
/// tail (`as`/glob/nested groups); `None` when it is not `use`-led or
/// not a single grouped `use` (the caller falls through).
fn parse_regroup_plus(line: &str) -> Result<Option<RegroupPlus>, StructuredError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    // Not `use`-led (word boundary): not a regroup shape at all.
    let after_use = match eat_keyword(skip_ws(trimmed), "use") {
        Some(rest) => rest,
        None => return Ok(None),
    };
    if after_use.is_empty() {
        return Err(regroup_err(
            "missing path after `use` (write a bare `use <path>` prefix)",
        ));
    }
    let mut text = trimmed.to_string();
    if !text.trim_end().ends_with(';') {
        text.push(';');
    }
    let item: syn::Item = syn::parse_str(&text).map_err(|_| {
        regroup_err("invalid regroup plus line (write `use <prefix>::{<members>};`)")
    })?;
    let syn::Item::Use(u) = item else {
        return Ok(None);
    };
    if !u.attrs.is_empty() {
        return Err(regroup_err(
            "mirrored attributes are not supported in `use`-path patterns in v1 (docs are preserved by default; drop the attribute lines or use an ordinary hunk)",
        ));
    }
    let leading = u.leading_colon.is_some();
    // Walk the path spine to the single trailing group.
    let mut prefix: Vec<String> = Vec::new();
    let mut tree = &u.tree;
    loop {
        match tree {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                tree = &p.tree;
            }
            syn::UseTree::Group(g) => {
                let mut members = Vec::new();
                for m in &g.items {
                    match m {
                        syn::UseTree::Name(n) => members.push(n.ident.to_string()),
                        syn::UseTree::Rename(r) => {
                            return Err(regroup_err(format!(
                                "`as` renames are not supported in `use`-regroup members in v1 (member `{}` keeps its alias only via an ordinary hunk)",
                                r.ident
                            )));
                        }
                        syn::UseTree::Glob(_) => {
                            return Err(regroup_err(
                                "glob (`*`) members are not supported in `use`-regroup members in v1 (state them via an ordinary hunk)",
                            ));
                        }
                        _ => {
                            return Err(regroup_err(
                                "nested groups are not supported in `use`-regroup members in v1 (merge only flat plain members, or use an ordinary hunk)",
                            ));
                        }
                    }
                }
                if members.is_empty() {
                    return Err(regroup_err(
                        "empty `use`-regroup group (write at least one member, or use an ordinary hunk)",
                    ));
                }
                for s in prefix.iter().chain(members.iter()) {
                    if is_keyword(s) && !is_path_keyword(s) {
                        return Err(regroup_err(format!(
                            "invalid `use` path segment `{s}` (expected an identifier)"
                        )));
                    }
                }
                return Ok(Some((leading, prefix, members)));
            }
            _ => return Ok(None),
        }
    }
}

/// Classify a minus/plus side pair as a `use`-regroup merge.
///
/// - `Ok(None)` — not a regroup shape; the caller falls through to
///   the next classifier (existing gates needle malformed `use`
///   spellings from there).
/// - `Ok(Some(regroup))` — valid merge.
/// - `Err(_)` — `use`-led on both sides but the merge law fails
///   (order/tails/visibility/attrs/duplicates); fail fast, never
///   guess.
pub fn classify_use_regroup(
    minus_raw: &str,
    plus_raw: &str,
    _table: &MetavarTable,
) -> Result<Option<UseRegroup>, StructuredError> {
    let minus_lines: Vec<&str> = minus_raw.lines().filter(|l| !l.trim().is_empty()).collect();
    let plus_lines: Vec<&str> = plus_raw.lines().filter(|l| !l.trim().is_empty()).collect();
    // Both sides must be `use`-led (word boundary) for a regroup to
    // even start; otherwise fall through untouched. Attribute-led
    // lines are allowed through this gate (they fail fast at the
    // mirrored-attributes needle below instead of falling through to
    // an unrelated classifier).
    let use_led = |l: &str| eat_keyword(split_use_vis(l).1, "use").is_some();
    let attr_led = |l: &str| {
        let t = l.trim_start();
        t.starts_with("#[") || t.starts_with("///") || t.starts_with("//!") || t.starts_with("/*")
    };
    if !minus_lines.iter().all(|l| use_led(l)) {
        return Ok(None);
    }
    if !plus_lines.iter().all(|l| use_led(l) || attr_led(l)) {
        return Ok(None);
    }
    // Mirrored attributes fail fast with the shared needle (checked
    // before the count gate so an attr-led regroup still needles
    // instead of silently falling through).
    for l in minus_lines.iter().chain(plus_lines.iter()) {
        if attr_led(l) {
            return Err(regroup_err(
                "mirrored attributes are not supported in `use`-path patterns in v1 (docs are preserved by default; drop the attribute lines or use an ordinary hunk)",
            ));
        }
    }
    if minus_lines.len() < 2 || plus_lines.len() != 1 {
        return Ok(None);
    }
    // Uniform visibility across minus lines and the plus line: a merge
    // must never change visibility.
    let mut vis: Option<String> = None;
    for l in minus_lines.iter().chain(plus_lines.iter()) {
        let (v, _) = split_use_vis(l);
        if vis.as_ref().is_some_and(|x| *x != v) {
            return Err(regroup_err(
                "mixed visibility in `use`-regroup (write all minus lines and the plus line with the same visibility, or use an ordinary hunk)",
            ));
        }
        vis = Some(v);
    }
    let vis = vis.unwrap_or_default();
    // Minus lines parse as plain path + single tail each. A minus line
    // that is not a plain single item is not a regroup shape — fall
    // through so the existing `use` gates needle it.
    let mut minus_paths: Vec<(bool, Vec<String>)> = Vec::new();
    for l in &minus_lines {
        let (_, rest) = split_use_vis(l);
        let Some(parsed) = parse_regroup_minus(rest) else {
            return Ok(None);
        };
        minus_paths.push(parsed);
    }
    // Uniform leading `::` (normalized away for the prefix math).
    let leading = minus_paths[0].0;
    if minus_paths.iter().any(|(l, _)| *l != leading) {
        return Err(regroup_err(
            "mixed leading `::` in `use`-regroup minus lines (write them uniformly, or use an ordinary hunk)",
        ));
    }
    let Some((plus_leading, plus_prefix, plus_members)) =
        parse_regroup_plus(split_use_vis(plus_lines[0]).1)?
    else {
        return Ok(None);
    };
    if plus_leading != leading {
        return Err(regroup_err(
            "mixed leading `::` in `use`-regroup (write minus and plus lines uniformly, or use an ordinary hunk)",
        ));
    }
    // Longest-common-prefix law: every minus path is exactly the shared
    // prefix plus one tail ident.
    let mut prefix = minus_paths[0].1.clone();
    prefix.pop();
    for (_, p) in minus_paths.iter() {
        let n = prefix
            .iter()
            .zip(p.iter())
            .take_while(|(a, b)| a == b)
            .count();
        prefix.truncate(n);
    }
    if prefix.is_empty() {
        return Err(regroup_err(
            "no shared `use`-regroup prefix (minus lines must share a leading path, or use an ordinary hunk)",
        ));
    }
    let mut tails: Vec<String> = Vec::new();
    for (_, p) in &minus_paths {
        if p.len() != prefix.len() + 1 {
            return Err(regroup_err(
                "unstatable `use`-regroup minus shape (every minus line must be `<prefix>::<Tail>` at the grouping depth, or use an ordinary hunk)",
            ));
        }
        tails.push(p[prefix.len()].clone());
    }
    // Duplicate tails can never merge honestly.
    {
        let mut sorted = tails.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != tails.len() {
            return Err(regroup_err(
                "duplicate tails in `use`-regroup minus lines (each tail must appear once, or use an ordinary hunk)",
            ));
        }
    }
    if tails.len() == 1 {
        return Err(regroup_err(
            "single-line `use`-regroup (a merge needs N≥2 minus lines, or use an ordinary hunk)",
        ));
    }
    if plus_prefix != prefix {
        return Err(regroup_err(
            "plus prefix is not the minus longest common prefix in `use`-regroup (group exactly at the shared prefix, or use an ordinary hunk)",
        ));
    }
    // Order-preserving: the group must list the tails in minus order —
    // the tool never manufactures order.
    if plus_members != tails {
        return Err(regroup_err(
            "plus group order differs from minus line order in `use`-regroup (list members in minus order, or use an ordinary hunk)",
        ));
    }
    Ok(Some(UseRegroup {
        prefix,
        members: tails,
        vis,
        plus_line: plus_lines[0].trim().to_string(),
    }))
}
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

    /// WI-4 slice 2b: a segment naming a declared `# symbol` parses as
    /// the verified prefix head (leading or whole); the C14 position
    /// gate owns the slot rule, so suffix symbols parse here too.
    #[test]
    fn symbol_head_segments_parse() {
        use rsmpl_patch::Directive;
        let table = MetavarTable::from_directives(&[
            Directive::Symbol {
                name: "Old".to_string(),
                path: "crate::old".to_string(),
            },
            Directive::Symbol {
                name: "New".to_string(),
                path: "crate::new".to_string(),
            },
        ])
        .unwrap();
        let (m, p) = classify_use_rename("use Old", "use New", &table)
            .unwrap()
            .unwrap();
        assert_eq!(m, vec!["Old"]);
        assert_eq!(p, vec!["New"]);
        let (m, p) = classify_use_rename("use Old::sub", "use New::sub", &table)
            .unwrap()
            .unwrap();
        assert_eq!(m, vec!["Old", "sub"]);
        assert_eq!(p, vec!["New", "sub"]);
        // Suffix symbols parse (C14 rejects them with the slot named).
        let (m, _) = classify_use_rename("use crate::Old", "use crate::New", &table)
            .unwrap()
            .unwrap();
        assert_eq!(m, vec!["crate", "Old"]);
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

    fn regroup(minus: &str, plus: &str) -> Result<Option<UseRegroup>, StructuredError> {
        classify_use_regroup(minus, plus, &table())
    }

    #[test]
    fn regroup_fires_plain() {
        let r = regroup("use a::b::X;\nuse a::b::Y;", "use a::b::{X, Y};")
            .unwrap()
            .expect("must fire");
        assert_eq!(r.prefix, vec!["a", "b"]);
        assert_eq!(r.members, vec!["X", "Y"]);
        assert_eq!(r.vis, "");
        assert_eq!(r.plus_line, "use a::b::{X, Y};");
    }

    #[test]
    fn regroup_fires_pub_and_deep_prefix() {
        let r = regroup(
            "pub(crate) use a::b::c::X;\npub(crate) use a::b::c::Y;",
            "pub(crate) use a::b::c::{X, Y};",
        )
        .unwrap()
        .expect("must fire");
        assert_eq!(r.prefix, vec!["a", "b", "c"]);
        assert_eq!(r.vis, "pub(crate)");
    }

    #[test]
    fn regroup_fallthrough_shapes() {
        // Single minus: meaningless merge, never a regroup.
        assert!(regroup("use a::b::X;", "use a::b::{X};").unwrap().is_none());
        // Non-`use` minus lines: not a regroup, fall through untouched.
        assert!(regroup("use a::b::X;\nfoo(E)", "use a::b::{X};")
            .unwrap()
            .is_none());
        // Bare plus: not a regroup (the rename gate owns it).
        assert!(regroup("use a::b::X;\nuse a::b::Y;", "use a::b::New;")
            .unwrap()
            .is_none());
        // Grouped minus: not a plain merge, fall through to the
        // existing pattern-side gates.
        assert!(regroup("use a::b::{X};\nuse a::b::Y;", "use a::b::{X, Y};")
            .unwrap()
            .is_none());
    }

    #[test]
    fn regroup_veto_matrix() {
        // Order swap: the group must list tails in minus order.
        assert!(regroup("use a::b::X;\nuse a::b::Y;", "use a::b::{Y, X};").is_err());
        // `as` members are unstatable tails.
        assert!(regroup("use a::b::X;\nuse a::b::Y;", "use a::b::{X, Y as Z};").is_err());
        // Glob members are unstatable tails.
        assert!(regroup("use a::b::X;\nuse a::b::Y;", "use a::b::{X, *};").is_err());
        // Nested groups never merge.
        assert!(regroup("use a::b::X;\nuse a::b::Y;", "use a::b::{X, {Y}};").is_err());
        // Mixed visibility must never change visibility.
        assert!(regroup("use a::b::X;\npub use a::b::Y;", "use a::b::{X, Y};").is_err());
        assert!(regroup("pub use a::b::X;\npub use a::b::Y;", "use a::b::{X, Y};").is_err());
        // No shared prefix: nothing to group at.
        assert!(regroup("use a::X;\nuse b::Y;", "use a::{X, Y};").is_err());
        // Depth mismatch: every minus must sit at the grouping depth.
        assert!(regroup(
            "use a::b::X;\nuse a::b::Y::Deep;",
            "use a::b::{X, Y::Deep};"
        )
        .is_err());
        // Duplicate tails can never merge honestly.
        let err = regroup("use a::b::X;\nuse a::b::X;", "use a::b::{X, X};").unwrap_err();
        assert!(
            err.render().contains("duplicate tails"),
            "got: {}",
            err.render()
        );
        // Mirrored attributes fail fast with the shared needle.
        let err = regroup("use a::b::X;\nuse a::b::Y;", "#[x]\nuse a::b::{X, Y};").unwrap_err();
        assert!(
            err.render().contains("mirrored attributes"),
            "got: {}",
            err.render()
        );
        // Mixed leading `::` never merges.
        assert!(regroup("use a::b::X;\nuse ::a::b::Y;", "use a::b::{X, Y};").is_err());
    }
}
