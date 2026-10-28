//! Expression/call pattern helpers (SPEC 13, 16).
//!
//! Strategy: substitute each declared metavariable occurrence with a unique
//! placeholder identifier (`__RSMPL_E_0`) and each `...` with a placeholder,
//! then parse the result with `syn`. The placeholder map lets the engine bind
//! metavars and enforce repeated-occurrence structural equality.

use rsmpl_patch::{ErrorCode, StructuredError};
use std::collections::HashMap;

use crate::metavar::MetavarTable;
use crate::quote_utils::{mask_literal_spans, unmask_literal_spans};

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

/// Byte index of the `)` matching the `(` at `open`, skipping `"..."`
/// string literals (escape-aware). Returns `None` when unbalanced.
///
/// The call router must only claim a call shape when the call consumes the
/// whole pattern (modulo a trailing `;`): without this, `foo(E) + 1` and
/// `E.foo() + 1` classify as a `foo(E)`/`E.foo()` call prefix and the
/// trailing `+ 1` is silently swallowed — the same router-precision family
/// as the T16a `let(...)` trap. Char literals containing parens (e.g.
/// `foo(')')`) are a known v1 limitation: the scan closes early and the
/// shape falls through to `Raw` (fails safe, never misclassifies).
fn balanced_close(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut i = open;
    let mut in_str = false;
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// True when `rest` (text after a call's closing paren) is empty or a lone
/// `;`: `foo(E)` and `foo(E);` are calls, `foo(E) + 1` is not.
fn is_call_trailer(rest: &str) -> bool {
    let t = rest.trim();
    t.is_empty() || t == ";"
}

/// Byte index of the last `.` outside `"..."` string literals
/// (escape-aware), or `None` when every dot sits inside a string.
///
/// The method-call split must not see dots inside string args: `sub`
/// is pattern text, so `Receiver.contains("a.b")` would otherwise split
/// at the string's dot and misroute to a dotted-callee `Call` that never
/// matches (T419 — same router-precision family as T144's
/// `balanced_close` string-skipping). `'` char literals are deliberately
/// NOT skipped: a `'` may open a lifetime (`&'a`), so treating it as a
/// char literal would swallow real dots — `R.m('.')` keeps today's
/// fail-safe no-match, mirroring T144's `foo(')')` limitation note.
fn rfind_dot_outside_strings(sub: &str) -> Option<usize> {
    let bytes = sub.as_bytes();
    let mut last = None;
    let mut i = 0;
    let mut in_str = false;
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'.' {
            last = Some(i);
        }
        i += 1;
    }
    last
}

