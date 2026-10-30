//! WI-7-history-strip: tracked/staged `.history/` audit logs strip
//! like the untracked one (one warning, never embedded) while the
//! real edit still converts.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diffrs"))
}

fn sibling_applyrs() -> PathBuf {
    let me = binary();
    me.parent().map(|p| p.join("applyrs")).unwrap()
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

// A staged tracked `.history/` audit log strips like the untracked
// one (one warning, never embedded) while the staged real edit
// still converts and checks clean.
#[test]
fn staged_tracked_history_is_stripped_with_warning() {
    let dir = init_repo();
    write(dir.path(), "base.txt", "base\n");
    write(dir.path(), ".history/audit.rsmpl", "audit noise\n");
    commit_all(dir.path());
    write(dir.path(), "base.txt", "base changed\n");
    write(dir.path(), ".history/audit.rsmpl", "audit noise\nmore\n");
    git(dir.path(), &["add", "-A"]);

    let out = run(dir.path(), &["--staged"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(patch.contains("base.txt"), "got: {patch}");
    assert!(!patch.contains(".history"), "history embedded: {patch}");
    assert!(stderr.contains("warning: .history/"), "got: {stderr}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}
