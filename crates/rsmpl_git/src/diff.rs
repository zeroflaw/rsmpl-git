//! Concrete unified-diff generation (SPEC 26.1-26.2).
//!
//! The lowered patch must contain no RSmPL directives (guaranteed: it is
//! generated from materialized file text, never from the input patch).

/// Build a `diff --git` section for one file with an explicit unified-diff
/// context radius, or `None` when unchanged.
///
/// `context` is the number of unchanged lines shown around each hunk
/// (`0` omits all context; headers stay `@@ -a,b +c,d @@`). The result
/// remains acceptable to `git apply` for any radius, including `0`.
/// Per-file `added`/`removed` stats (see [`file_line_stats`]) are computed
/// from the line diff itself and are independent of this radius.
pub fn unified_diff_for_file_with_context(
    path: &str,
    old: &str,
    new: &str,
    context: usize,
) -> Option<String> {
    if old == new {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("diff --git a/{path} b/{path}\n"));
    out.push_str(&format!("--- a/{path}\n"));
    out.push_str(&format!("+++ b/{path}\n"));
    let diff = similar::TextDiff::from_lines(old, new);
    // Portable `@@ -a,b +c,d @@` hunks with N context lines (unified format).
    // `---`/`+++` headers are emitted above (a/ b/ style, git-compatible).
    let hunks = diff.unified_diff().context_radius(context).to_string();
    out.push_str(&hunks);
    Some(out)
}

/// Build a `diff --git` section for a newly created file.
///
/// Emits `new file mode 100644` + `--- /dev/null` headers so the lowered
/// patch applies with a bare `git apply` (SPEC 27.6). Callers must only
/// use this when the target did not exist before the patch; an
/// existing-but-empty file keeps the ordinary `--- a/…` headers.
/// Returns `None` when `new` is empty (no changes, nothing to create).
pub fn unified_diff_for_new_file_with_context(
    path: &str,
    new: &str,
    context: usize,
) -> Option<String> {
    if new.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("diff --git a/{path} b/{path}\n"));
    out.push_str("new file mode 100644\n");
    out.push_str("--- /dev/null\n");
    out.push_str(&format!("+++ b/{path}\n"));
    let diff = similar::TextDiff::from_lines("", new);
    let hunks = diff.unified_diff().context_radius(context).to_string();
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
        let d =
            unified_diff_for_file_with_context("src/lib.rs", "fn foo() {}\n", "fn bar() {}\n", 3)
                .unwrap();
        assert!(d.contains("diff --git a/src/lib.rs b/src/lib.rs"));
        assert!(d.contains("--- a/src/lib.rs"));
        assert!(d.contains("+++ b/src/lib.rs"));
        assert!(d.contains("@@"));
        assert!(!d.contains("# lang=rust"));
    }

    #[test]
    fn unchanged_returns_none() {
        assert!(unified_diff_for_file_with_context("x", "a\n", "a\n", 3).is_none());
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

    #[test]
    fn zero_context_drops_unchanged_but_keeps_edits() {
        let old = "l1\nl2\nl3\nl4\nold\nl6\nl7\nl8\nl9\n";
        let new = "l1\nl2\nl3\nl4\nnew\nl6\nl7\nl8\nl9\n";
        let d0 = unified_diff_for_file_with_context("x", old, new, 0).unwrap();
        assert!(d0.contains("-old\n"), "got: {d0}");
        assert!(d0.contains("+new\n"), "got: {d0}");
        assert!(!d0.contains(" l1\n"), "got: {d0}");
        assert!(!d0.contains(" l9\n"), "got: {d0}");
        assert!(d0.contains("@@"), "got: {d0}");
        // Stats are independent of the context radius.
        assert_eq!(file_line_stats(old, new), (1, 1));
    }

    #[test]
    fn smaller_context_shrinks_hunks() {
        let old = "l1\nl2\nl3\nl4\nold\nl6\nl7\nl8\nl9\n";
        let new = "l1\nl2\nl3\nl4\nnew\nl6\nl7\nl8\nl9\n";
        let d3 = unified_diff_for_file_with_context("x", old, new, 3).unwrap();
        let d1 = unified_diff_for_file_with_context("x", old, new, 1).unwrap();
        assert!(
            d1.lines().count() < d3.lines().count(),
            "1-context should be shorter: {} vs {}",
            d1.lines().count(),
            d3.lines().count()
        );
        assert!(d1.contains("-old\n") && d1.contains("+new\n"));
    }

    #[test]
    fn with_context_unchanged_returns_none() {
        assert!(unified_diff_for_file_with_context("x", "a\n", "a\n", 0).is_none());
    }

    #[test]
    fn new_file_section_uses_dev_null_headers() {
        // T98/T125: created files lower with `new file mode` so a bare
        // `git apply` creates them (plain `--- a/…` fails instead).
        let d = unified_diff_for_new_file_with_context("src/new.rs", "use x;\n", 3).unwrap();
        assert!(d.contains("new file mode"), "got: {d}");
        assert!(d.contains("--- /dev/null"), "got: {d}");
        assert!(d.contains("+++ b/src/new.rs"), "got: {d}");
        assert!(!d.contains("--- a/"), "got: {d}");
        assert!(d.contains("+use x;"), "got: {d}");
    }

    #[test]
    fn new_file_empty_returns_none() {
        assert!(unified_diff_for_new_file_with_context("x", "", 3).is_none());
    }
}
