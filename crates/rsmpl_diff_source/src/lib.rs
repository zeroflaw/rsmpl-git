//! Git-facing source for the inverse mapping (`diffrs`).
//!
//! Shells out to external `git` (the T294/T299 decision binds this: no
//! `gix` dependency for a minimal CLI). Collects the pending native
//! diff — tracked modifications via `git diff`, plus untracked files
//! via `git status` (emitted as new-file ordinary sections, since
//! `git diff` never shows them) — for the infer crate to compress.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("cannot run git diff in {repo}: {inner}")]
    Git { repo: String, inner: String },
    #[error("cannot read untracked file {path}: {inner}")]
    Untracked { path: String, inner: String },
}

/// What to collect: `staged` selects `git diff --cached` (index vs
/// `HEAD`/rev); otherwise the worktree-vs-index diff. `rev` forwards a
/// single `git diff [rev]` revision (e.g. `HEAD`, `HEAD~1`) to also
/// cover history with the same mechanism.
pub struct GitSource {
    pub repo: PathBuf,
    pub staged: bool,
    pub rev: Option<String>,
}

/// Raw native diff plus the file list it covers (tracked and untracked).
pub struct CollectedDiff {
    pub raw: String,
    pub files: Vec<String>,
    /// A top-level `.history/` audit log was present but skipped
    /// (warned on stderr, never embedded).
    pub skipped_history: bool,
}

