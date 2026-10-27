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
            .arg("--no-renames")
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
        // + `rename from/to` headers), which RSmPL-Git v0.1 cannot
        // express (file renames are unsupported, SPEC 6); callers
        // configure `--no-renames` above so moves surface as
        // delete+create ordinary hunks that round-trip cleanly.
        // `--unified=0` keeps ordinary passthrough minimal-token:
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
            let content =
                std::fs::read(self.repo.join(path)).map_err(|e| SourceError::Untracked {
                    path: path.clone(),
                    inner: e.to_string(),
                })?;
            // Binary untracked files are skipped here; the infer stage
            // strips binary hunks with a warning, and raw bytes cannot
            // ride a UTF-8 patch.
            if content.contains(&0) {
                continue;
            }
            let text = String::from_utf8_lossy(&content);
            raw.push_str(&new_file_section(path, &text));
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

/// A new-file ordinary section for untracked `path` with `content`.
fn new_file_section(path: &str, content: &str) -> String {
    let mut section = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
    );
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
/// every `diff --git a/… b/…` header.
pub fn files_in_diff(raw: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let mut parts = rest.splitn(2, ' ');
            let b = parts.nth(1).unwrap_or("");
            let path = b.strip_prefix("b/").unwrap_or(b);
            let path = path.trim_matches('"').to_string();
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
        let section = new_file_section("src/new.rs", "fn n() {}\n");
        assert!(section.starts_with("diff --git a/src/new.rs b/src/new.rs\n"));
        assert!(section.contains("new file mode 100644\n"));
        assert!(section.contains("--- /dev/null\n"));
        assert!(section.contains("@@ -0,0 +1,1 @@\n"));
        assert!(section.contains("+fn n() {}\n"));
    }

    #[test]
    fn new_file_section_marks_missing_trailing_newline() {
        let section = new_file_section("x.txt", "no-newline");
        assert!(section.contains("\\ No newline at end of file\n"));
    }

    #[test]
    fn files_in_diff_empty() {
        assert!(files_in_diff("").is_empty());
        assert!(files_in_diff(SAMPLE) == vec!["src/lib.rs"]);
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
