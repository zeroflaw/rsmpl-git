//! Concrete unified-diff generation (SPEC 26.1-26.2).
//!
//! The lowered patch must contain no RSmPL directives (guaranteed: it is
//! generated from materialized file text, never from the input patch).

/// Build a `diff --git` section for one file, or `None` when unchanged.
pub fn unified_diff_for_file(path: &str, old: &str, new: &str) -> Option<String> {
    if old == new {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("diff --git a/{path} b/{path}\n"));
    out.push_str(&format!("--- a/{path}\n"));
    out.push_str(&format!("+++ b/{path}\n"));
    let diff = similar::TextDiff::from_lines(old, new);
    // Portable `@@ -a,b +c,d @@` hunks with 3 context lines (unified format).
    // `---`/`+++` headers are emitted above (a/ b/ style, git-compatible).
    let hunks = diff.unified_diff().context_radius(3).to_string();
    out.push_str(&hunks);
    Some(out)
}

/// Assemble per-file sections into one patch document.
pub fn assemble_patch(sections: Vec<String>) -> String {
    let mut out = String::new();
    for s in sections {
        out.push_str(&s);
        if !s.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Count `(added, removed)` lines between old and new file contents.
///
/// Counts come from the line diff itself (not the rendered unified hunks),
/// so they are independent of `context_radius` and stay exact even when the
/// reported diff string is truncated or omitted for token budget (SPEC D.2.1).
pub fn file_line_stats(old: &str, new: &str) -> (usize, usize) {
    if old == new {
        return (0, 0);
    }
    let diff = similar::TextDiff::from_lines(old, new);
    let mut added = 0;
    let mut removed = 0;
    for op in diff.ops() {
        match op {
            similar::DiffOp::Equal { .. } => {}
            similar::DiffOp::Delete { old_len, .. } => removed += old_len,
            similar::DiffOp::Insert { new_len, .. } => added += new_len,
            similar::DiffOp::Replace {
                old_len, new_len, ..
            } => {
                removed += old_len;
                added += new_len;
            }
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_git_sections_without_directives() {
        let d = unified_diff_for_file("src/lib.rs", "fn foo() {}\n", "fn bar() {}\n").unwrap();
        assert!(d.contains("diff --git a/src/lib.rs b/src/lib.rs"));
        assert!(d.contains("--- a/src/lib.rs"));
        assert!(d.contains("+++ b/src/lib.rs"));
        assert!(d.contains("@@"));
        assert!(!d.contains("# lang=rust"));
    }

    #[test]
    fn unchanged_returns_none() {
        assert!(unified_diff_for_file("x", "a\n", "a\n").is_none());
    }

    #[test]
    fn stats_count_single_line_rewrite() {
        assert_eq!(file_line_stats("fn foo() {}\n", "fn bar() {}\n"), (1, 1));
    }

    #[test]
    fn stats_count_pure_insertion() {
        assert_eq!(file_line_stats("a\n", "a\nb\nc\n"), (2, 0));
    }

    #[test]
    fn stats_count_pure_deletion() {
        assert_eq!(file_line_stats("a\nb\nc\n", "a\n"), (0, 2));
    }

    #[test]
    fn stats_unchanged_is_zero() {
        assert_eq!(file_line_stats("a\n", "a\n"), (0, 0));
    }

    #[test]
    fn stats_total_across_hunks() {
        let old = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
        let new = "L1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nL10\n";
        assert_eq!(file_line_stats(old, new), (2, 2));
    }
}
