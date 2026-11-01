//! WI-3 durable fixture: binder honesty keyed by binding, end-to-end.
//!
//! Engine units pin the detectors, the follow walk, and the probe
//! adjustment. These tests apply real patches and prove the post-tree
//! builds: the F3 let-wrapped closure spelling completes and reports
//! `verified`; a shadowed inner binder keeps its uses, reports
//! `partial` with `shadowed` evidence, and still builds (downgrade is
//! evidence-driven, not build-driven). Closure-param and fn-param
//! completions ride the same path.

mod common;
use common::*;
use std::process::Command;

fn cargo_crate(dir: &std::path::Path, lib_rs: &str) {
    write(
        dir,
        "Cargo.toml",
        "[package]\nname = \"wi3crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
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

fn check_json(dir: &std::path::Path, patch: &str) -> serde_json::Value {
    let out = run_stdin(
        dir,
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    json_of(&out)
}

fn apply_patch(dir: &std::path::Path, patch: &str) {
    let out = run_stdin(dir, patch, &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// F3 headline: the let-wrapped closure spelling that escaped every
// surface detector now completes (`|sum| sum + 1`), not just confesses.
const LET_CLOSURE_PATCH: &str =
    "diff --git src/lib.rs\n@@ -2,1 @@ rule=rename_closure_init\n- let g = |total| E;\n+ let g = |sum| E;\n";

#[test]
fn let_wrapped_closure_completion_applies_and_checks_clean() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let g = |total| total + 1;\n    let _ = g(1);\n}\n",
    );
    let v = check_json(dir.path(), LET_CLOSURE_PATCH);
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
    apply_patch(dir.path(), LET_CLOSURE_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let g = |sum| sum + 1;"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn let_wrapped_closure_shadowed_reports_partial() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let g = |total| { let total = 1; total };\n    let _ = g(1);\n}\n",
    );
    let v = check_json(dir.path(), LET_CLOSURE_PATCH);
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
    assert!(
        v["rules"][0]["orphans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["reason"] == serde_json::json!("shadowed")),
        "got: {v}"
    );
    apply_patch(dir.path(), LET_CLOSURE_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("|sum|"), "got: {post}");
    assert!(post.contains("let total = 1; total"), "got: {post}");
    cargo_check(dir.path());
}

const CLOSURE_PARAM_PATCH: &str =
    "diff --git src/lib.rs\n@@ -2,1 @@ rule=rename_closure\n- |total: u32| E\n+ |sum: u32| E\n";

#[test]
fn closure_param_completion_applies_and_checks_clean() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let f = |total: u32| total + total;\n    let _ = f;\n}\n",
    );
    let v = check_json(dir.path(), CLOSURE_PARAM_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), CLOSURE_PARAM_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("|sum: u32| sum + sum"), "got: {post}");
    cargo_check(dir.path());
}

const FN_PARAM_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_param\n- pub fn foo(total: u32) { ... }\n+ pub fn foo(sum: u32) { ... }\n";

#[test]
fn fn_param_completion_applies_and_checks_clean() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn foo(total: u32) -> u32 {\n    total + 1\n}\n",
    );
    let v = check_json(dir.path(), FN_PARAM_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), FN_PARAM_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("fn foo(sum: u32)"), "got: {post}");
    assert!(post.contains("sum + 1"), "got: {post}");
    cargo_check(dir.path());
}
