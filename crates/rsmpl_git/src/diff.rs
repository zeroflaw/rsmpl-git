//! Concrete unified-diff generation (SPEC 26.1-26.2).
//!
//! The lowered patch must contain no RSmPL directives (guaranteed: it is
//! generated from materialized file text, never from the input patch).

/// Quote one `diff --git`/`---`/`+++` header slot (T398): control
/// chars, `"` and `\` must be C-quoted (bare git cannot parse them
/// raw — it never emits them raw either); spaces stay raw (T379).
fn quoted_header(prefix: &str, path: &str) -> String {
    rsmpl_patch::quote_git_path(&format!("{prefix}/{path}"))
}

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
    let (a_slot, b_slot) = (quoted_header("a", path), quoted_header("b", path));
    out.push_str(&format!("diff --git {a_slot} {b_slot}\n"));
    out.push_str(&format!("--- {a_slot}\n"));
    out.push_str(&format!("+++ {b_slot}\n"));
    let diff = similar::TextDiff::from_lines(old, new);
    // Portable `@@ -a,b +c,d @@` hunks with N context lines (unified format).
    // `---`/`+++` headers are emitted above (a/ b/ style, git-compatible).
    let hunks = diff.unified_diff().context_radius(context).to_string();
    out.push_str(&hunks);
    Some(out)
}

/// Build a `diff --git` section for a newly created file.
///
/// Emits `new file mode {mode}` headers (+ `--- /dev/null` and hunks
/// when `new` is non-empty) so the lowered patch applies with a bare
/// mode (`100644` default; `120000` for symlink creates -- T355;
/// mode (`100644` default; `120000` for symlink creates -- T355;
/// `100755` for executables); only use this when the target did not
/// exist before the patch; an existing-but-empty file keeps the
/// ordinary `--- a/...` headers. An empty `new` emits headers only
/// (exactly what real git shows for an empty new file -- T364):
/// `git apply` creates it, while the old `None` silently dropped
/// the create.
pub fn unified_diff_for_new_file_with_context(
    path: &str,
    new: &str,
    context: usize,
    mode: &str,
) -> Option<String> {
    let mut out = String::new();
    let (a_slot, b_slot) = (quoted_header("a", path), quoted_header("b", path));
    out.push_str(&format!("diff --git {a_slot} {b_slot}\n"));
    out.push_str(&format!("new file mode {mode}\n"));
    if new.is_empty() {
        return Some(out);
    }
    out.push_str("--- /dev/null\n");
    out.push_str(&format!("+++ {b_slot}\n"));
    let diff = similar::TextDiff::from_lines("", new);
    let hunks = diff.unified_diff().context_radius(context).to_string();
    out.push_str(&hunks);
    Some(out)
}

/// Emits `deleted file mode 100644` (regular files) or `deleted file
/// mode 120000` (symlinks) headers (+ `--- a/...`, `+++ /dev/null`,
/// and hunks when `old` is non-empty) so the lowered patch removes the
/// path with a bare `git apply` instead of truncating it to empty
/// (T350). `git apply` type-checks the mode line, so a symlink
/// deleted as 100644 fails with `wrong type` (T353 history dogfood:
/// 53372c4). Callers must only use this when the input section targets
/// `/dev/null` and the engine emptied the content; an ordinary
/// all-minus hunk truncates the file and keeps it. An empty `old`
/// emits headers only (exactly what real git shows for deleting an
/// empty file -- T364): `git apply` removes it, while the old `None`
/// silently kept it.
pub fn unified_diff_for_deleted_file_with_context(
    path: &str,
    old: &str,
    context: usize,
    symlink: bool,
) -> Option<String> {
    let mut out = String::new();
    let (a_slot, b_slot) = (quoted_header("a", path), quoted_header("b", path));
    out.push_str(&format!("diff --git {a_slot} {b_slot}\n"));
    out.push_str(if symlink {
        "deleted file mode 120000\n"
    } else {
        "deleted file mode 100644\n"
    });
    if old.is_empty() {
        return Some(out);
    }
    out.push_str(&format!("--- {a_slot}\n"));
    out.push_str("+++ /dev/null\n");
    let diff = similar::TextDiff::from_lines(old, "");
    let hunks = diff.unified_diff().context_radius(context).to_string();
    out.push_str(&hunks);
    Some(out)
}

