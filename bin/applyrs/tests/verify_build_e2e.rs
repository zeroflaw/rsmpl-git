//! WI-2a `--verify build` end-to-end pins (split from `cli_e2e.rs`, D5).
//!
//! Check-mode verifies the post-images in a temp copy (worktree untouched);
//! apply-mode verifies post-apply (patch stays applied). JSON failures
//! carry machine `applied`, successes carry `build: "pass"`.

mod common;
use common::*;

fn cargo_crate(dir: &std::path::Path) {
    write(
        dir,
        "Cargo.toml",
        "[package]\nname = \"wicrate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        dir,
        "src/lib.rs",
        "pub fn plus_one(x: i32) -> i32 {\n    x + 1\n}\n",
    );
}

#[test]
fn verify_build_passes_on_compiling_crate() {
    // WI-2 `--verify build`: a clean post-apply tree checks quietly.
    let dir = init_repo();
    cargo_crate(dir.path());
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=rename_plus\n# lang=rust\n- pub fn plus_one(...) {\n+ pub fn plus_two(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--verify", "build"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path(), "src/lib.rs").contains("fn plus_two"));
}

#[test]
fn verify_build_fails_on_broken_crate() {
    // WI-2 `--verify build`: the patch lands via `git apply`, then the
    // failed `cargo check` exits 2 with the first error line (never silent).
    let dir = init_repo();
    cargo_crate(dir.path());
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@\n-    x + 1\n+    undefined_fn(x)\n";
    let out = run_stdin(dir.path(), patch, &["--verify", "build"]);
    assert!(!out.status.success(), "expected failure");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("--verify build"),
        "first diagnostic on stderr, got: {stderr}"
    );
    // The lowering applied; verification failure does not roll it back.
    assert!(read(dir.path(), "src/lib.rs").contains("undefined_fn"));
}

#[test]
fn verify_build_check_mode_fails_without_touching_tree() {
    // WI-2a: `--check --verify build` verifies the post-images in a temp
    // copy — a breaking patch fails loud while the worktree stays clean.
    let dir = init_repo();
    cargo_crate(dir.path());
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@\n-    x + 1\n+    undefined_fn(x)\n";
    let out = run_stdin(dir.path(), patch, &["--check", "--verify", "build"]);
    assert!(!out.status.success(), "expected failure");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("--verify build"), "got: {stderr}");
    assert!(stderr.contains("worktree untouched"), "got: {stderr}");
    assert!(read(dir.path(), "src/lib.rs").contains("x + 1"));
}

#[test]
fn verify_build_check_mode_passes_leaving_tree_clean() {
    // WI-2a + WI-1c: a compiling post-image passes under `--check` and
    // leaves the tree byte-identical (`--check` writes no `.history/`
    // unless `--history` is passed).
    let dir = init_repo();
    cargo_crate(dir.path());
    let before = read(dir.path(), "src/lib.rs");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=rename_plus\n# lang=rust\n- pub fn plus_one(...) {\n+ pub fn plus_two(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check", "--verify", "build"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), before);
}

#[test]
fn verify_build_json_applied_flag() {
    // WI-2a: JSON verify failures carry machine `applied` — false for
    // check-mode (temp copy), true for post-apply — plus `build: pass`.
    let dir = init_repo();
    cargo_crate(dir.path());
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@\n-    x + 1\n+    undefined_fn(x)\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--check", "--verify", "build"],
    );
    assert!(!out.status.success(), "expected failure");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["applied"], serde_json::json!(false), "got: {v}");

    let dir = init_repo();
    cargo_crate(dir.path());
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--verify", "build"],
    );
    assert!(!out.status.success(), "expected failure");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["applied"], serde_json::json!(true), "got: {v}");

    let dir = init_repo();
    cargo_crate(dir.path());
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=rename_plus\n# lang=rust\n- pub fn plus_one(...) {\n+ pub fn plus_two(...) {\n      ...\n  }\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--check", "--verify", "build"],
    );
    assert!(
        out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["checked"]["build"], serde_json::json!("pass"), "got: {v}");
}
