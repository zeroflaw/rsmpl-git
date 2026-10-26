//! Applying the lowered patch with `git apply` (SPEC 26.1).
//!
//! The engine converts the RSmPL-Git patch into an ordinary Git patch; this
//! module hands that patch to Git. `--check` maps to `git apply --check`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use rsmpl_patch::{ErrorCode, StructuredError};

fn git_error(code: ErrorCode, message: String) -> StructuredError {
    StructuredError::new(code, message)
}

fn run_git_apply(repo: &Path, patch: &str, check_only: bool) -> Result<String, StructuredError> {
    let mut cmd = Command::new("git");
    cmd.arg("apply");
    if check_only {
        cmd.arg("--check");
    }
    cmd.current_dir(repo)
        .stdin(Stdio::piped())
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
}
