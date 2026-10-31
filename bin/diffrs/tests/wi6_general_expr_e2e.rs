//! WI-6 e2e: general-expression raising compresses body rewrites the
//! engine applies but no detector raised (`E0 + 1` → `E0 + 2`); twin,
//! single-site, and call-shape classes veto. Whole-arg permutation at
//! subterm granularity re-infers the authored sweep.
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

// WI-6: binary-op body rewrite compresses and real-applies byte-identical.
#[test]
fn general_binary_op_compresses_and_reapplies() {
    let dir = init_repo();
    let before = "fn a() -> i32 {\n    1 + 1\n}\nfn b() -> i32 {\n    2 + 1\n}\nfn c() -> i32 {\n    3 + 1\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn a() -> i32 {\n    1 + 2\n}\nfn b() -> i32 {\n    2 + 2\n}\nfn c() -> i32 {\n    3 + 2\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- E0 + 1\n"), "got: {patch}");
    assert!(patch.contains("+ E0 + 2\n"), "got: {patch}");
    assert!(patch.contains("# allow=many\n"), "got: {patch}");
}

// WI-6: an untouched twin matching the minus template vetoes to ordinary.
#[test]
fn general_untouched_twin_vetoes() {
    let dir = init_repo();
    let before = "fn a() -> i32 {\n    1 + 1\n}\nfn b() -> i32 {\n    2 + 1\n}\nfn c() -> i32 {\n    3 + 1\n}\nfn untouched() -> i32 {\n    4 + 1\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn a() -> i32 {\n    1 + 2\n}\nfn b() -> i32 {\n    2 + 2\n}\nfn c() -> i32 {\n    3 + 2\n}\nfn untouched() -> i32 {\n    4 + 1\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!patch.contains("E0 + 1"), "twin must veto, got: {patch}");
}

// WI-6: whole-line call shapes stay with the tighter call detectors.
#[test]
fn general_call_whole_line_declines() {
    let dir = init_repo();
    let before = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn m() {\n    foo(3);\n    foo(4);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!patch.contains("general_expr"), "got: {patch}");
}

// WI-6 slice 2: sub-line whole-arg permutation re-infers the authored
// sweep and real-applies byte-identical.
#[test]
fn subline_permutation_compresses_and_reapplies() {
    let dir = init_repo();
    let before =
        "fn swap(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn f(x: i32, y: i32) -> i32 {\n    swap(x, y) + swap(y, x)\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after =
        "fn swap(a: i32, b: i32) -> i32 {\n    a + b\n}\nfn f(x: i32, y: i32) -> i32 {\n    swap(y, x) + swap(x, y)\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- swap(E0, E1)\n"), "got: {patch}");
    assert!(patch.contains("+ swap(E1, E0)\n"), "got: {patch}");
}
