//! Expression/call pattern helpers (SPEC 13, 16).
//!
//! Strategy: substitute each declared metavariable occurrence with a unique
//! placeholder identifier (`__RSMPL_E_0`) and each `...` with a placeholder,
//! then parse the result with `syn`. The placeholder map lets the engine bind
//! metavars and enforce repeated-occurrence structural equality.

use rsmpl_patch::{ErrorCode, StructuredError};
use std::collections::HashMap;

use crate::metavar::MetavarTable;
use crate::quote_utils::strip_literal_quotes;

/// An argument slot in a call-like pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgPat {
    /// `...` wildcard (matches zero or more args).
    Ellipsis,
    /// A metavariable capturing one expression.
    Meta(String),
    /// Concrete expression text.
    Concrete(String),
}

/// Shape of an expression pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprPatternKind {
    Call {
        callee: String,
        args: Vec<ArgPat>,
    },
    MethodCall {
        receiver: String,
        method: String,
        args: Vec<ArgPat>,
    },
    Macro {
        name: String,
        body_ellipsis: bool,
    },
    Raw {
        text: String,
    },
}

/// Placeholder substitution maps for syn parsing.
#[derive(Debug, Clone, Default)]
pub struct PlaceholderMaps {
    /// placeholder ident -> (kind, original name). Kind is informational.
    pub expr_placeholders: HashMap<String, String>,
    pub ellipsis_ids: Vec<String>,
}

/// A parsed expression pattern.
#[derive(Debug, Clone)]
pub struct ExprPattern {
    pub kind: ExprPatternKind,
    pub placeholders: PlaceholderMaps,
    pub literals: Vec<String>,
    pub normalized_minus: String,
    pub normalized_plus: String,
}

impl ExprPattern {
    /// Substitute metavars with placeholders for syn parsing.
    pub fn placeholder_expr(text: &str, table: &MetavarTable) -> (String, PlaceholderMaps) {
        let mut maps = PlaceholderMaps::default();
        let mut out = text.to_string();
        // Replace `...` first with a comma-safe marker.
        let mut ell_n = 0;
        while let Some(pos) = out.find("...") {
            let id = format!("__RSMPL_ELLIPSIS_{ell_n}");
            out.replace_range(pos..pos + 3, &id);
            maps.ellipsis_ids.push(id);
            ell_n += 1;
        }
        // Replace declared metavars (longest names first) as whole tokens.
        let mut names: Vec<String> = table.kinds.keys().cloned().collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        for name in names {
            // Skip symbol bindings here? Symbols are also placeholders.
            let ph = format!("__RSMPL_{name}");
            // Whole-token replace: scan and replace ident tokens only.
            out = replace_ident_token(&out, &name, &ph, table.sigiled);
            maps.expr_placeholders.insert(ph, name);
        }
        (out, maps)
    }
}