impl GitSource {
    pub fn collect(&self) -> Result<CollectedDiff, SourceError> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.repo)
            .arg("diff")
            .arg("--no-color")
            .arg("--no-ext-diff")
            .arg("--unified=0");
        if self.staged {
            cmd.arg("--cached");
        }
        if let Some(rev) = &self.rev {
            cmd.arg(rev);
        }
        let out = cmd.output().map_err(|e| SourceError::Git {
            repo: self.repo.display().to_string(),
            inner: format!("failed to run `git diff` (is git installed?): {e}"),
        })?;
        if !out.status.success() {
            return Err(SourceError::Git {
                repo: self.repo.display().to_string(),
                inner: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        let mut raw = String::from_utf8_lossy(&out.stdout).to_string();
        // `git diff` detects renames by default (`similarity index`
        // + `rename from/to` headers), which applyrs lowers back to
        // rename sections (T377) -- moves stay moves instead of
        // delete+create pairs. `--unified=0` keeps ordinary passthrough minimal-token:
        // zero-context hunks still round-trip through `applyrs
        // --check`, and inference reads only `-`/`+` lines.

        // Untracked files never appear in `git diff`; emit them as
        // new-file ordinary sections so the patch covers all pending
        // changes (agents create new files constantly).
        let mut untracked = untracked_files(&self.repo).map_err(|e| SourceError::Git {
            repo: self.repo.display().to_string(),
            inner: e,
        })?;
        untracked.sort();
        // `.history/` is the `applyrs` audit log (SPEC D.1): tool
        // output, never a lowering input. Old worktrees (whose
        // `.gitignore` predates the T253 entry) still report it as
        // untracked; embedding it would feed previous patches back
        // into the next conversion, so skip it here (warned by the
        // binary, like stripped binaries).
        let (untracked, skipped): (Vec<String>, Vec<String>) =
            untracked.into_iter().partition(|p| !is_history_path(p));
        let skipped_history = !skipped.is_empty();
        for path in &untracked {
            let abs = self.repo.join(path);
            // Read the link itself, never follow it (T354, mirrors the
            // T353 applyrs read side): a dangling symlink would fail
            // `std::fs::read` with `SourceError` (exit 2) and a
            // link-to-dir with EISDIR; `read_link` serves both.
            let (content, mode) = match std::fs::symlink_metadata(&abs) {
                Ok(md) if md.file_type().is_symlink() => match std::fs::read_link(&abs) {
                    Ok(target) => (target.to_string_lossy().into_owned().into_bytes(), "120000"),
                    Err(e) => {
                        return Err(SourceError::Untracked {
                            path: path.clone(),
                            inner: e.to_string(),
                        });
                    }
                },
                Ok(md) => (
                    std::fs::read(&abs).map_err(|e| SourceError::Untracked {
                        path: path.clone(),
                        inner: e.to_string(),
                    })?,
                    // T364: preserve the exec bit so scripts round-trip
                    // with their mode (the old hardcoded 100644 silently
                    // dropped +x).
                    regular_file_mode(&md),
                ),
                Err(e) => {
                    return Err(SourceError::Untracked {
                        path: path.clone(),
                        inner: e.to_string(),
                    });
                }
            };
            // Binary untracked files cannot ride a UTF-8 patch: emit the
            // same `Binary files … differ` marker section `git diff`
            // shows once staged, so the infer stage strips them with a
            // warning (T378) instead of silently dropping them (T364
            // silent-drop class).
            if content.contains(&0) {
                raw.push_str(&format!(
                    "diff --git a/{path} b/{path}\nBinary files a/{path} and b/{path} differ\n"
                ));
                continue;
            }
            let text = String::from_utf8_lossy(&content);
            raw.push_str(&new_file_section(path, &text, mode));
        }

        let mut files = files_in_diff(&raw);
        for path in untracked {
            if !files.contains(&path) {
                files.push(path);
            }
        }
        Ok(CollectedDiff {
            raw,
            files,
            skipped_history,
        })
    }
}

/// Repo-relative paths of untracked files (`git status --porcelain`).
fn untracked_files(repo: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("status")
        .arg("--porcelain=v1")
        .arg("-uall")
        .output()
        .map_err(|e| format!("failed to run `git status` (is git installed?): {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let mut paths = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // `?? <path>`; renames never appear as untracked.
        if let Some(path) = line.strip_prefix("?? ") {
            paths.push(path.trim_matches('"').to_string());
        }
    }
    Ok(paths)
}

/// Tool-internal audit paths: `applyrs` archives every run under
/// top-level `.history/` (SPEC D.1); those files are never lowering
/// inputs, so the source skips them even when untracked.
fn is_history_path(path: &str) -> bool {
    path == ".history" || path.starts_with(".history/")
}
/// Mode for an untracked regular file (T364): `100755` when any exec
/// bit is set, else `100644`. Unix only -- elsewhere everything is
/// `100644` (no exec-bit concept to preserve).
#[cfg(unix)]
fn regular_file_mode(md: &std::fs::Metadata) -> &'static str {
    use std::os::unix::fs::PermissionsExt;
    if md.permissions().mode() & 0o111 != 0 {
        "100755"
    } else {
        "100644"
    }
}
#[cfg(not(unix))]
fn regular_file_mode(_md: &std::fs::Metadata) -> &'static str {
    "100644"
}

/// A new-file ordinary section for untracked `path` with `content` and
/// `mode` (`100644`, `100755`, or `120000` -- exactly what `git diff`
/// shows for a staged new file). The caller selects the mode: symlinks
/// read as `120000` (T354), executables as `100755` (T364, unix only).
/// Empty content emits headers only, exactly what real git shows for
/// an empty new file (T364) -- the old `@@ -0,0 +1,0 @@` shape was
/// never git's and masked the empty-create drop.
fn new_file_section(path: &str, content: &str, mode: &str) -> String {
    let mut section = format!("diff --git a/{path} b/{path}\nnew file mode {mode}\n");
    if content.is_empty() {
        return section;
    }
    section.push_str(&format!("--- /dev/null\n+++ b/{path}\n"));
    let line_count = content.lines().count();
    section.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
    for line in content.lines() {
        section.push('+');
        section.push_str(line);
        section.push('\n');
    }
    // `git diff` ends a section missing the trailing newline with the
    // `\ No newline at end of file` marker; reproduce it so the shape
    // stays `git apply`-compatible.
    if !content.ends_with('\n') {
        section.push_str("\\ No newline at end of file\n");
    }
    section
}

