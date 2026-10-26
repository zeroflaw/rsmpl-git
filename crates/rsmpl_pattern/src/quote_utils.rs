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
}