/// Build a `diff --git` delete+create pair for a file↔symlink typechange.
///
/// Git represents a typechange as two same-path sections: a delete half
/// (`deleted file mode <old>`) plus a create half (`new file mode <new>`,
/// T357). `old_symlink` selects the delete mode line (120000 vs 100644,
/// T353); `new_mode` is the declared create mode (120000 vs 100644,
/// T355 default). Empty-side halves emit headers only (the new-file and
/// delete helpers never return `None` anymore -- T364), so the caller
pub fn unified_diff_for_typechange_with_context(
    path: &str,
    old: &str,
    new: &str,
    old_symlink: bool,
    new_mode: &str,
    context: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(s) = unified_diff_for_deleted_file_with_context(path, old, context, old_symlink) {
        out.push(s);
    }
    if let Some(s) = unified_diff_for_new_file_with_context(path, new, context, new_mode) {
        out.push(s);
    }
    out
}

/// Build a headers-only mode-flip section (`old mode` + `new mode`, no
/// hunks) for a tracked file whose content is unchanged. Exactly what
/// real git emits for a pure mode change -- T364: `git apply` flips the
/// mode, while content-only lowering silently dropped it.
pub fn mode_change_section(path: &str, old_mode: &str, new_mode: &str) -> String {
    let (a_slot, b_slot) = (quoted_header("a", path), quoted_header("b", path));
    format!("diff --git {a_slot} {b_slot}\nold mode {old_mode}\nnew mode {new_mode}\n")
}

/// Splice `old mode`/`new mode` lines into a lowered content section
/// (right after the `diff --git` header, where git puts them) so a
/// content change that also flips the mode keeps both -- T364. The
/// input section must actually declare both modes (equal or absent
/// means content-only, today's behavior).
pub fn with_mode_lines(section: &str, old_mode: &str, new_mode: &str) -> String {
    if old_mode == new_mode {
        return section.to_string();
    }
    // T402: splice after the first `\n` without re-splitting the
    // body. `str::lines` strips a trailing `\r` from every line,
    // which silently corrupted CRLF content on real-apply (bare
    // git preserves it). Byte-identical for LF inputs.
    match section.find('\n') {
        Some(i) => format!(
            "{}old mode {old_mode}\nnew mode {new_mode}\n{}",
            &section[..=i],
            &section[i + 1..]
        ),
        None => format!("{section}\nold mode {old_mode}\nnew mode {new_mode}\n"),
    }
}

