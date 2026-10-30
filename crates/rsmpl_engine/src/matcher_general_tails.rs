//! WI-5b tail substitution for the general matcher (split from
//! `matcher_general` per D5 — the parent file sits at the 2000-line gate).
//!
//! `substitute_general_plus` replays absorbed trailing-block tails into the
//! plus `...` slots (or drops them when the plus side omits `...`). Slotting
//! precedes binding substitution so replayed tails are never rewritten by
//! metavar replacement, binding values containing `...` are never mistaken
//! for slots, and `...` inside string literals, char literals, or comments
//! stays concrete text.

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};
use rsmpl_pattern::{char_lit_len, string_end};

use crate::matcher_expr::substitute_plus;

/// Substitute bound metavariables into the plus pattern, restoring backtick
/// literals (masked so substitution cannot touch them).
///
/// WI-5b: trailing block `...` slots replay the absorbed tails. Each
/// standalone code `...` becomes a `__RSMPL_TAIL_N` slot filled from `tails`
/// in order (plus with no `...` drops every tail); `...` inside string
/// literals, char literals, or comments is concrete text and rides through
/// untouched. Slotting precedes binding substitution so replayed tails are
/// never rewritten by metavar replacement, and binding values containing
/// `...` are never mistaken for slots.
pub(crate) fn substitute_general_plus(
    plus_raw: &str,
    bindings: &HashMap<String, String>,
    tails: &[String],
    sigiled: bool,
) -> Result<String, StructuredError> {
    let (masked, sentinels) = rsmpl_pattern::mask_literal_spans(plus_raw)?;
    let (slotted, dots) = slot_block_ellipsis(&masked);
    let mut out = substitute_plus(&slotted, bindings, &[], sigiled);
    for (sent, lit) in &sentinels {
        out = out.replace(sent, lit);
    }
    for (sent, _) in &dots {
        out = out.replace(sent, "...");
    }
    for (i, tail) in tails.iter().enumerate() {
        out = out.replace(&format!("__RSMPL_TAIL_{i}"), tail);
    }
    if out.contains("__RSMPL_TAIL_") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "block tail replay failed (plus `...` count does not match the absorbed tails)",
        ));
    }
    Ok(out)
}

/// Split plus-side `...` into replay slots: standalone code occurrences
/// become `__RSMPL_TAIL_N` (in order); occurrences inside strings, chars,
/// or comments become opaque `__RSMPL_DOTS_M` sentinels restored to `...`
/// after substitution. Returns the slotted text plus the dots sentinels.
fn slot_block_ellipsis(text: &str) -> (String, Vec<(String, ())>) {
    let mut out = String::with_capacity(text.len());
    let mut dots = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut nslots = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '"' {
            match string_end(text, i) {
                Some(end) => {
                    push_masked_dots(&text[i..end], &mut out, &mut dots);
                    i = end;
                }
                None => {
                    let rest = &text[i..];
                    let end = rest.find('\n').map(|p| i + p).unwrap_or(text.len());
                    push_masked_dots(&text[i..end], &mut out, &mut dots);
                    i = end;
                }
            }
            continue;
        }
        if c == '\'' {
            match char_lit_len(text, i) {
                Some(len) => {
                    push_masked_dots(&text[i..i + len], &mut out, &mut dots);
                    i += len;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let rest = &text[i..];
            let end = rest.find('\n').map(|p| i + p).unwrap_or(text.len());
            push_masked_dots(&text[i..end], &mut out, &mut dots);
            i = end;
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            if let Some(rel) = text[i + 2..].find("*/") {
                push_masked_dots(&text[i..i + 2 + rel + 2], &mut out, &mut dots);
                i += 2 + rel + 2;
                continue;
            }
            push_masked_dots(&text[i..], &mut out, &mut dots);
            break;
        }
        if c == '.' && is_standalone_ellipsis(text, i) {
            out.push_str(&format!("__RSMPL_TAIL_{nslots}"));
            nslots += 1;
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    (out, dots)
}

/// Copy `span` (a string/char/comment slice) hiding any `...` inside as
/// opaque sentinels, so later substitution treats them as concrete text.
fn push_masked_dots(span: &str, out: &mut String, dots: &mut Vec<(String, ())>) {
    let mut j = 0;
    while let Some(rel) = span[j..].find("...") {
        out.push_str(&span[j..j + rel]);
        let sent = format!("__RSMPL_DOTS_{}", dots.len());
        out.push_str(&sent);
        dots.push((sent, ()));
        j += rel + 3;
    }
    out.push_str(&span[j..]);
}

/// True when the `...` at byte `i` stands alone (not part of a longer dot
/// run), mirroring the substitution pass it feeds.
fn is_standalone_ellipsis(text: &str, i: usize) -> bool {
    let bytes = text.as_bytes();
    if i + 2 >= bytes.len() || &text[i..i + 3] != "..." {
        return false;
    }
    let before_ok = i == 0 || bytes[i - 1] != b'.';
    let after_ok = i + 3 >= bytes.len() || bytes[i + 3] != b'.';
    before_ok && after_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(plus: &str, tails: &[&str]) -> String {
        let owned: Vec<String> = tails.iter().map(|s| s.to_string()).collect();
        substitute_general_plus(plus, &HashMap::new(), &owned, false).unwrap()
    }

    #[test]
    fn replay_fills_slots_in_order() {
        let out = sub("if E { a(); ... } else { b(); ... }", &["t1();", "t2();"]);
        assert!(out.contains("t1();"), "got: {out}");
        assert!(out.contains("t2();"), "got: {out}");
        assert!(!out.contains("..."), "got: {out}");
    }

    #[test]
    fn drop_leaves_no_slot() {
        let out = sub("if E { a(); }", &["t1();"]);
        assert_eq!(out, "if E { a(); }", "got: {out}");
    }

    #[test]
    fn string_dots_stay_concrete() {
        let out = sub("if E { log(\"...\"); ... }", &["t();"]);
        assert!(out.contains("log(\"...\")"), "got: {out}");
        assert!(out.contains("t();"), "got: {out}");
    }

    #[test]
    fn empty_tail_replays_empty() {
        let out = sub("if E { a(); ... }", &[""]);
        assert!(!out.contains("..."), "got: {out}");
        assert!(!out.contains("__RSMPL_TAIL_"), "got: {out}");
    }
}
