//! WI-8 slice 1 e2e: untyped-let raising compresses `Some(P0) = E0`
//! init deltas, `const`-block/`_` uniform deltas, and tuple-position
//! swaps that the engine applies but no detector raised; twins veto.
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

// WI-8 slice 1: hetero `Some` unwrap compresses and re-applies
// byte-identical.
#[test]
fn untyped_some_unwrap_compresses_and_reapplies() {
    let dir = init_repo();
    let before = "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let _ = (v, w);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o.unwrap_or(0);\n    let Some(w) = p.unwrap_or(0);\n    let _ = (v, w);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- let Some(P0) = E0;\n"), "got: {patch}");
    assert!(
        patch.contains("+ let Some(P0) = E0.unwrap_or(0);\n"),
        "got: {patch}"
    );
    assert!(patch.contains("# allow=many\n"), "got: {patch}");
}

// WI-8 slice 1: tuple-position swap compresses and re-applies
// byte-identical.
#[test]
fn pat_swap_compresses_and_reapplies() {
    let dir = init_repo();
    let before = "fn a(t: (i32, i32), u: (i32, i32)) {\n    let (x, y) = t;\n    let (m, n) = u;\n    let _ = (x, y, m, n);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn a(t: (i32, i32), u: (i32, i32)) {\n    let (y, x) = t;\n    let (n, m) = u;\n    let _ = (x, y, m, n);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- let (P0, P1) = E0;\n"), "got: {patch}");
    assert!(patch.contains("+ let (P1, P0) = E0;\n"), "got: {patch}");
}

// WI-8 slice 1: const-block uniform delta fires across sites.
#[test]
fn untyped_const_block_compresses_and_reapplies() {
    let dir = init_repo();
    let before =
        "fn f() {\n    let v = const { 1 + 2 };\n}\nfn g() {\n    let w = const { 1 + 2 };\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after =
        "fn f() {\n    let v = const { 1 + 3 };\n}\nfn g() {\n    let w = const { 1 + 3 };\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(
        patch.contains("- let P0 = const { 1 + 2 };\n"),
        "got: {patch}"
    );
    assert!(
        patch.contains("+ let P0 = const { 1 + 3 };\n"),
        "got: {patch}"
    );
}

// WI-8 slice 1: an untouched twin matching the minus template vetoes
// to ordinary.
#[test]
fn untyped_untouched_twin_vetoes() {
    let dir = init_repo();
    let before = "fn a(o: Option<i32>, p: Option<i32>, q: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let Some(z) = q;\n    let _ = (v, w, z);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn a(o: Option<i32>, p: Option<i32>, q: Option<i32>) {\n    let Some(v) = o.unwrap_or(0);\n    let Some(w) = p.unwrap_or(0);\n    let Some(z) = q;\n    let _ = (v, w, z);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.contains("let Some(P0) = E0;"),
        "twin must veto, got: {patch}"
    );
}