/// Build a `diff --git` section for a file rename (`old_rel` -> `new_rel`).
///
/// Emits `rename from/to` headers (+ `--- a/old` / `+++ b/new` and
/// 0-context hunks when the content differs; headers only for a pure
/// rename -- exactly what real git shows -- T377). `similarity index`
/// and `index` lines are omitted: `git apply` needs neither, so the
/// lowering stays minimal (same doctrine as the T343/T374 strips).
/// `old_mode`/`new_mode` carry a mode flip declared by the input
/// section (both `Some` and differing); otherwise no mode lines.
/// Always returns `Some` -- even a content-equal rename must lower
/// (to the headers-only shape), never silently drop.
#[allow(clippy::too_many_arguments)]
pub fn unified_diff_for_rename_with_context(
    old_rel: &str,
    new_rel: &str,
    old: &str,
    new: &str,
    old_mode: Option<&str>,
    new_mode: Option<&str>,
    context: usize,
) -> String {
    let mut out = String::new();
    let (a_old, b_new) = (quoted_header("a", old_rel), quoted_header("b", new_rel));
    let q_old = rsmpl_patch::quote_git_path(old_rel);
    let q_new = rsmpl_patch::quote_git_path(new_rel);
    out.push_str(&format!("diff --git {a_old} {b_new}\n"));
    out.push_str(&format!("rename from {q_old}\n"));
    out.push_str(&format!("rename to {q_new}\n"));
    if let (Some(o), Some(n)) = (old_mode, new_mode) {
        if o != n {
            out.push_str(&format!("old mode {o}\nnew mode {n}\n"));
        }
    }
    if old == new {
        return out;
    }
    out.push_str(&format!("--- {a_old}\n"));
    out.push_str(&format!("+++ {b_new}\n"));
    let diff = similar::TextDiff::from_lines(old, new);
    let hunks = diff.unified_diff().context_radius(context).to_string();
    out.push_str(&hunks);
    out
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
    fn control_chars_quote_but_spaces_and_unicode_stay_raw() {
        // T398: bare git cannot parse raw control chars in unquoted
        // headers (it never emits them raw), so the lowering C-quotes
        // those slots; spaces + non-ASCII keep the native raw shape.
        let d = unified_diff_for_file_with_context("tab\tname.txt", "a\n", "b\n", 0).unwrap();
        assert!(
            d.contains("diff --git \"a/tab\\tname.txt\" \"b/tab\\tname.txt\""),
            "got: {d}"
        );
        assert!(d.contains("--- \"a/tab\\tname.txt\""), "got: {d}");
        let s = unified_diff_for_file_with_context("my file.txt", "a\n", "b\n", 0).unwrap();
        assert!(
            s.contains("diff --git a/my file.txt b/my file.txt"),
            "got: {s}"
        );
        let u = unified_diff_for_file_with_context("café.txt", "a\n", "b\n", 0).unwrap();
        assert!(u.contains("diff --git a/café.txt b/café.txt"), "got: {u}");
        // Quoted slots re-parse to the same target.
        let p = rsmpl_patch::parse_patch(&d).unwrap();
        assert_eq!(p.files[0].target_path, "tab\tname.txt");
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
        let d =
            unified_diff_for_new_file_with_context("src/new.rs", "use x;\n", 3, "100644").unwrap();
        assert!(d.contains("new file mode"), "got: {d}");
        assert!(d.contains("--- /dev/null"), "got: {d}");
        assert!(d.contains("+++ b/src/new.rs"), "got: {d}");
        assert!(!d.contains("--- a/"), "got: {d}");
        assert!(d.contains("+use x;"), "got: {d}");
    }

    #[test]
    fn new_file_empty_returns_none() {
        // T364: an empty create emits headers only (exactly what real
        // git shows) instead of `None` -- the old `None` silently
        // dropped the create.
        let d = unified_diff_for_new_file_with_context("x", "", 3, "100644").unwrap();
        assert!(d.contains("new file mode 100644"), "got: {d}");
        assert!(!d.contains("@@"), "got: {d}");
        assert!(!d.contains("--- /dev/null"), "got: {d}");
    }

    #[test]
    fn deleted_file_section_uses_dev_null_headers() {
        // T350: deleted files lower with `deleted file mode` so a bare
        // `git apply` removes them (plain `+++ b/…` truncates instead).
        let d =
            unified_diff_for_deleted_file_with_context("src/old.rs", "use x;\n", 3, false).unwrap();
        assert!(d.contains("deleted file mode"), "got: {d}");
        assert!(d.contains("+++ /dev/null"), "got: {d}");
        assert!(d.contains("--- a/src/old.rs"), "got: {d}");
        assert!(!d.contains("+++ b/"), "got: {d}");
        assert!(d.contains("-use x;"), "got: {d}");
    }

    #[test]
    fn deleted_file_empty_returns_none() {
        // T364: deleting an empty file emits headers only (exactly
        // what real git shows) instead of `None` -- the old `None`
        // silently kept the file.
        let d = unified_diff_for_deleted_file_with_context("x", "", 3, false).unwrap();
        assert!(d.contains("deleted file mode 100644"), "got: {d}");
        assert!(!d.contains("@@"), "got: {d}");
    }

    #[test]
    fn deleted_symlink_section_uses_120000_mode() {
        // T353: `git apply` type-checks the mode line, so symlink
        // deletes lower as 120000 (100644 fails `wrong type`).
        let d = unified_diff_for_deleted_file_with_context("link", "target\n", 0, true).unwrap();
        assert!(d.contains("deleted file mode 120000"), "got: {d}");
        assert!(!d.contains("100644"), "got: {d}");
    }

    #[test]
    fn typechange_file_to_symlink_emits_delete_plus_120000_create() {
        // T357: git's file->symlink shape is a delete+create pair.
        let pair =
            unified_diff_for_typechange_with_context("f", "hello\n", "target", false, "120000", 0);
        assert_eq!(pair.len(), 2, "got: {pair:?}");
        assert!(
            pair[0].contains("deleted file mode 100644"),
            "got: {}",
            pair[0]
        );
        assert!(pair[0].contains("-hello"), "got: {}", pair[0]);
        assert!(pair[1].contains("new file mode 120000"), "got: {}", pair[1]);
        assert!(pair[1].contains("+target"), "got: {}", pair[1]);
        assert!(
            pair[1].contains("No newline at end of file"),
            "got: {}",
            pair[1]
        );
    }

    #[test]
    fn typechange_symlink_to_file_emits_120000_delete_plus_create() {
        // T357 reverse direction: 120000 delete + 100644 create.
        let pair =
            unified_diff_for_typechange_with_context("f", "target", "hello\n", true, "100644", 0);
        assert_eq!(pair.len(), 2, "got: {pair:?}");
        assert!(
            pair[0].contains("deleted file mode 120000"),
            "got: {}",
            pair[0]
        );
        assert!(pair[1].contains("new file mode 100644"), "got: {}", pair[1]);
        assert!(pair[1].contains("+hello"), "got: {}", pair[1]);
    }

    #[test]
    fn new_symlink_section_uses_120000_and_no_newline_marker() {
        // T355: symlink creates lower as 120000 with the link-target
        // string (no trailing newline), matching `git diff` for a
        // staged symlink (T354).
        let d = unified_diff_for_new_file_with_context("link", "real.txt", 0, "120000").unwrap();
        assert!(d.contains("new file mode 120000"), "got: {d}");
        assert!(d.contains("+real.txt"), "got: {d}");
        assert!(d.contains("No newline at end of file"), "got: {d}");
        assert!(!d.contains("100644"), "got: {d}");
    }

    #[test]
    fn mode_change_section_is_headers_only() {
        // T364: a pure mode change lowers to the exact shape real git
        // emits (no hunks) -- `git apply` flips the mode.
        let d = mode_change_section("run.sh", "100755", "100644");
        assert!(d.contains("old mode 100755"), "got: {d}");
        assert!(d.contains("new mode 100644"), "got: {d}");
        assert!(!d.contains("@@"), "got: {d}");
        assert!(!d.contains("--- "), "got: {d}");
    }

    #[test]
    fn with_mode_lines_splices_after_diff_git_header() {
        // T364: content+mode changes keep both -- mode lines land
        // where git puts them (right after `diff --git`).
        let section = unified_diff_for_file_with_context("f", "a\n", "b\n", 0).unwrap();
        let d = with_mode_lines(&section, "100644", "100755");
        let mut lines = d.lines();
        assert_eq!(lines.next(), Some("diff --git a/f b/f"));
        assert_eq!(lines.next(), Some("old mode 100644"));
        assert_eq!(lines.next(), Some("new mode 100755"));
        assert!(d.contains("--- a/f"), "got: {d}");
    }

    #[test]
    fn with_mode_lines_equal_modes_is_identity() {
        // Equal modes mean content-only, no-op.
        let section = unified_diff_for_file_with_context("f", "a\n", "b\n", 0).unwrap();
        assert_eq!(with_mode_lines(&section, "100644", "100644"), section);
    }

    #[test]
    fn rename_with_content_change_emits_rename_headers_and_hunks() {
        // T377: rename+modify lowers to rename headers + ---/+++ +
        // hunks, with no similarity/index lines (git apply needs
        // neither) -- the minimal appliable shape.
        let d = unified_diff_for_rename_with_context(
            "old.txt",
            "new.txt",
            "a\n",
            "b\n",
            None,
            None,
            0,
        );
        assert!(d.contains("diff --git a/old.txt b/new.txt"), "got: {d}");
        assert!(d.contains("rename from old.txt"), "got: {d}");
        assert!(d.contains("rename to new.txt"), "got: {d}");
        assert!(d.contains("--- a/old.txt"), "got: {d}");
        assert!(d.contains("+++ b/new.txt"), "got: {d}");
        assert!(d.contains("-a\n") && d.contains("+b\n"), "got: {d}");
        assert!(!d.contains("similarity"), "got: {d}");
        assert!(!d.contains("index "), "got: {d}");
    }

    #[test]
    fn rename_pure_is_headers_only() {
        // T377: a pure rename lowers to headers only, exactly what
        // real git shows -- and never to an empty string.
        let d = unified_diff_for_rename_with_context(
            "old.txt",
            "new.txt",
            "same\n",
            "same\n",
            None,
            None,
            0,
        );
        assert!(d.contains("rename from old.txt"), "got: {d}");
        assert!(d.contains("rename to new.txt"), "got: {d}");
        assert!(!d.contains("@@"), "got: {d}");
        assert!(!d.contains("--- "), "got: {d}");
    }

    #[test]
    fn rename_carries_mode_flip() {
        // T377: a rename that also flips the mode keeps both.
        let d = unified_diff_for_rename_with_context(
            "old.sh",
            "new.sh",
            "x\n",
            "x\n",
            Some("100644"),
            Some("100755"),
            0,
        );
        assert!(d.contains("old mode 100644"), "got: {d}");
        assert!(d.contains("new mode 100755"), "got: {d}");
        assert!(!d.contains("@@"), "got: {d}");
    }

    #[test]
    fn typechange_empty_sides_emit_headers_only_pair() {        // T364: empty halves no longer drop -- the pair stays complete
        // (delete headers + create headers), in git order.
        let pair = unified_diff_for_typechange_with_context("f", "", "", false, "120000", 0);
        assert_eq!(pair.len(), 2, "got: {pair:?}");
        assert!(
            pair[0].contains("deleted file mode 100644"),
            "got: {}",
            pair[0]
        );
        assert!(!pair[0].contains("@@"), "got: {}", pair[0]);
        assert!(pair[1].contains("new file mode 120000"), "got: {}", pair[1]);
        assert!(!pair[1].contains("@@"), "got: {}", pair[1]);
    }

    #[test]
    fn with_mode_lines_preserves_cr_byte_exact() {
        // T402: mode+content over CRLF must keep the `\r`
        // byte-exact (bare git preserves it; the old
        // `lines()`-based splice silently stripped it,
        // corrupting real-apply while `--check` stayed green).
        let section = unified_diff_for_file_with_context("f", "a\n", "b\r\n", 0).unwrap();
        assert!(section.contains("+b\r\n"), "got: {section:?}");
        let d = with_mode_lines(&section, "100644", "100755");
        assert!(d.contains("+b\r\n"), "got: {d:?}");
        assert!(d.contains("old mode 100644"), "got: {d:?}");
        assert!(d.contains("new mode 100755"), "got: {d:?}");
    }
}