/// High-level classify used by `pattern.rs`. Backtick literals (SPEC 15.4)
/// are masked — not stripped — so a literal `` `E` `` never becomes a
/// placeholder: sentinel args classify as `Concrete(literal)` and
/// callee/receiver/method sentinels unmask to literal text (exact match,
/// never bind). Plain `E` still classifies as `Meta`.
pub fn classify_call_pattern(
    minus_raw: &str,
    table: &MetavarTable,
) -> Result<(ExprPatternKind, PlaceholderMaps, Vec<String>), StructuredError> {
    let (masked, sentinels) = mask_literal_spans(minus_raw)?;
    let lit_strs: Vec<String> = sentinels.iter().map(|(_, lit)| lit.clone()).collect();
    let (sub, maps) = ExprPattern::placeholder_expr(&masked, table);
    // `let`-led patterns are statements, not calls: bypass call/method
    // routing so `let (E, x) = ...` reaches the `let` gate (SPEC 18.6)
    // instead of classifying as a `let(...)` call.
    if is_let_led(&unmask_literal_spans(&masked, &sentinels)) {
        return Ok((
            ExprPatternKind::Raw {
                text: normalize_trailing_commas(&unmask_literal_spans(&sub, &sentinels)),
            },
            maps,
            lit_strs,
        ));
    }
    // Reference-wrapped calls (`&E.join("")`, `&dir.path()`): the leading
    // `&` is a general-expression reference wrapper, not part of a call
    // receiver — routing it as a method call would verbatim-compare the
    // `&`-carrying receiver and never match (SPEC 18.5). Bypass to `Raw`
    // so the general matcher handles it structurally (T252).
    if sub.trim_start().starts_with('&') {
        return Ok((
            ExprPatternKind::Raw {
                text: normalize_trailing_commas(&unmask_literal_spans(&sub, &sentinels)),
            },
            maps,
            lit_strs,
        ));
    }
    // Leading `!`/`-` unary (`!f(...)`, `-f(...)`): like the `&` wrapper
    // above, the prefix is a general-expression operator, not part of a
    // call callee — routing it as `Call` would verbatim-compare the
    // `!`/`-`-carrying callee and never match (T415). Bypass to `Raw` so
    // the general matcher handles it structurally.
    let trimmed = sub.trim_start();
    if trimmed.starts_with('!') || trimmed.starts_with('-') {
        return Ok((
            ExprPatternKind::Raw {
                text: normalize_trailing_commas(&unmask_literal_spans(&sub, &sentinels)),
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
                    name: unmask_literal_spans(before, &sentinels),
                    body_ellipsis,
                },
                maps,
                lit_strs,
            ));
        }
    }
    // Method call `recv.method(args)`. A receiver carrying `=`/`;` (e.g.
    // `let x = a.b(c)`, already bypassed above for `let`) or a method
    // containing whitespace is not a method call; fall through. The call
    // must consume the whole pattern (T144 router-precision fix): trailing
    // text such as `+ 1` falls through to the general matcher. The dot
    // split skips `"..."` string literals (T419): a dot inside a string
    // arg (`Receiver.contains("a.b")`) never splits receiver from method.
    if let Some(dot) = rfind_dot_outside_strings(&sub) {
        if let Some(paren_rel) = sub[dot..].find('(') {
            let open = dot + paren_rel;
            if let Some(close) = balanced_close(&sub, open) {
                if is_call_trailer(&sub[close + 1..]) {
                    let recv = sub[..dot].trim().to_string();
                    let method = sub[dot + 1..open].trim().to_string();
                    let recv_ok = !recv.is_empty() && !recv.contains('=') && !recv.contains(';');
                    let method_ok = !method.is_empty() && !method.contains(char::is_whitespace);
                    if recv_ok && method_ok {
                        let args = classify_args(&sub[open + 1..close], table)
                            .into_iter()
                            .map(|a| restore_arg(a, &sentinels))
                            .collect();
                        return Ok((
                            ExprPatternKind::MethodCall {
                                receiver: unmask_literal_spans(recv.trim(), &sentinels),
                                method: unmask_literal_spans(method.trim(), &sentinels),
                                args,
                            },
                            maps,
                            lit_strs,
                        ));
                    }
                }
            }
        }
    }
    // Plain call `callee(args)`. A callee containing whitespace or a Rust
    // statement keyword (`let`/`if`/`match`/...) is not a call callee;
    // return `Raw` so the caller can try the general matcher. Like method
    // calls, the call must consume the whole pattern (trailing `;` kept so
    // `foo(E);` still routes to the call matcher).
    if let Some(paren) = sub.find('(') {
        if let Some(close) = balanced_close(&sub, paren) {
            if is_call_trailer(&sub[close + 1..]) {
                let callee = sub[..paren].trim().to_string();
                let callee_unmasked = unmask_literal_spans(callee.trim(), &sentinels);
                if !callee_unmasked.contains(char::is_whitespace)
                    && !is_statement_keyword(&callee_unmasked)
                {
                    let args = classify_args(&sub[paren + 1..close], table)
                        .into_iter()
                        .map(|a| restore_arg(a, &sentinels))
                        .collect();
                    return Ok((
                        ExprPatternKind::Call {
                            callee: restore_path_ellipsis(callee_unmasked, &maps),
                            args,
                        },
                        maps,
                        lit_strs,
                    ));
                }
            }
        }
    }
    Ok((
        ExprPatternKind::Raw {
            text: normalize_trailing_commas(&unmask_literal_spans(&sub, &sentinels)),
        },
        maps,
        lit_strs,
    ))
}

/// Restore `...` path-ellipsis markers (SPEC 16.5) in an extracted callee:
/// `placeholder_expr` rewrites every `...` to `__RSMPL_ELLIPSIS_N`, but the
/// engine's `match_path_segments` matches on literal `"..."` — without this
/// restore a `crate::...::old` callee would fall through to exact compare
/// and never match. Callee-only: receiver/method/macro-name `...` is
/// undocumented. Arg-list markers are handled separately by
/// `classify_args`/`restore_arg` and never pass through here.
fn restore_path_ellipsis(mut text: String, maps: &PlaceholderMaps) -> String {
    for id in &maps.ellipsis_ids {
        text = text.replace(id, "...");
    }
    text
}

