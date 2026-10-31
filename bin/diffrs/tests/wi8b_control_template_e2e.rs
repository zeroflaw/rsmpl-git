//! WI-8b e2e: control-position templates compress subterm deltas the
//! engine applies but no detector raised (match guards, closure bodies,
//! `if` conditions); twins veto.
//!
//! Binary-level proofs via the real `diffrs`/`applyrs` binaries
//! (helpers duplicated, not shared — T246 precedent).

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

/// `diffrs` the worktree change, then prove the semantic patch
/// real-applies byte-identical: stash, apply for real, compare bytes.
fn diffrs_and_reapply(dir: &Path, after: &str) -> String {
    let out = run(dir, &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    git(dir, &["stash", "-q"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let back = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert_eq!(back, after, "real-apply diverged");
    patch
}

// WI-8b: match-arm guards compress and re-apply byte-identical.
#[test]
fn control_guard_compresses_and_reapplies() {
    let dir = init_repo();
    let before = "fn f(v: i32) -> i32 {\n    match v {\n        _ if v > 0 => 1,\n        _ => 0,\n    }\n}\nfn g(w: i32) -> i32 {\n    match w {\n        _ if w > 0 => 1,\n        _ => 0,\n    }\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn f(v: i32) -> i32 {\n    match v {\n        _ if v >= 0 => 1,\n        _ => 0,\n    }\n}\nfn g(w: i32) -> i32 {\n    match w {\n        _ if w >= 0 => 1,\n        _ => 0,\n    }\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- E0 > 0\n"), "got: {patch}");
    assert!(patch.contains("+ E0 >= 0\n"), "got: {patch}");
    assert!(patch.contains("# allow=many\n"), "got: {patch}");
}

// WI-8b: closure bodies compress and re-apply byte-identical.
#[test]
fn control_closure_compresses_and_reapplies() {
    let dir = init_repo();
    let before =
        "fn f() {\n    let f = |x| x + 1;\n    let g = |y| y + 1;\n    let _ = (f(1), g(2));\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after =
        "fn f() {\n    let f = |x| x + 2;\n    let g = |y| y + 2;\n    let _ = (f(1), g(2));\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- E0 + 1\n"), "got: {patch}");
    assert!(patch.contains("+ E0 + 2\n"), "got: {patch}");
}

// WI-8b: `if` conditions compress and re-apply byte-identical.
#[test]
fn control_if_cond_compresses_and_reapplies() {
    let dir = init_repo();
    let before = "fn f(v: i32) -> i32 {\n    if v > 0 {\n        1\n    } else {\n        0\n    }\n}\nfn g(w: i32) -> i32 {\n    if w > 0 {\n        1\n    } else {\n        0\n    }\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn f(v: i32) -> i32 {\n    if v >= 0 {\n        1\n    } else {\n        0\n    }\n}\nfn g(w: i32) -> i32 {\n    if w >= 0 {\n        1\n    } else {\n        0\n    }\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- E0 > 0\n"), "got: {patch}");
    assert!(patch.contains("+ E0 >= 0\n"), "got: {patch}");
}

// WI-8b: an untouched guard twin vetoes to ordinary (the body is never
// rewritten).
#[test]
fn control_untouched_twin_vetoes() {
    let dir = init_repo();
    let before = "fn f(v: i32) -> i32 {\n    match v {\n        _ if v > 0 => 1,\n        _ => 0,\n    }\n}\nfn g(w: i32) -> i32 {\n    match w {\n        _ if w > 0 => 1,\n        _ => 0,\n    }\n}\nfn h(z: i32) -> i32 {\n    match z {\n        _ if z > 0 => 1,\n        _ => 0,\n    }\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn f(v: i32) -> i32 {\n    match v {\n        _ if v >= 0 => 1,\n        _ => 0,\n    }\n}\nfn g(w: i32) -> i32 {\n    match w {\n        _ if w >= 0 => 1,\n        _ => 0,\n    }\n}\nfn h(z: i32) -> i32 {\n    match z {\n        _ if z > 0 => 1,\n        _ => 0,\n    }\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!patch.contains("E0 > 0"), "twin must veto, got: {patch}");
}
