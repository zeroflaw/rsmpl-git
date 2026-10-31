//! Bracket/brace macro-argument extraction (WI-7a). Split from
//! `infer.rs` per the D5 hygiene gate: the call-slot pass needs the
//! inner text of `name![...]` / `name!{...}` invocations (the paren
//! path never sees these lines since there is no `(` to split on).

/// Inner text of the first bracket/brace macro invocation (`name![...]`,
/// `name!{...}`) on the line: the `!` whose next non-space char opens a
/// `[`/`{` group, with the balanced interior (nesting-aware over all
/// delimiter kinds, string-aware). Paren macros (`!(...)`) return `None`
/// — the paren path owns them. `!=` / `!cond` never match.
pub fn macro_bracket_args(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut si = 0;
    while si < bytes.len() {
        let rel = s[si..].find('!')?;
        let bang = si + rel;
        let mut k = bang + 1;
        while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
            k += 1;
        }
        let (open, close) = match bytes.get(k) {
            Some(b'[') => (b'[', b']'),
            Some(b'{') => (b'{', b'}'),
            _ => {
                si = bang + 1;
                continue;
            }
        };
        let mut depth = 0;
        let mut in_str = false;
        let mut esc = false;
        let mut i = k;
        while i < bytes.len() {
            let b = bytes[i];
            if in_str {
                if esc {
                    esc = false;
                } else if b == b'\\' {
                    esc = true;
                } else if b == b'"' {
                    in_str = false;
                }
            } else if b == b'"' {
                in_str = true;
            } else if b == open || b == b'(' || b == b'[' || b == b'{' {
                depth += 1;
            } else if b == close || b == b')' || b == b']' || b == b'}' {
                depth -= 1;
                if depth == 0 {
                    return Some(s[k + 1..i].to_string());
                }
            }
            i += 1;
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::macro_bracket_args;

    #[test]
    fn bracket_and_brace_fire() {
        assert_eq!(macro_bracket_args("my_vec![E, 2]").as_deref(), Some("E, 2"));
        assert_eq!(macro_bracket_args("mac!{a, b}").as_deref(), Some("a, b"));
    }

    #[test]
    fn paren_and_non_macro_decline() {
        assert!(macro_bracket_args("foo!(a, b)").is_none());
        assert!(macro_bracket_args("a != b").is_none());
        assert!(macro_bracket_args("!cond").is_none());
        assert!(macro_bracket_args("plain").is_none());
    }

    #[test]
    fn nesting_and_strings_hold() {
        assert_eq!(
            macro_bracket_args("m![f(a, b), \"x,y\"]").as_deref(),
            Some("f(a, b), \"x,y\"")
        );
    }
}
