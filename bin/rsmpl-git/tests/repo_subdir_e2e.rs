//! T186: `--repo <subdir>` applies against the work-tree root.
//!
//! `git apply` in a repo subdirectory resolves patch paths against the
//! work-tree root and silently skips CWD-relative ones (exit 0). The CLI
//! runs `git apply` at the enclosing root with `--directory=<prefix>`
//! instead, so subdir invocation applies identically (and `--check`
//! failures still fail instead of passing vacuously).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git"))
}

fn init_repo_with_sub() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let st = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(st.success());
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    (dir, sub)
}

fn run_patch(repo: &Path, patch: &str, extra: &[&str]) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(binary())
        .args(extra)
        .arg("--repo")
        .arg(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn subdir_repo_apply_mutates_worktree() {
    // T186: `--repo <subdir>` used to print `applied 1 file(s)` while the
    // worktree stayed untouched. Now it applies.
    let (_dir, sub) = init_repo_with_sub();
    fs::write(sub.join("f.rs"), "struct Old;\n").unwrap();
    let patch = "diff --git f.rs\n@@ -1,1 @@ rule=t\n# lang=rust\n- struct Old\n+ struct New\n";
    let out = run_patch(&sub, patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(sub.join("f.rs")).unwrap(),
        "struct New;\n"
    );
}

#[test]
fn subdir_repo_check_failure_still_fails() {
    // T186: the vacuous `--check` pass is gone — a bad hunk under
    // `--repo <subdir>` fails with GIT_HUNK_CONFLICT.
    let (_dir, sub) = init_repo_with_sub();
    fs::write(sub.join("f.rs"), "struct Old;\n").unwrap();
    let before = fs::read_to_string(sub.join("f.rs")).unwrap();
    let patch = "diff --git f.rs\n@@ -1,1 +1,1 @@\n-struct WRONG;\n+struct New;\n";
    let out = run_patch(&sub, patch, &["--check"]);
    assert!(!out.status.success(), "must fail");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("GIT_HUNK_CONFLICT"),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_to_string(sub.join("f.rs")).unwrap(), before);
}
