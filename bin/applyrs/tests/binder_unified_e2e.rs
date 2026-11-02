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

// WI-13: field/method receivers headed by the binder and captures in
// nested closures are binder uses — they complete and report
// `verified`.
const LET_RENAME_PATCH: &str =
    "diff --git src/lib.rs\n@@ -2,1 @@ rule=rename_let\n- let total = make();\n+ let sum = make();\n";

#[test]
fn let_field_method_and_nested_capture_complete_clean() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "struct S { field: u32 }\nimpl S { fn method(&self) -> u32 { self.field } }\nfn make() -> S { S { field: 0 } }\npub fn run() {\n    let total = make();\n    let x = total.field;\n    let y = total.method();\n    let f = || { let g = || total.field + 1; g() };\n    let _ = (x, y, f);\n}\n",
    );
    let v = check_json(dir.path(), LET_RENAME_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), LET_RENAME_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let sum = make();"), "got: {post}");
    assert!(post.contains("sum.field"), "got: {post}");
    assert!(!post.contains("total.field"), "got: {post}");
    assert!(post.contains("sum.method()"), "got: {post}");
    assert!(post.contains("|| sum.field + 1"), "got: {post}");
    cargo_check(dir.path());
}

// WI-13: a shadowing `for` binder governs only the loop body — the
// outer use after the loop rewrites; the in-body use stays a
// `shadowed` orphan (so the run is `partial`, never `verified`).
const LET_SIMPLE_PATCH: &str =
    "diff --git src/lib.rs\n@@ -2,1 @@ rule=rename_let\n- let total = 1;\n+ let sum = 1;\n";

#[test]
fn let_for_shadow_after_use_rewrites_shadowed_stays() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let total = 1;\n    for total in 0..3 { let x = total; }\n    let y = total;\n}\n",
    );
    let v = check_json(dir.path(), LET_SIMPLE_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    let orphans = v["rules"][0]["orphans"].as_array().unwrap();
    assert!(
        !orphans.is_empty()
            && orphans
                .iter()
                .all(|o| o["reason"] == serde_json::json!("shadowed")),
        "got: {v}"
    );
    assert!(
        !orphans
            .iter()
            .any(|o| o["reason"] == serde_json::json!("off-target")),
        "got: {v}"
    );
    apply_patch(dir.path(), LET_SIMPLE_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let y = sum;"), "got: {post}");
    assert!(post.contains("for total in"), "got: {post}");
}

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
