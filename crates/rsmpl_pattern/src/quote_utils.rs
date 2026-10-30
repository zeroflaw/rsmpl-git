//! Backtick literal-token quoting (SPEC 15.4).
//!
//! Bare `E` with `# type E` declared means the metavariable; `` `E` `` means
//! the literal identifier `E`. `` `r#type` `` is the canonical escape for the
//! raw identifier `r#type`; `r#` + backtick is invalid.

use rsmpl_patch::{ErrorCode, StructuredError};

/// A literal token that was backtick-quoted in the pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiteralToken {
    /// De-quoted literal text (e.g. `` `E` `` -> `E`, `` `r#type` `` -> `r#type`).
    pub literal: String,
    /// Byte offset of the opening backtick in the original text.
    pub offset: usize,
}

/// Validate quoting without stripping: balanced backticks, no `r#`` split.
pub fn validate_quoting(text: &str) -> Result<(), StructuredError> {
    let count = text.chars().filter(|&c| c == '`').count();
    if count % 2 != 0 {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("unbalanced backtick quoting in pattern `{text}`"),
        ));
    }
    if text.contains("r#`") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "invalid raw-identifier quoting `r#` + backtick in `{text}` (use `` `r#...` ``)"
            ),
        ));
    }
    // Each quoted section must hold exactly one lexical token.
    for (inner, _) in quoted_sections(text)? {
        validate_single_token(&inner)?;
    }
    Ok(())
}

/// Strip backtick quotes: returns normalized text + literals found.
pub fn strip_literal_quotes(text: &str) -> Result<(String, Vec<LiteralToken>), StructuredError> {
    validate_quoting(text)?;
    let mut out = String::with_capacity(text.len());
    let mut literals = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let rel = text[i + 1..].find('`').ok_or_else(|| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("unbalanced backtick quoting in pattern `{text}`"),
                )
            })?;
            let inner = &text[i + 1..i + 1 + rel];
            literals.push(LiteralToken {
                literal: inner.to_string(),
                offset: i,
            });
            out.push_str(inner);
            i += 1 + rel + 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok((out, literals))
}

/// Replace each backtick-quoted span with a unique sentinel ident so later
/// whole-token substitution cannot mistake literals for metavariables.
/// Returns the masked text plus (sentinel -> literal) pairs for restoration.
pub fn mask_literal_spans(text: &str) -> Result<(String, Vec<(String, String)>), StructuredError> {
    validate_quoting(text)?;
    let mut out = String::with_capacity(text.len());
    let mut map = Vec::new();
    let mut i = 0;
    let mut n = 0usize;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        if c == '`' {
            let rest = &text[i + c.len_utf8()..];
            let rel = rest.find('`').ok_or_else(|| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("unbalanced backtick quoting in pattern `{text}`"),
                )
            })?;
            let inner = &rest[..rel];
            let sent = format!("__RSMPLQLIT{n}Q");
            n += 1;
            map.push((sent.clone(), inner.to_string()));
            out.push_str(&sent);
            i += c.len_utf8() + rel + 1;
        } else {
            out.push(c);
            i += c.len_utf8();
        }
    }
    Ok((out, map))
}

/// Restore masked sentinels to their literal text.
pub fn unmask_literal_spans(text: &str, map: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (sent, lit) in map {
        out = out.replace(sent, lit);
    }
    out
}

fn quoted_sections(text: &str) -> Result<Vec<(String, usize)>, StructuredError> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let rel = text[i + 1..].find('`').ok_or_else(|| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("unbalanced backtick quoting in pattern `{text}`"),
                )
            })?;
            out.push((text[i + 1..i + 1 + rel].to_string(), i));
            i += 1 + rel + 1;
        } else {
            i += 1;
        }
    }
    Ok(out)
}

/// A quoted section must be one Rust lexical token: identifier, raw
/// identifier, lifetime/label, or path segment.
fn validate_single_token(inner: &str) -> Result<(), StructuredError> {
    if inner.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "empty backtick quote ```` in pattern".to_string(),
        ));
    }
    if inner.contains(char::is_whitespace) {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("backtick quote must hold a single token, got `{inner}`"),
        ));
    }
    // Lifetimes/labels start with `'`.
    let body = inner.strip_prefix('\'').unwrap_or(inner);
    // Raw identifiers: `r#ident` or `r#_`.
    if let Some(rest) = body.strip_prefix("r#") {
        if rest.is_empty()
            || !(rest.chars().next().unwrap().is_ascii_alphabetic() || rest.starts_with('_'))
            || !rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid raw identifier in backtick quote `{inner}`"),
            ));
        }
        return Ok(());
    }
    // Plain identifier (possibly `_`).
    let mut chars = body.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid literal token in backtick quote `{inner}`"),
        ));
    }
    if !body.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid literal token in backtick quote `{inner}`"),
        ));
    }
    Ok(())
}

/// Byte index just past the closing `"` of the string starting at `start`
/// (the opening quote), escape-aware. Returns `None` when unclosed before
/// the newline — the caller then treats the rest of the line as opaque.
pub fn string_end(text: &str, start: usize) -> Option<usize> {
    let mut it = text[start + 1..].char_indices();
    while let Some((rel, c)) = it.next() {
        if c == '\n' {
            return None;
        }
        if c == '\\' {
            it.next();
            continue;
        }
        if c == '"' {
            return Some(start + 1 + rel + c.len_utf8());
        }
    }
    None
}

