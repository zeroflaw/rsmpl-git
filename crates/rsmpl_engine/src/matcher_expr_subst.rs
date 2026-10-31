//! Plus-pattern substitution for call/method-call rewrites (SPEC 15.4, 16.3).
//!
//! Split from `matcher_expr.rs` (gate headroom): `substitute_plus` /
//! `substitute_call_plus` plus the ellipsis/ident helpers. Byte-identical
//! move; matching and apply stay in the parent module.

use std::collections::HashMap;

/// Substitute bound metavariables and `...` into the plus pattern.
/// Each standalone `...` replays, in order, the target args the matching
/// `- ...` absorbed (T157: the whole arg list is NOT replayed). An empty
/// remainder swallows one adjacent comma so `bar(E, ...)` on `foo(1)`
/// yields `bar(1)`, not `bar(1, )`. Surplus `...` (unreachable after the
/// plus/minus count gate) replay empty.
pub fn substitute_plus(
    plus_raw: &str,
    bindings: &HashMap<String, String>,
    ellipses: &[String],
    sigiled: bool,
) -> String {
    // Replace standalone `...` first (not part of longer dot runs).
    let mut out = replace_ellipsis_tokens(plus_raw, ellipses);
    // Longest names first to avoid prefix collisions.
    let mut names: Vec<&String> = bindings.keys().collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in names {
        let cap = &bindings[name];
        if sigiled {
            out = replace_ident_token(&out, &format!("${name}"), cap);
        } else {
            out = replace_ident_token(&out, name, cap);
            out = replace_ident_token(&out, &format!("${name}"), cap);
        }
    }
    out
}

/// Substitute into a call-shape plus pattern, protecting backtick
/// literals (SPEC 15.4): mask → `substitute_plus` → unmask, mirroring
/// `substitute_general_plus`. Sentinels are ident-shaped and never equal
/// a declared name, so substitution cannot touch them; the restored
/// literal then emits verbatim. All call/method-call plus sites
/// (normal, `# repeat` fold, nested fold) must use this, never raw
/// `substitute_plus` on backticked text.
pub fn substitute_call_plus(
    plus_raw: &str,
    bindings: &HashMap<String, String>,
    ellipses: &[String],
    sigiled: bool,
) -> String {
    let (masked, sentinels) = match rsmpl_pattern::mask_literal_spans(plus_raw) {
        Ok(v) => v,
        // Unbalanced backticks are rejected at parse time; fall back to
        // raw substitution rather than failing the whole rewrite here.
        Err(_) => return substitute_plus(plus_raw, bindings, ellipses, sigiled),
    };
    let out = substitute_plus(&masked, bindings, ellipses, sigiled);
    rsmpl_pattern::unmask_literal_spans(&out, &sentinels)
}