/// Restore backtick-literal sentinels after classification: a bare
/// sentinel arg is the literal itself (`Concrete`); sentinels embedded in
/// larger concrete text unmask in place. Plain `Meta` placeholders pass
/// through untouched.
fn restore_arg(arg: ArgPat, sentinels: &[(String, String)]) -> ArgPat {
    match arg {
        ArgPat::Meta(ph) => match sentinels.iter().find(|(s, _)| *s == ph) {
            Some((_, lit)) => ArgPat::Concrete(lit.clone()),
            None => ArgPat::Meta(ph),
        },
        ArgPat::Concrete(c) => ArgPat::Concrete(unmask_literal_spans(&c, sentinels)),
        ArgPat::Ellipsis => ArgPat::Ellipsis,
    }
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
    fn amp_led_patterns_bypass_call_router() {
        // T252: `&E.join("")` must not classify as a method call with a
        // composite `&E` receiver (verbatim-compare, never matches); the
        // `Raw` shape lets `pattern.rs` try the general-expression gate,
        // which handles the reference wrapper structurally.
        for text in ["&E.join(\"\")", "&dir.path()", "&foo(E)"] {
            let (kind, _, _) = classify_call_pattern(text, &table()).unwrap();
            assert!(
                matches!(kind, ExprPatternKind::Raw { .. }),
                "{text:?} got: {kind:?}"
            );
        }
        // Genuine calls without a `&` wrapper are untouched.
        let (kind, _, _) = classify_call_pattern("E.join(\"\")", &table()).unwrap();
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

    #[test]
    fn trailing_text_falls_through_to_general() {
        // T144 router-precision fix (same family as T16a): a call shape
        // must consume the whole pattern. `foo(E) + 1` is a binary general
        // expression, not a `foo(E)` call prefix; likewise `E.foo() + 1`
        // is not a bare `E.foo()` method call. `Raw` lets `pattern.rs` try
        // the general matcher. (Residual: `a.b(c) + d.e(f)` still claims
        // the last-dot method call and fails safe with no match — split
        // multi-call binaries need an explicit general shape; see TASKS.)
        for text in ["foo(E) + 1", "__RSMPL_E.foo() + 1", "foo(E) + foo(E)"] {
            let (kind, _, _) = classify_call_pattern(text, &table()).unwrap();
            assert!(
                matches!(kind, ExprPatternKind::Raw { .. }),
                "{text:?} got: {kind:?}"
            );
        }
        // Genuine calls (incl. trailing `;` and nested args) are untouched.
        for text in [
            "foo(E)",
            "foo(E);",
            "Receiver.old_method(Arg)",
            "__RSMPL_E.foo();",
            "foo(a(b))",
            "foo(__RSMPL_E, x)",
        ] {
            let (kind, _, _) = classify_call_pattern(text, &table()).unwrap();
            assert!(
                matches!(kind, ExprPatternKind::Call { .. })
                    || matches!(kind, ExprPatternKind::MethodCall { .. }),
                "{text:?} got: {kind:?}"
            );
        }
    }

    #[test]
    fn backtick_literals_stay_concrete_on_call_shapes() {
        // T162: `` `E` `` with E declared must classify as Concrete, not
        // Meta — masking (not stripping) preserves the literal.
        let (kind, _, _) = classify_call_pattern("foo(`E`)", &table()).unwrap();
        let ExprPatternKind::Call { callee, args } = kind else {
            panic!("foo(`E`) got a non-call shape");
        };
        assert_eq!(callee, "foo");
        assert_eq!(args, vec![ArgPat::Concrete("E".to_string())]);
        // Undeclared backtick names are likewise concrete.
        let (kind, _, _) = classify_call_pattern("foo(`Q`)", &table()).unwrap();
        let ExprPatternKind::Call { args, .. } = kind else {
            panic!("foo(`Q`) got a non-call shape");
        };
        assert_eq!(args, vec![ArgPat::Concrete("Q".to_string())]);
        // A backticked callee is a concrete callee, never a bind.
        let (kind, _, _) = classify_call_pattern("`foo`(E)", &table()).unwrap();
        let ExprPatternKind::Call { callee, args } = kind else {
            panic!("`foo`(E) got a non-call shape");
        };
        assert_eq!(callee, "foo");
        assert_eq!(args, vec![ArgPat::Meta("__RSMPL_E".to_string())]);
        // Plain `foo(E)` still binds.
        let (kind, _, _) = classify_call_pattern("foo(E)", &table()).unwrap();
        let ExprPatternKind::Call { args, .. } = kind else {
            panic!("foo(E) got a non-call shape");
        };
        assert_eq!(args, vec![ArgPat::Meta("__RSMPL_E".to_string())]);
        // Method-call literal args stay concrete too.
        let (kind, _, _) = classify_call_pattern("Receiver.old_method(`Arg`)", &table()).unwrap();
        let ExprPatternKind::MethodCall { args, .. } = kind else {
            panic!("method literal got a non-method shape");
        };
        assert_eq!(args, vec![ArgPat::Concrete("Arg".to_string())]);
    }

    #[test]
    fn path_ellipsis_survives_callee_extraction() {
        // T164 (SPEC 16.5): `placeholder_expr` must not destroy the
        // callee's `...` — the engine matches on literal `"..."`.
        let (kind, _, _) = classify_call_pattern("crate::...::old(E)", &table()).unwrap();
        let ExprPatternKind::Call { callee, args } = kind else {
            panic!("path-ellipsis call got a non-call shape");
        };
        assert_eq!(callee, "crate::...::old");
        assert_eq!(args, vec![ArgPat::Meta("__RSMPL_E".to_string())]);
        // Plain concrete callees and arg ellipses are untouched.
        let (kind, _, _) = classify_call_pattern("crate::a::old(E)", &table()).unwrap();
        let ExprPatternKind::Call { callee, .. } = kind else {
            panic!("concrete-path call got a non-call shape");
        };
        assert_eq!(callee, "crate::a::old");
        let (kind, _, _) = classify_call_pattern("foo(...)", &table()).unwrap();
        let ExprPatternKind::Call { callee, args } = kind else {
            panic!("arg-ellipsis call got a non-call shape");
        };
        assert_eq!(callee, "foo");
        assert_eq!(args, vec![ArgPat::Ellipsis]);
    }
    #[test]
    fn method_turbofish_stays_method_call() {
        // T176: `Receiver.collect::<T>()` must classify as `MethodCall`
        // (method text keeps the turbofish) so the engine's structural
        // `match_method_turbofish` arm sees it — never `Raw`.
        let (kind, _, _) = classify_call_pattern("Receiver.collect::<T>()", &table()).unwrap();
        let ExprPatternKind::MethodCall {
            receiver,
            method,
            args,
        } = kind
        else {
            panic!("turbofish method got a non-method shape");
        };
        assert_eq!(receiver, "__RSMPL_Receiver");
        assert!(method.contains('<'), "turbofish lost: {method}");
        assert!(args.is_empty());
    }

    #[test]
    fn method_dot_inside_string_arg_stays_method_call() {
        // T419: the receiver/method dot split skips `"..."` string
        // literals (same router-precision family as T144's
        // `balanced_close` string-skipping) — `Receiver.contains("a.b")`
        // split at the string's dot before, misrouting to a dotted-callee
        // `Call` that never matched.
        let (kind, _, _) = classify_call_pattern("Receiver.contains(\"a.b\")", &table()).unwrap();
        let ExprPatternKind::MethodCall {
            receiver,
            method,
            args,
        } = kind
        else {
            panic!("dotted-string method got a non-method shape");
        };
        assert_eq!(receiver, "__RSMPL_Receiver");
        assert_eq!(method, "contains");
        assert_eq!(args, vec![ArgPat::Concrete("\"a.b\"".to_string())]);
        // A receiver carrying a dotted string still routes correctly.
        let (kind, _, _) = classify_call_pattern("foo(\"a.b\").m(x)", &table()).unwrap();
        let ExprPatternKind::MethodCall {
            receiver, method, ..
        } = kind
        else {
            panic!("string-carrying receiver got a non-method shape");
        };
        assert_eq!(receiver, "foo(\"a.b\")");
        assert_eq!(method, "m");
        // Parens and dots inside strings never confuse the split.
        let (kind, _, _) = classify_call_pattern("R.m(\"a.b(c)\")", &table()).unwrap();
        assert!(
            matches!(kind, ExprPatternKind::MethodCall { .. }),
            "string parens misrouted: {kind:?}"
        );
        // String-free shapes classify exactly as before.
        let (kind, _, _) = classify_call_pattern("R.m(\"nodot\")", &table()).unwrap();
        assert!(
            matches!(kind, ExprPatternKind::MethodCall { .. }),
            "plain string arg regressed: {kind:?}"
        );
        let (kind, _, _) = classify_call_pattern("a.b(c)", &table()).unwrap();
        assert!(
            matches!(kind, ExprPatternKind::MethodCall { .. }),
            "plain method regressed: {kind:?}"
        );
    }
    #[test]
    fn leading_unary_bypasses_call_router_to_raw() {
        // T415: a leading `!`/`-` is a general-expression operator, not
        // part of a call callee — `!foo(...)` reached the macro pre-route
        // and `-foo(...)` classified as `Call` callee `-foo` (both
        // probe-verified never-match). Both bypass to `Raw` so
        // `try_general` compiles them as unary general-exprs (mirrors
        // the T252 `&` bypass).
        for text in ["!check(E)", "!hit(&a, &b)", "-neg(E)", "-foo(x)"] {
            let (kind, _, _) = classify_call_pattern(text, &table()).unwrap();
            assert!(matches!(kind, ExprPatternKind::Raw { .. }), "{text}");
        }
        // `NAME!(...)` still classifies as a macro invocation.
        let (kind, _, _) = classify_call_pattern("foo!(E)", &table()).unwrap();
        assert!(matches!(kind, ExprPatternKind::Macro { .. }));
    }
}