/// Byte length of the char literal starting at `start` (`'<c>'` or
/// `'<esc>'`), else `None`. Lifetimes (`'a`, `'static`) never match: only
/// a quote-closed one- or two-char body counts.
pub fn char_lit_len(text: &str, start: usize) -> Option<usize> {
    let mut it = text[start + 1..].chars();
    let first = it.next()?;
    if first == '\n' {
        return None;
    }
    if first == '\\' {
        let esc = it.next()?;
        if esc == '\n' {
            return None;
        }
        let q = it.next()?;
        if q != '\'' {
            return None;
        }
        return Some(2 + first.len_utf8() + esc.len_utf8());
    }
    let q = it.next()?;
    if q != '\'' {
        return None;
    }
    Some(1 + first.len_utf8() + 1)
}

/// True when `s` is exactly a minted `...` marker (`__RSMPL_ELLIPSIS_<digits>`).
/// Metavar placeholders (`__RSMPL_{name}`) with `ELLIPSIS_*` names are fenced
/// at parse time, but the predicate stays exact so such text never reads as
/// a marker even if it reaches here (WI-5b-follow).
pub fn is_ellipsis_marker_ident(s: &str) -> bool {
    s.strip_prefix("__RSMPL_ELLIPSIS_")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// True when `text` holds a minted `...` marker anywhere (digit-suffixed only,
/// so `__RSMPL_ELLIPSIS_FOO` never counts — WI-5b-follow).
pub fn has_ellipsis_marker(text: &str) -> bool {
    let mut i = 0;
    while let Some(rel) = text[i..].find("__RSMPL_ELLIPSIS_") {
        let start = i + rel + "__RSMPL_ELLIPSIS_".len();
        let mut j = start;
        while j < text.len() && text.as_bytes()[j].is_ascii_digit() {
            j += 1;
        }
        if j > start {
            return true;
        }
        i = start;
    }
    false
}

/// Count minted `...` markers (`__RSMPL_ELLIPSIS_<digits>`) in `text`.
/// Digit-suffixed only, matching `is_ellipsis_marker_ident` (WI-5b-follow).
pub fn count_ellipsis_markers(text: &str) -> usize {
    let mut n = 0;
    let mut i = 0;
    while let Some(rel) = text[i..].find("__RSMPL_ELLIPSIS_") {
        let start = i + rel + "__RSMPL_ELLIPSIS_".len();
        let mut j = start;
        while j < text.len() && text.as_bytes()[j].is_ascii_digit() {
            j += 1;
        }
        if j > start {
            n += 1;
        }
        i = start;
    }
    n
}

/// Qualifier-led fn routing (WI-6 slice 1): `async {` / `unsafe {` open
/// block expressions, not fn headers — only treat an `async `/`unsafe `-led
/// line as fn-like when `fn`, `trait`, or `impl` appears before the first
/// `{` (or anywhere when the line has no `{`, e.g. a terse header).
/// Callers check the `async `/`unsafe ` prefix first; this decides the rest.
pub fn qualifier_leads_to_fn(t: &str) -> bool {
    let head = t.split_once('{').map(|(h, _)| h).unwrap_or(t);
    head.contains("fn ")
        || head.contains("fn(")
        || head.contains("trait ")
        || head.contains("impl ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_simple() {
        let (n, l) = strip_literal_quotes("Result<T, `E`>").unwrap();
        assert_eq!(n, "Result<T, E>");
        assert_eq!(l[0].literal, "E");
    }

    #[test]
    fn raw_ident_ok() {
        assert!(validate_quoting("`r#type`").is_ok());
        assert!(validate_quoting("crate::`r#type`::new").is_ok());
    }

    #[test]
    fn split_raw_ident_rejected() {
        assert!(validate_quoting("r#`type`").is_err());
    }

    #[test]
    fn unbalanced_rejected() {
        assert!(validate_quoting("`E").is_err());
    }

    #[test]
    fn ellipsis_marker_is_digit_suffixed_only() {
        // WI-5b-follow: only minted `__RSMPL_ELLIPSIS_<digits>` counts.
        assert!(is_ellipsis_marker_ident("__RSMPL_ELLIPSIS_0"));
        assert!(is_ellipsis_marker_ident("__RSMPL_ELLIPSIS_12"));
        assert!(!is_ellipsis_marker_ident("__RSMPL_ELLIPSIS"));
        assert!(!is_ellipsis_marker_ident("__RSMPL_ELLIPSIS_"));
        assert!(!is_ellipsis_marker_ident("__RSMPL_ELLIPSIS_FOO"));
        assert!(!is_ellipsis_marker_ident("__RSMPL_E_0"));
    }

    #[test]
    fn ellipsis_marker_scan_ignores_non_digits() {
        assert!(has_ellipsis_marker("call(__RSMPL_ELLIPSIS_3)"));
        assert_eq!(
            count_ellipsis_markers("a __RSMPL_ELLIPSIS_1 b __RSMPL_ELLIPSIS_2"),
            2
        );
        assert!(!has_ellipsis_marker("declares __RSMPL_ELLIPSIS_FOO here"));
        assert_eq!(count_ellipsis_markers("__RSMPL_ELLIPSIS_FOO"), 0);
    }

    #[test]
    fn qualifier_routing_tells_blocks_from_headers() {
        // WI-6 slice 1: block expressions are not fn-like.
        assert!(!qualifier_leads_to_fn("async { foo() }"));
        assert!(!qualifier_leads_to_fn("async move { foo() }"));
        assert!(!qualifier_leads_to_fn("unsafe { foo() }"));
        // Headers stay fn-like, before or without a `{`.
        assert!(qualifier_leads_to_fn("async fn foo() {"));
        assert!(qualifier_leads_to_fn("unsafe fn foo() {"));
        assert!(qualifier_leads_to_fn("unsafe trait T {"));
        assert!(qualifier_leads_to_fn("unsafe impl S {"));
        assert!(qualifier_leads_to_fn("async fn foo()"));
    }
}