fn replace_ellipsis_tokens(text: &str, replacements: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut n = 0;
    while i < bytes.len() {
        // WI-5b-follow: `...` inside strings, chars, or comments is concrete
        // text (same discipline as the general path) — copy it verbatim.
        let c = bytes[i] as char;
        if c == '"' {
            match rsmpl_pattern::string_end(text, i) {
                Some(end) => {
                    out.push_str(&text[i..end]);
                    i = end;
                    continue;
                }
                None => {
                    out.push_str(&text[i..]);
                    break;
                }
            }
        }
        if c == '\'' {
            if let Some(len) = rsmpl_pattern::char_lit_len(text, i) {
                out.push_str(&text[i..i + len]);
                i += len;
                continue;
            }
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let rest = &text[i..];
            let end = rest.find('\n').map(|p| i + p).unwrap_or(text.len());
            out.push_str(&text[i..end]);
            i = end;
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            if let Some(rel) = text[i + 2..].find("*/") {
                out.push_str(&text[i..i + 2 + rel + 2]);
                i += 2 + rel + 2;
                continue;
            }
            out.push_str(&text[i..]);
            break;
        }
        if i + 2 < bytes.len() && &text[i..i + 3] == "..." {
            let before_ok = i == 0 || bytes[i - 1] != b'.';
            let after_ok = i + 3 >= bytes.len() || bytes[i + 3] != b'.';
            if before_ok && after_ok {
                let rep = replacements.get(n).map(String::as_str).unwrap_or("");
                n += 1;
                if rep.is_empty() {
                    // Swallow one adjacent comma (preceding preferred) so
                    // empty replays leave no `, )` / `( ,` artifact.
                    if let Some(pos) = last_comma_before(&out) {
                        out.truncate(pos);
                    } else {
                        i += 3;
                        let mut j = i;
                        while j < bytes.len()
                            && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n')
                        {
                            j += 1;
                        }
                        if j < bytes.len() && bytes[j] == b',' {
                            j += 1;
                            while j < bytes.len()
                                && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n')
                            {
                                j += 1;
                            }
                        }
                        i = j;
                        continue;
                    }
                } else {
                    out.push_str(rep);
                }
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Byte index of the trailing comma (skipping trailing whitespace) in
/// `out`, or `None` when the pre-ellipsis text does not end with one.
fn last_comma_before(out: &str) -> Option<usize> {
    let bytes = out.as_bytes();
    let mut j = bytes.len();
    while j > 0 && (bytes[j - 1] == b' ' || bytes[j - 1] == b'\t' || bytes[j - 1] == b'\n') {
        j -= 1;
    }
    (j > 0 && bytes[j - 1] == b',').then_some(j - 1)
}

/// Count standalone `...` tokens (same definition as the substitution
/// pass: not part of longer dot runs).
pub(crate) fn count_ellipsis_tokens(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        // WI-5b-follow: strings, chars, and comments hold no ellipsis tokens.
        let c = bytes[i] as char;
        if c == '"' {
            match rsmpl_pattern::string_end(text, i) {
                Some(end) => {
                    i = end;
                    continue;
                }
                None => break,
            }
        }
        if c == '\'' {
            if let Some(len) = rsmpl_pattern::char_lit_len(text, i) {
                i += len;
                continue;
            }
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let rest = &text[i..];
            i = rest.find('\n').map(|p| i + p).unwrap_or(text.len());
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            if let Some(rel) = text[i + 2..].find("*/") {
                i += 2 + rel + 2;
                continue;
            }
            break;
        }
        if i + 2 < bytes.len() && &text[i..i + 3] == "..." {
            let before_ok = i == 0 || bytes[i - 1] != b'.';
            let after_ok = i + 3 >= bytes.len() || bytes[i + 3] != b'.';
            if before_ok && after_ok {
                count += 1;
                i += 3;
                continue;
            }
        }
        i += 1;
    }
    count
}

/// True when the text uses `::...::` path ellipsis (SPEC 16.5), as opposed
/// to argument/body ellipsis `foo(...)`. Local copy of the pattern-crate
/// predicate (which is private there).
pub(crate) fn plus_has_path_ellipsis(text: &str) -> bool {
    // WI-5b-follow: path `...` inside strings/comments is concrete text.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '"' {
            match rsmpl_pattern::string_end(text, i) {
                Some(end) => {
                    i = end;
                    continue;
                }
                None => {
                    let rest = &text[i..];
                    i = rest.find('\n').map(|p| i + p).unwrap_or(text.len());
                    continue;
                }
            }
        }
        if c == '\'' {
            if let Some(len) = rsmpl_pattern::char_lit_len(text, i) {
                i += len;
                continue;
            }
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let rest = &text[i..];
            i = rest.find('\n').map(|p| i + p).unwrap_or(text.len());
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            if let Some(rel) = text[i + 2..].find("*/") {
                i += 2 + rel + 2;
                continue;
            }
            return false;
        }
        if text[i..].starts_with("::...") || text[i..].starts_with("...::") {
            return true;
        }
        i += 1;
    }
    false
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn replace_ident_token(text: &str, name: &str, replacement: &str) -> String {
    if name.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if let Some(rest) = text[i..].strip_prefix(name) {
            let before_ok = i == 0 || !is_ident_char(text[..i].chars().last().unwrap_or(' '));
            let after_ok = rest
                .chars()
                .next()
                .is_none_or(|c| !is_ident_char(c) && c != '$');
            // For `$Name` patterns the char after is part of matching.
            if before_ok && after_ok {
                out.push_str(replacement);
                i += name.len();
                continue;
            }
        }
        out.push(text[i..].chars().next().unwrap());
        i += text[i..].chars().next().unwrap().len_utf8();
    }
    out
}