/// Repo-relative paths covered by a unified diff: the `b/` side of
/// every `diff --git a/… b/…` header (unquoted spaces rejoined via
/// `split_git_paths`, T379; a single-path header yields that path).
pub fn files_in_diff(raw: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = match rsmpl_patch::split_git_paths(rest) {
                Some(parts) => {
                    let side = parts.last().cloned().unwrap_or_default();
                    side.strip_prefix("b/").unwrap_or(&side).to_string()
                }
                None => continue,
            };
            if !files.contains(&path) {
                files.push(path);
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/lib.rs b/src/lib.rs\nindex 1..2 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n.";

    #[test]
    fn files_in_diff_lists_both_sides_once() {
        let raw = "diff --git a/a.rs b/a.rs\n@@\n\
                   diff --git a/b.rs b/b.rs\n@@\n\
                   diff --git a/a.rs b/a.rs\n@@\n";
        assert_eq!(files_in_diff(raw), vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn new_file_section_shape() {
        let section = new_file_section("src/new.rs", "fn n() {}\n", "100644");
        assert!(section.starts_with("diff --git a/src/new.rs b/src/new.rs\n"));
        assert!(section.contains("new file mode 100644\n"));
        assert!(section.contains("--- /dev/null\n"));
        assert!(section.contains("@@ -0,0 +1,1 @@\n"));
        assert!(section.contains("+fn n() {}\n"));
    }

    #[test]
    fn new_file_section_symlink_mode() {
        // Link-target strings carry no trailing newline, so the
        // section reproduces git's exact shape (`+target` + marker).
        let section = new_file_section("link.txt", "real.txt", "120000");
        assert!(section.contains("new file mode 120000\n"));
        assert!(section.contains("+real.txt\n"));
        assert!(section.contains("\\ No newline at end of file\n"));
    }

    #[test]
    fn new_file_section_marks_missing_trailing_newline() {
        let section = new_file_section("x.txt", "no-newline", "100644");
        assert!(section.contains("\\ No newline at end of file\n"));
    }

    #[test]
    fn new_file_section_empty_is_headers_only() {
        // T364: an empty untracked file emits exactly what real git
        // shows (headers only, no hunk) -- the old `@@ -0,0 +1,0 @@`
        // shape never occurs in git output.
        let section = new_file_section("empty.txt", "", "100644");
        assert!(section.contains("new file mode 100644\n"));
        assert!(!section.contains("@@"), "got: {section}");
        assert!(!section.contains("--- /dev/null"), "got: {section}");
    }

    #[test]
    fn new_file_section_exec_mode_passes_through() {
        // T364: the caller-selected 100755 for executables reaches
        // the section verbatim (the lowering honors it).
        let section = new_file_section("run.sh", "#!/bin/sh\n", "100755");
        assert!(section.contains("new file mode 100755\n"));
        assert!(section.contains("--- /dev/null\n"));
    }

    #[test]
    fn files_in_diff_empty() {
        assert!(files_in_diff("").is_empty());
        assert!(files_in_diff(SAMPLE) == vec!["src/lib.rs"]);
    }

    #[test]
    fn files_in_diff_rejoins_unquoted_spaces() {
        // T379: real git never quotes spaces, so the `b/` side is
        // rejoined via the shared splitter instead of `splitn(2, ' ')`.
        let raw = "diff --git a/my file.txt b/my file.txt\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(files_in_diff(raw), vec!["my file.txt"]);
        // A single-path header (SPEC 6) yields that path, not "".
        assert_eq!(
            files_in_diff("diff --git src/lib.rs\n@@\n"),
            vec!["src/lib.rs"]
        );
    }

    #[test]
    fn history_paths_are_detected() {
        assert!(is_history_path(".history/x.rsmpl"));
        assert!(is_history_path(".history"));
        assert!(!is_history_path("src/.history/x"));
        assert!(!is_history_path("history.rs"));
        assert!(!is_history_path("src/lib.rs"));
    }
}
