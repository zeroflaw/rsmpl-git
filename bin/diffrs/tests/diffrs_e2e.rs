//! T340 e2e: `diffrs` compresses native git state into a valid
//! RSmPL-Git patch (v0: ordinary-hunk passthrough).
//!
//! Validity = the output parses and `applyrs --check` accepts it
//! against the pre-change tree. Requires a workspace build so the
//! sibling `applyrs` binary sits next to `diffrs`
//! (`cargo test --workspace`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diffrs"))
}

fn sibling_applyrs() -> PathBuf {
    let me = binary();
    let file = if cfg!(windows) {
        "applyrs.exe".to_string()
    } else {
        "applyrs".to_string()
    };
    me.parent().map(|p| p.join(&file)).unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn commit_all(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    );
}

fn write(dir: &Path, rel: &str, text: &str) {
    if let Some(parent) = Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(dir.join(parent)).unwrap();
        }
    }
    std::fs::write(dir.join(rel), text).unwrap()
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

fn check_with_applyrs(dir: &Path, patch: &str) -> std::process::Output {
    Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(patch)
        .arg("--check")
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn unstaged_edit_round_trips_through_applyrs() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    write(dir.path(), "src/lib.rs", "fn foo() {}\nfn bar() {}\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    assert!(patch.contains("+fn bar() {}"));

    // Back to the pre-change tree: the captured patch must check clean.
    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[test]
fn untracked_new_file_round_trips_through_applyrs() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    write(dir.path(), "src/new.rs", "fn brand_new() {}\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git a/src/new.rs b/src/new.rs"));
    assert!(patch.contains("new file mode"));
    assert!(patch.contains("+fn brand_new() {}"));

    // Remove the new file: the tree is now the pre-change state and the
    // captured patch (a creation) must check clean.
    std::fs::remove_file(dir.path().join("src/new.rs")).unwrap();
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[test]
fn empty_repo_reports_no_changes() {
    let dir = init_repo();
    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("(no changes)"), "got: {stderr}");
}

#[test]
fn staged_flag_isolates_index() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() {}\n");
    write(dir.path(), "b.rs", "fn b() {}\n");
    commit_all(dir.path());
    write(dir.path(), "a.rs", "fn a() {}\n// staged\n");
    write(dir.path(), "b.rs", "fn b() {}\n// unstaged\n");
    git(dir.path(), &["add", "a.rs"]);

    let staged = run(dir.path(), &["--staged"]);
    assert_eq!(staged.status.code(), Some(0));
    let staged_patch = String::from_utf8_lossy(&staged.stdout).to_string();
    assert!(staged_patch.contains("a.rs"), "got: {staged_patch}");
    assert!(!staged_patch.contains("b.rs"), "got: {staged_patch}");

    let unstaged = run(dir.path(), &[]);
    assert_eq!(unstaged.status.code(), Some(0));
    let unstaged_patch = String::from_utf8_lossy(&unstaged.stdout).to_string();
    assert!(unstaged_patch.contains("b.rs"), "got: {unstaged_patch}");
    assert!(!unstaged_patch.contains("a.rs"), "got: {unstaged_patch}");
}

#[test]
fn rev_argument_covers_history() {
    let dir = init_repo();
    write(dir.path(), "f.rs", "fn v1() {}\n");
    commit_all(dir.path());
    write(dir.path(), "f.rs", "fn v2() {}\n");
    commit_all(dir.path());

    // Worktree is clean; `HEAD~1` still surfaces the last commit.
    let out = run(dir.path(), &["HEAD~1"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("+fn v2() {}"), "got: {patch}");
}

#[test]
fn json_envelope_shape() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    write(dir.path(), "src/lib.rs", "fn foo() {}\nfn bar() {}\n");

    let out = run(dir.path(), &["--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["version"], serde_json::json!(1));
    assert_eq!(
        v["files"],
        serde_json::json!([{"path": "src/lib.rs"}]),
        "got: {v}"
    );
    assert!(
        v["patch"].as_str().unwrap_or("").contains("diff --git"),
        "got: {v}"
    );
}

#[test]
fn bad_repo_fails_with_envelope() {
    let out = run(Path::new("/nonexistent-diffrs-repo"), &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert!(v["error"]["rule"].is_null());
}