fn replace_ident_token(text: &str, name: &str, placeholder: &str, sigiled: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let starts = if sigiled {
            c == '$'
        } else {
            c == '$' || c.is_ascii_alphabetic() || c == '_'
        };
        if starts {
            let mut j = i + 1;
            while j < bytes.len() {
                let d = bytes[j] as char;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            let tok = &text[i..j];
            let bare = tok.strip_prefix('$').unwrap_or(tok);
            if bare == name && (!sigiled || tok.starts_with('$')) {
                // Check token boundaries (prev/next not ident chars).
                out.push_str(placeholder);
            } else {
                out.push_str(tok);
            }
            i = j;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Classify a call-like pattern's argument list text into `ArgPat`s.
pub fn classify_args(args_text: &str, table: &MetavarTable) -> Vec<ArgPat> {
    let mut args = Vec::new();
    for part in split_top_level(args_text) {
        let t = part.trim().to_string();
        if t.is_empty() {
            continue;
        }
        if t == "..." || t.contains("__RSMPL_ELLIPSIS") {
            args.push(ArgPat::Ellipsis);
        } else if table.is_meta(&t)
            || (t.starts_with('$') && table.is_meta(t.trim_start_matches('$')))
        {
            args.push(ArgPat::Meta(t.trim_start_matches('$').to_string()));
        } else if t.starts_with("__RSMPL_") {
            // Placeholder produced earlier; map back if possible.
            args.push(ArgPat::Meta(t));
        } else {
            args.push(ArgPat::Concrete(t));
        }
    }
    args
}

fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    let chars = s.chars().peekable();
    for c in chars {
        match c {
            '(' | '[' | '{' | '<' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur);
    }
    out
}

/// Try to parse normalized text as a Rust expression (SPEC 13.4).
pub fn try_parse_expr(text: &str) -> Result<syn::Expr, StructuredError> {
    syn::parse_str::<syn::Expr>(text).map_err(|e| {
        StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid expression pattern `{text}`: {e}"),
        )
    })
}

/// Validate trailing-comma tolerance note: patterns ending `,)` also match
/// without comma — enforced at match time, nothing to parse here.
pub fn normalize_trailing_commas(text: &str) -> String {
    text.to_string()
}

/// True when the (pre-placeholder) pattern text is `let`-led: `let`
/// followed by whitespace, `(`, or `[`. Such patterns are statements and
/// must reach the `let` gate, never the call router.
fn is_let_led(stripped: &str) -> bool {
    let t = stripped.trim_start();
    t.strip_prefix("let").is_some_and(|rest| {
        rest.starts_with(char::is_whitespace) || rest.starts_with('(') || rest.starts_with('[')
    })
}

/// Statement keywords that can never be a plain-call callee. A pattern
/// like `if (x)` is not a call to `if`; returning `Raw` lets the general
/// matcher produce the correct fail-fast diagnostic.
fn is_statement_keyword(callee: &str) -> bool {
    matches!(
        callee,
        "let" | "if" | "match" | "for" | "while" | "loop" | "return" | "else"
    )
}

/// High-level classify used by `pattern.rs`.
pub fn classify_call_pattern(
    minus_raw: &str,
    table: &MetavarTable,
) -> Result<(ExprPatternKind, PlaceholderMaps, Vec<String>), StructuredError> {
    let (stripped, literals) = strip_literal_quotes(minus_raw)?;
    let lit_strs: Vec<String> = literals.into_iter().map(|l| l.literal).collect();
    let (sub, maps) = ExprPattern::placeholder_expr(&stripped, table);
    // `let`-led patterns are statements, not calls: bypass call/method
    // routing so `let (E, x) = ...` reaches the `let` gate (SPEC 18.6)
    // instead of classifying as a `let(...)` call.
    if is_let_led(&stripped) {
        return Ok((
            ExprPatternKind::Raw {
                text: normalize_trailing_commas(&sub),
            },
            maps,
            lit_strs,
        ));
    }
    // Macro invocation `name!(...)`.
    if let Some(bang) = sub.find('!') {
        let before = sub[..bang].trim();
        if !before.contains(char::is_whitespace) && !before.is_empty() {
            let body_ellipsis = sub.contains("__RSMPL_ELLIPSIS");
            return Ok((
                ExprPatternKind::Macro {
                    name: before.to_string(),
                    body_ellipsis,
                },
                maps,
                lit_strs,
            ));
        }
    }
    // Method call `recv.method(args)`. A receiver carrying `=`/`;` (e.g.
    // `let x = a.b(c)`, already bypassed above for `let`) or a method
    // containing whitespace is not a method call; fall through.
    if let Some(dot) = sub.rfind('.') {
        if let Some(paren) = sub[dot..].find('(') {
            let recv = sub[..dot].trim().to_string();
            let method = sub[dot + 1..dot + paren].trim().to_string();
            let recv_ok = !recv.is_empty() && !recv.contains('=') && !recv.contains(';');
            let method_ok = !method.is_empty() && !method.contains(char::is_whitespace);
            if recv_ok && method_ok {
                let args_inner = sub[dot + paren + 1..]
                    .rsplit_once(')')
                    .map(|(a, _)| a)
                    .unwrap_or("");
                let args = classify_args(args_inner, table);
                return Ok((
                    ExprPatternKind::MethodCall {
                        receiver: recv,
                        method,
                        args,
                    },
                    maps,
                    lit_strs,
                ));
            }
        }
    }
    // Plain call `callee(args)`. A callee containing whitespace or a Rust
    // statement keyword (`let`/`if`/`match`/...) is not a call callee;
    // return `Raw` so the caller can try the general matcher.
    if let Some(paren) = sub.find('(') {
        let callee = sub[..paren].trim().to_string();
        if !callee.contains(char::is_whitespace) && !is_statement_keyword(&callee) {
            let args_inner = sub[paren + 1..]
                .rsplit_once(')')
                .map(|(a, _)| a)
                .unwrap_or("");
            let args = classify_args(args_inner, table);
            return Ok((ExprPatternKind::Call { callee, args }, maps, lit_strs));
        }
    }
    Ok((
        ExprPatternKind::Raw {
            text: normalize_trailing_commas(&sub),
        },
        maps,
        lit_strs,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_patch::{Directive, MetavarKind};

    fn table() -> MetavarTable {
        MetavarTable::from_directives(&[Directive::MetaVar {
            kind: MetavarKind::Expr,
            names: vec!["E".to_string(), "Receiver".to_string(), "Arg".to_string()],
        }])
        .unwrap()
    }

    #[test]
    fn classifies_call_with_ellipsis() {
        let (kind, _, _) = classify_call_pattern("foo(...)", &table()).unwrap();
        assert!(matches!(kind, ExprPatternKind::Call { .. }));
        if let ExprPatternKind::Call { args, .. } = kind {
            assert_eq!(args, vec![ArgPat::Ellipsis]);
        }
    }

    #[test]
    fn classifies_method_call() {
        let (kind, _, _) = classify_call_pattern("Receiver.old_method(Arg)", &table()).unwrap();
        assert!(matches!(kind, ExprPatternKind::MethodCall { .. }));
    }

    #[test]
    fn parses_concrete_expr() {
        assert!(try_parse_expr("foo(x)").is_ok());
    }

    #[test]
    fn let_led_patterns_bypass_call_router() {
        // T16a: `let (...)` must not classify as a `let(...)` call; the
        // `Raw` shape lets `pattern.rs` try the general `let` gate.
        for text in [
            "let (E, x) = E;",
            "let [E, x] = E;",
            "let x = E;",
            "let x = foo(E)",
        ] {
            let (kind, _, _) = classify_call_pattern(text, &table()).unwrap();
            assert!(
                matches!(kind, ExprPatternKind::Raw { .. }),
                "{text:?} got: {kind:?}"
            );
        }
        // Genuine calls are untouched.
        let (kind, _, _) = classify_call_pattern("foo(E)", &table()).unwrap();
        assert!(matches!(kind, ExprPatternKind::Call { .. }));
        let (kind, _, _) = classify_call_pattern("Receiver.old_method(Arg)", &table()).unwrap();
        assert!(matches!(kind, ExprPatternKind::MethodCall { .. }));
    }

    #[test]
    fn statement_keywords_are_not_callees() {
        for text in ["if (E)", "match (E)", "return (E)"] {
            let (kind, _, _) = classify_call_pattern(text, &table()).unwrap();
            assert!(
                matches!(kind, ExprPatternKind::Raw { .. }),
                "{text:?} got: {kind:?}"
            );
        }
    }
}
