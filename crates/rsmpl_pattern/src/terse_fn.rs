//! WI-1b: terse fn rename `- fn foo` / `+ fn bar` (SPEC 14.1 diet form).
//!
//! A terse fn pattern is a bare fn-name line with no slot or body text.
//! Params, return type, generics and where clause are unconstrained
//! (matched loosely, preserved byte-for-byte); bodies are never
//! rewritten. Any visibility/qualifier tokens written before `fn` keep
//! matching literally. Any explicit slot text (`(...)`, `<...>`,
//! `-> T`, `where`, `{`, `;`, `...`) keeps the long-form path — this
//! module only builds the all-`Any` pattern once the caller has
//! established that no tokens remain after the fn name.

use super::fn_pattern::{FnPattern, GenericPat, NamePat, RetPat, VisPat, WherePat};

/// Build the rename-only pattern for a terse fn line.
///
/// `visibility`/`qualifiers`/`qualifier_meta`/`name` are the already-parsed
/// prefix (same parsing as the long form, so `pub`, `pub(crate)`,
/// `async`/`const`/`unsafe`/`extern`, `V`/`Q` metas all behave identically);
/// every value slot becomes unconstrained and both ellipses hold so the
/// engine's body gate and slot planner preserve the target verbatim.
pub(crate) fn terse_pattern(
    attrs_written: Vec<String>,
    visibility: VisPat,
    qualifiers: Vec<String>,
    qualifier_meta: Option<String>,
    name: NamePat,
    returns_none: bool,
) -> FnPattern {
    FnPattern {
        attrs_written,
        visibility,
        qualifiers,
        qualifier_meta,
        name,
        generics: GenericPat::Any,
        params_ellipsis: true,
        params_text: None,
        ret: RetPat::Any,
        where_clause: WherePat::Any,
        body_ellipsis: true,
        returns_none_directive: returns_none,
    }
}

/// True when a pattern is terse-shaped (all value slots `Any` with no
/// concrete param text). The long form never produces `Any` generics, so
/// this discriminates terse from long without extra plumbing.
pub(crate) fn is_terse(pattern: &FnPattern) -> bool {
    matches!(pattern.generics, GenericPat::Any)
        && matches!(pattern.where_clause, WherePat::Any)
        && matches!(pattern.ret, RetPat::Any)
        && pattern.params_ellipsis
        && pattern.params_text.is_none()
}

/// Heuristic for the impl-context block slicer: a fn-start line with no
/// `(` outside a leading `pub(...)` vis is terse (or garbage) — either way
/// the single line is the whole pattern and trailing context lines such as
/// the closing `}` must not be swallowed. Lines carrying `(` keep the
/// existing brace/`;` balancing path.
pub fn is_terse_fn_line(line: &str) -> bool {
    let t = line.trim();
    if !t.contains("fn ") {
        return false;
    }
    let rest = strip_pub_paren(t);
    !rest.contains('(')
}

/// True when the tokens after a bare fn name are only body-ish residue
/// (`...`, `{`, `}`, `;`) — i.e. the author wrote a terse fn line plus
/// body/context lines. Neither terse (bare pair only) nor long (needs
/// `(...)`): the caller fails fast with the terse needle instead of the
/// misleading missing-parens error.
pub(crate) fn is_stray_body_tail(rest: &[String]) -> bool {
    !rest.is_empty()
        && rest
            .iter()
            .all(|t| matches!(t.as_str(), "..." | "{" | "}" | ";"))
}

/// Strip one leading `pub(...)` visibility (balanced) so its parens do not
/// read as a parameter list. Returns the input unchanged when it does not
/// start with `pub(` or the parens do not balance.
fn strip_pub_paren(t: &str) -> &str {
    let rest = match t.strip_prefix("pub(") {
        Some(r) => r,
        None => return t,
    };
    let mut depth = 1i32;
    for (idx, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return t[idx + 4..].trim_start();
                }
            }
            _ => {}
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::{is_terse, is_terse_fn_line, terse_pattern};
    use crate::fn_pattern::{NamePat, VisPat};

    fn plain(name: &str) -> crate::fn_pattern::FnPattern {
        terse_pattern(
            Vec::new(),
            VisPat::Absent,
            Vec::new(),
            None,
            NamePat::Concrete(name.into()),
            false,
        )
    }

    #[test]
    fn terse_pattern_is_all_any() {
        // WI-1b: the constructor leaves every value slot unconstrained and
        // both ellipses holding so matching preserves the target verbatim.
        let p = plain("foo");
        assert!(is_terse(&p));
        assert!(p.body_ellipsis);
        // Long-form shapes never produce `Any` generics.
        let long =
            crate::fn_pattern::parse_fn_pattern("fn foo(...) { ... }", &[], &[], &[], &[], false)
                .unwrap();
        assert!(!is_terse(&long));
    }

    #[test]
    fn terse_line_heuristic() {
        // WI-1b: bare fn lines (any vis/qual prefix) slice as one line;
        // anything with a parameter list keeps the balancing path.
        assert!(is_terse_fn_line("fn foo"));
        assert!(is_terse_fn_line("pub async fn foo"));
        assert!(is_terse_fn_line("pub(crate) fn foo"));
        assert!(!is_terse_fn_line("fn foo(...) { ... }"));
        assert!(!is_terse_fn_line("fn foo<T>(...) { ... }"));
        assert!(!is_terse_fn_line("impl Service {"));
    }

    #[test]
    fn parse_bare_fn_lines_end_to_end() {
        // WI-1b: `parse_fn_pattern` returns the terse pattern once no tokens
        // remain after the name; any slot text keeps the long form.
        for shape in ["fn foo", "pub fn foo", "pub(crate) async unsafe fn foo"] {
            let p = crate::fn_pattern::parse_fn_pattern(shape, &[], &[], &[], &[], false).unwrap();
            assert!(is_terse(&p), "unexpected: {shape}");
        }
        // Slot/body text still routes long (and then enforces long rules —
        // `fn foo()` parses as a concrete-empty-params long shape, never terse).
        assert!(
            crate::fn_pattern::parse_fn_pattern("fn foo(...) { ... }", &[], &[], &[], &[], false)
                .unwrap()
                .generics
                != crate::fn_pattern::GenericPat::Any
        );
        assert!(!is_terse(
            &crate::fn_pattern::parse_fn_pattern("fn foo()", &[], &[], &[], &[], false).unwrap()
        ));
        // Leading attribute lines peel before the terse check, so an
        // attributed bare line is still terse (attrs preserved by default).
        let attr =
            crate::fn_pattern::parse_fn_pattern("#[inline]\nfn foo", &[], &[], &[], &[], false)
                .unwrap();
        assert!(is_terse(&attr));
        assert_eq!(attr.attrs_written.len(), 1);
    }

    #[test]
    fn stray_body_tail_needle() {
        // WI-1b: a bare fn line plus body/context lines is neither terse
        // nor long — the needle names the fix, not the missing parens.
        for shape in ["fn foo { ... }", "fn foo\n...\n}", "fn foo;"] {
            let err = crate::fn_pattern::parse_fn_pattern(shape, &[], &[], &[], &[], false)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("terse fn rename takes no body lines"),
                "unexpected: {err}"
            );
        }
    }
}
