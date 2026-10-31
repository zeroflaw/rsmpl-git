//! Applying the lowered patch with `git apply` (SPEC 26.1).
//!
//! The engine converts the RSmPL-Git patch into an ordinary Git patch; this
//! module hands that patch to Git. `--check` maps to `git apply --check`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn work_tree_for(repo: &Path) -> Option<(PathBuf, String)> {
    // T186: `git apply` in a repo subdirectory resolves patch paths
    // against the work-tree root and silently skips CWD-relative ones
    // (exit 0). Run at the root with `--directory=<prefix>` instead.
    // Outside a work tree (empty-dir lint) CWD-relative paths apply
    // fine, so a failed probe keeps today's CWD=repo behavior.
    let top = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())?;
    let prefix = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["rev-parse", "--show-prefix"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .trim_end_matches('/')
                .to_string()
        })?;
    Some((PathBuf::from(top), prefix))
}

use rsmpl_patch::{ErrorCode, StructuredError};

fn git_error(code: ErrorCode, message: String) -> StructuredError {
    // WI-3: `git apply` failures are never prose-only. Spawn/pipe faults
    // name the environment fix; a rejected lowering names the patch-side
    // retry (fresh ranges after re-reading the file).
    let repair = if message.starts_with("failed to run")
        || message.starts_with("failed to feed")
        || message.starts_with("failed waiting")
    {
        StructuredError::fix_pattern_repair(
            "environment",
            "ensure `git` is installed and the repo is writable, then retry the same patch",
        )
    } else {
        StructuredError::fix_pattern_repair(
            "stale",
            "re-read the target files and retry with fresh ranges; if the failure persists it is an engine lowering bug, not a patch bug",
        )
    };
    StructuredError::new(code, message).with_repair(repair)
}

fn run_git_apply(repo: &Path, patch: &str, check_only: bool) -> Result<String, StructuredError> {
    let mut cmd = Command::new("git");
    // `--unidiff-zero` permits the always-0-context lowering (SPEC D.2.2);
    // it is a no-op for hunks that do carry context, so it is
    // passed unconditionally rather than threaded through every caller.
    cmd.args(["apply", "--unidiff-zero"]);
    if check_only {
        cmd.arg("--check");
    }
    // T186: `--repo <subdir>` silently no-ops without this: git skips
    // CWD-relative paths below the work-tree root (exit 0).
    match work_tree_for(repo) {
        Some((top, prefix)) => {
            if !prefix.is_empty() {
                cmd.arg(format!("--directory={prefix}"));
            }
            cmd.current_dir(top);
        }
        None => {
            cmd.current_dir(repo);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        git_error(
            ErrorCode::GitHunkConflict,
            format!("failed to run `git apply` (is git installed?): {e}"),
        )
    })?;
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(patch.as_bytes())
        .map_err(|e| {
            git_error(
                ErrorCode::GitHunkConflict,
                format!("failed to feed patch to git: {e}"),
            )
        })?;
    let output = child.wait_with_output().map_err(|e| {
        git_error(
            ErrorCode::GitHunkConflict,
            format!("failed waiting for `git apply`: {e}"),
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(git_error(
            ErrorCode::GitHunkConflict,
            format!(
                "`git apply{}` failed:\n{}{}",
                if check_only { " --check" } else { "" },
                stdout,
                stderr
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Dry run: verify the lowered patch applies cleanly.
pub fn check_with_git(repo: &Path, patch: &str) -> Result<(), StructuredError> {
    run_git_apply(repo, patch, true).map(|_| ())
}

/// Apply the lowered patch to the repository worktree.
pub fn apply_with_git(repo: &Path, patch: &str) -> Result<(), StructuredError> {
    run_git_apply(repo, patch, false).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn round_trips_a_lowered_patch_through_git() {
        let dir = tempfile::tempdir().unwrap();
        let st = Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(st.success());
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let patch =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,3 @@\n a\n+b2\n b\n";
        check_with_git(dir.path(), patch).unwrap();
        apply_with_git(dir.path(), patch).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a\nb2\nb\n"
        );
        // A conflicting patch must fail with a GIT error, not silently.
        let bad =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,2 @@\nzzz\n b\n";
        let err = check_with_git(dir.path(), bad).unwrap_err();
        assert!(err.code == ErrorCode::GitHunkConflict);
    }

    #[test]
    fn zero_context_patch_round_trips_with_unidiff_zero() {
        let dir = tempfile::tempdir().unwrap();
        let st = Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(st.success());
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\n").unwrap();
        // No context lines at all: plain `git apply` rejects this, but our
        // runner passes `--unidiff-zero` so it verifies and applies.
        let patch = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -2 +2 @@\n-b\n+B\n";
        check_with_git(dir.path(), patch).unwrap();
        apply_with_git(dir.path(), patch).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a\nB\nc\n"
        );
    }

    #[test]
    fn subdir_repo_applies_against_work_tree_root() {
        // T186: `git apply` in a repo subdirectory resolves patch paths
        // against the work-tree root and silently skips CWD-relative
        // ones (exit 0). The runner must apply, not skip.
        let dir = tempfile::tempdir().unwrap();
        let st = Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(st.success());
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/f.txt"), "a\nb\n").unwrap();
        let patch =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,3 @@\n a\n+X\n b\n";
        let sub = dir.path().join("sub");
        check_with_git(&sub, patch).unwrap();
        apply_with_git(&sub, patch).unwrap();
        assert_eq!(
            std::fs::read_to_string(sub.join("f.txt")).unwrap(),
            "a\nX\nb\n"
        );
    }
}
