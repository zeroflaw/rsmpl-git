//! WI-2b durable fixture: `let`-binder block completion end-to-end.
//!
//! Engine unit pins (`use_graph_local.rs`) cover the walker only. These two
//! tests apply a `let total = 1;` → `let sum = 1;` rename for real and prove
//! the post-tree builds: plain completion rewrites every block-scoped use and
//! reports `verified`; an inner `let total` shadow keeps its uses, reports
//! `partial` with `off-target` evidence, and still builds (downgrade is
//! evidence-driven, not build-driven).

mod common;
use common::*;
use std::process::Command;

fn cargo_crate(dir: &std::path::Path, lib_rs: &str) {
    write(
        dir,
        "Cargo.toml",
        "[package]\nname = \"wi2bcrate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(dir, "src/lib.rs", lib_rs);
}

fn cargo_check(dir: &std::path::Path) {
    let out = Command::new("cargo")
        .args(["check", "--quiet"])
        .current_dir(dir)
        .output()
        .expect("cannot run `cargo check`");
    assert!(
        out.status.success(),
        "cargo check failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON")
}

const RENAME_PATCH: &str =
    "diff --git src/lib.rs\n@@ -2,1 @@ rule=rename_let\n- let total = 1;\n+ let sum = 1;\n";

#[test]
fn let_binder_completion_applies_and_checks_clean() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn compute() -> i32 {\n    let total = 1;\n    let x = total + total;\n    x\n}\n",
    );
    // JSON probe pins the honesty word before the real apply.
    let out = run_stdin(
        dir.path(),
        RENAME_PATCH,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["semantics"],
        serde_json::json!("instantiated"),
        "got: {v}"
    );

    // Real apply + build.
    let out = run_stdin(dir.path(), RENAME_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let sum = 1;"), "got:\n{post}");
    assert!(post.contains("let x = sum + sum;"), "got:\n{post}");
    assert!(!post.contains("total"), "got:\n{post}");
    cargo_check(dir.path());
}

#[test]
fn let_binder_shadow_keeps_inner_and_reports_partial() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn compute() -> i32 {\n    let total = 1;\n    let a = total;\n    {\n        let total = 2;\n        let b = total;\n    }\n    let c = total;\n    a + c\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        RENAME_PATCH,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["semantics"],
        serde_json::json!("reparse-only"),
        "got: {v}"
    );
    let orphans = v["rules"][0]["orphans"].as_array().expect("orphans");
    assert_eq!(orphans.len(), 1, "got: {v}");
    assert_eq!(
        orphans[0]["reason"],
        serde_json::json!("off-target"),
        "got: {v}"
    );
    assert_eq!(orphans[0]["line"], serde_json::json!(6), "got: {v}");

    let out = run_stdin(dir.path(), RENAME_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let sum = 1;"), "got:\n{post}");
    assert!(post.contains("let a = sum;"), "got:\n{post}");
    assert!(post.contains("let c = sum;"), "got:\n{post}");
    // Inner shadow untouched.
    assert!(post.contains("let total = 2;"), "got:\n{post}");
    assert!(post.contains("let b = total;"), "got:\n{post}");
    cargo_check(dir.path());
}
