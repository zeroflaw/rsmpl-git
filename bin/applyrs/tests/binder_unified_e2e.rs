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

// C21: a Stmt/Expr general template that rewrites bound binder idents
// without binder identity never applies — the old shape rewrote the
// header, stranded body/file uses, and still reported `verified` with no
// orphans. Stmt multi-leaf + if-let pin the fail-loud gate (text and
// JSON), the worktree stays untouched, and the single-binder twins above
// (incl. Stmt single-leaf destructuring and rename-once sequences, which
// complete like `let`) stay green as the exemption pins. Sequences are
// exempt except a rename-once `let` rename (plain or single-leaf
// destructuring, with optional authored init-substitution of later
// in-window uses — hetero fusion owns other binder-renaming runs
// there; spelled-`old` uses replay verbatim as orphans, partial swaps
// and `new`/pat mentions stay declined window-only).
const C21_REASON: &str = "general template rewrites bound binder idents without binder identity";

fn check_c21_text(dir: &std::path::Path, patch: &str, content: &str) {
    let out = run_stdin(dir, patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: src/lib.rs"), "got: {stderr}");
    assert!(stderr.contains(C21_REASON), "got: {stderr}");
    assert!(stderr.contains("C21"), "got: {stderr}");
    let got = read(dir, "src/lib.rs");
    assert_eq!(got, content, "worktree must be untouched");
}

fn check_c21_json(dir: &std::path::Path, patch: &str, content: &str) {
    let out = run_stdin(dir, patch, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["reason"], serde_json::json!("general template rewrites bound binder idents without binder identity; v1 general templates never own binder renames (C21)"));
    assert_eq!(
        v["error"]["repair"]["code"],
        serde_json::json!("C21"),
        "got: {v}"
    );
    let got = read(dir, "src/lib.rs");
    assert_eq!(got, content, "worktree must be untouched");
}

#[test]
fn destructuring_single_leaf_completes() {
    // Residue slice 2: a Stmt single-leaf destructuring rename enters
    // binder identity (header + post-use rewrite, `verified`).
    let dir = init_repo();
    let content = "pub fn run() {\n    let (a, old) = (1, 2);\n    let y = old + 1;\n    let _ = (a, y);\n}\n";
    cargo_crate(dir.path(), content);
    let patch = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# expr E\n- let (a, old) = E;\n+ let (a, new) = E;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let (a, new) = (1, 2);"), "got: {post}");
    assert!(post.contains("let y = new + 1;"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn destructuring_single_leaf_shadowed_reports_partial() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let (a, old) = (1, 2);\n    let y = old;\n    {\n        let (b, old) = (3, 4);\n        let z = old;\n        let _ = (a, b, y, z);\n    }\n    let w = old;\n    let _ = (a, y, w);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# expr E\n- let (a, old) = E;\n+ let (a, new) = E;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
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
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let (a, new) = (1, 2);"), "got: {post}");
    assert!(post.contains("let (b, old) = (3, 4);"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn destructuring_multi_leaf_still_fails_c21() {
    // Two leaves move: no binder identity, still fail-loud.
    let dir = init_repo();
    let content = "pub fn run() {\n    let (a, b) = (1, 2);\n    let _ = (a, b);\n}\n";
    cargo_crate(dir.path(), content);
    let patch = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# expr E\n- let (a, b) = E;\n+ let (c, d) = E;\n";
    check_c21_text(dir.path(), patch, content);
    check_c21_json(dir.path(), patch, content);
}

#[test]
fn destructuring_if_let_rewrite_fails_c21() {
    let dir = init_repo();
    let content = "pub fn run(opt: Option<u32>) {\n    if let Some(old) = opt { println!(\"{}\", old); }\n}\n";
    cargo_crate(dir.path(), content);
    let patch = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# expr E\n- if let Some(old) = E { ... }\n+ if let Some(new) = E { ... }\n";
    check_c21_text(dir.path(), patch, content);
    check_c21_json(dir.path(), patch, content);
}

#[test]
fn attr_strip_plus_rename_fails_c21() {
    // Attr envelopes break binder identity (minus/plus attrs differ),
    // so strip + rename was the same hole in disguise: strip-only
    // hunks still apply (no binder change), the combo fails at C21.
    let dir = init_repo();
    let content = "pub fn run() {\n    #[allow(x)]\n    let x = 1;\n}\n";
    cargo_crate(dir.path(), content);
    let patch = "diff --git src/lib.rs\n@@ -2,2 @@ rule=r\n# attrs=strip\n# expr E\n- #[allow(x)]\n- let x = E;\n+ let y = E;\n";
    check_c21_text(dir.path(), patch, content);
}

#[test]
fn let_rename_leaves_attribute_value_and_reports_partial() {
    // Attribute values never resolve to the binder: `#[doc = total]`
    // stays verbatim while the binder and the plain use rewrite; the
    // leftover surfaces as a `macro-hidden` orphan (never silent
    // `verified`).
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let total = 1;\n    #[doc = total]\n    let y = total;\n    let _ = y;\n}\n",
    );
    let v = check_json(dir.path(), LET_SIMPLE_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    let orphans = v["rules"][0]["orphans"].as_array().unwrap();
    assert_eq!(orphans.len(), 1, "got: {v}");
    assert_eq!(orphans[0]["line"], serde_json::json!(3), "got: {v}");
    assert_eq!(
        orphans[0]["reason"],
        serde_json::json!("macro-hidden"),
        "got: {v}"
    );
    apply_patch(dir.path(), LET_SIMPLE_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let sum = 1;"), "got: {post}");
    assert!(post.contains("#[doc = total]"), "got: {post}");
    assert!(post.contains("let y = sum;"), "got: {post}");
}

#[test]
fn closure_param_rename_leaves_attribute_value_and_reports_partial() {
    // Non-let twin: the closure body use rewrites, the `#[doc = old]`
    // value stays verbatim, and the leftover is an `off-target`
    // orphan — uniform with the `#[old]` path twin.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let f = |total: u32| { #[doc = total] total + 1 };\n    let _ = f;\n}\n",
    );
    let v = check_json(dir.path(), CLOSURE_PARAM_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    let orphans = v["rules"][0]["orphans"].as_array().unwrap();
    assert!(
        orphans
            .iter()
            .any(|o| o["reason"] == serde_json::json!("off-target")),
        "got: {v}"
    );
    apply_patch(dir.path(), CLOSURE_PARAM_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("|sum: u32|"), "got: {post}");
    assert!(post.contains("#[doc = total]"), "got: {post}");
    assert!(post.contains("sum + 1"), "got: {post}");
}

// Sequence-contained renames (residue slices 1-3): a rename-once
// `let old` -> `let new` (plain or single-leaf destructuring) at any
// single position completes post-window uses in-hunk and reports
// `verified`; shadowed/macro post-window uses file orphans and cap at
// `partial`; authored init-substitution (`let b = old` -> `let b = new`)
// completes in-window by authoring; spelled-`old` in-window uses replay
// verbatim and strand as orphans (`partial`); partial swaps and
// `new`/pat mentions stay declined window-only (pinned remainder).
const SEQ_RENAME_LAST_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let a = E0;\n- let old = E1;\n+ let a = E0;\n+ let new = E1;\n";

#[test]
fn sequence_final_rename_completes_and_checks_clean() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let a = 1;\n    let old = 2;\n    let c = old + 1;\n    let _ = (a, c);\n}\n",
    );
    let v = check_json(dir.path(), SEQ_RENAME_LAST_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), SEQ_RENAME_LAST_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let c = new + 1;"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn sequence_final_rename_shadowed_reports_partial() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let a = 1;\n    let old = 2;\n    let b = old;\n    {\n        let old = 9;\n        let c = old;\n        let _ = c;\n    }\n    let d = old;\n    let _ = (a, b, d);\n}\n",
    );
    let v = check_json(dir.path(), SEQ_RENAME_LAST_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
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
    apply_patch(dir.path(), SEQ_RENAME_LAST_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let b = new;"), "got: {post}");
    assert!(post.contains("let old = 9;"), "got: {post}");
    assert!(post.contains("let c = old;"), "got: {post}");
    assert!(post.contains("let d = new;"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn sequence_final_rename_macro_reports_partial() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let a = 1;\n    let old = 2;\n    println!(\"{}\", old);\n    let d = old;\n    let _ = (a, d);\n}\n",
    );
    let v = check_json(dir.path(), SEQ_RENAME_LAST_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert!(
        v["rules"][0]["orphans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["reason"] == serde_json::json!("macro-hidden")),
        "got: {v}"
    );
    apply_patch(dir.path(), SEQ_RENAME_LAST_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("println!(\"{}\", old);"), "got: {post}");
    assert!(post.contains("let d = new;"), "got: {post}");
    // No `cargo_check`: the `macro-hidden` leftover use dangles by
    // design (the attribute-value partial twin pins the same posture).
}

#[test]
fn sequence_non_final_rename_completes_and_checks_clean() {
    // Residue slice 2: rename-once at any position completes.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = 1;\n    let b = 2;\n    let c = old;\n    let _ = (b, c);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 1;"), "got: {post}");
    assert!(post.contains("let c = new;"), "got: {post}");
    assert!(!post.contains("old"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn sequence_spelled_substitution_completes_and_checks_clean() {
    // Residue slice 3: the author spells the intra-window use with the
    // new name; window applies verbatim, post-window completes,
    // `verified`.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = 1;\n    let b = old;\n    let c = old;\n    let _ = (b, c);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = old;\n+ let new = E0;\n+ let b = new;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 1;"), "got: {post}");
    assert!(post.contains("let b = new;"), "got: {post}");
    assert!(post.contains("let c = new;"), "got: {post}");
    assert!(!post.contains("old"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn sequence_spelled_old_strands_partial() {
    // Residue slice 3: the in-window use replays the old name (no
    // substitution spelled); post-window still completes and the
    // leftover strands as an orphan, capping at `partial`.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = 1;\n    let b = old;\n    let c = old;\n    let _ = (b, c);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = old;\n+ let new = E0;\n+ let b = old;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    // The narrowed sequence cover surfaces the replayed in-window use
    // (line 3) instead of hiding it inside the window edit.
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([{
            "file": "src/lib.rs",
            "line": 3,
            "reason": "off-target",
            "text": "let b = old;",
        }]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 1;"), "got: {post}");
    assert!(post.contains("let b = old;"), "got: {post}");
    assert!(post.contains("let c = new;"), "got: {post}");
}

#[test]
fn sequence_destructuring_position_completes_and_checks_clean() {
    // Residue slice 3: single-leaf destructuring at a sequence
    // position completes post-window uses, `verified`.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let (x, old) = (1, 2);\n    let b = 3;\n    let c = old;\n    let _ = (x, b, c);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let (x, old) = E0;\n- let b = E1;\n+ let (x, new) = E0;\n+ let b = E1;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let (x, new) = (1, 2);"), "got: {post}");
    assert!(post.contains("let c = new;"), "got: {post}");
    assert!(!post.contains("old"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn sequence_destructuring_shadowed_reports_partial() {
    // Residue slice 3: the narrowed cover keeps post-window shadow
    // precision for destructuring defs too.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let (x, old) = (1, 2);\n    let b = old;\n    {\n        let old = 9;\n        let c = old;\n        let _ = c;\n    }\n    let d = old;\n    let _ = (x, b, d);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let (x, old) = E0;\n- let b = old;\n+ let (x, new) = E0;\n+ let b = new;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
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
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let (x, new) = (1, 2);"), "got: {post}");
    assert!(post.contains("let b = new;"), "got: {post}");
    assert!(post.contains("let old = 9;"), "got: {post}");
    assert!(post.contains("let d = new;"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn sequence_multi_substitution_completes_and_checks_clean() {
    // Residue slice 3: several later in-window uses spelled with the
    // new name all complete in-window plus post-window, `verified`.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = 1;\n    let b = old;\n    let c = old;\n    let d = old;\n    let _ = (b, c, d);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = old;\n- let c = old;\n+ let new = E0;\n+ let b = new;\n+ let c = new;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 1;"), "got: {post}");
    assert!(post.contains("let b = new;"), "got: {post}");
    assert!(post.contains("let c = new;"), "got: {post}");
    assert!(post.contains("let d = new;"), "got: {post}");
    assert!(!post.contains("old"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn sequence_substitution_hole_block_surfaces_shadowed() {
    // Hole-hides-shadow precision: the hole-bound block keeps its inner
    // binder uses; they surface as `shadowed` instead of staying
    // silent, post-window still completes, `partial`.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = g(1);\n    let b = f(old, { let old = 1; old });\n    let c = old;\n    let _ = (b, c);\n}\npub fn g(x: i32) -> i32 { x }\npub fn f(x: i32, y: i32) -> i32 { x + y }\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = f(old, E1);\n+ let new = E0;\n+ let b = f(new, E1);\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([{
            "file": "src/lib.rs",
            "line": 3,
            "reason": "shadowed",
            "text": "let b = f(old, { let old = 1; old });",
        }]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = g(1);"), "got: {post}");
    assert!(
        post.contains("let b = f(new, { let old = 1; old });"),
        "got: {post}"
    );
    assert!(post.contains("let c = new;"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn sequence_substitution_hole_macro_surfaces_macro_hidden() {
    // Hole-bound macro interior likewise surfaces instead of hiding.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = g(1);\n    let b = f(old, foo!(old));\n    let c = old;\n    let _ = (b, c);\n}\npub fn g(x: i32) -> i32 { x }\npub fn f(x: i32, y: i32) -> i32 { x + y }\nmacro_rules! foo { ($x:expr) => { $x } }\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = f(old, E1);\n+ let new = E0;\n+ let b = f(new, E1);\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([{
            "file": "src/lib.rs",
            "line": 3,
            "reason": "macro-hidden",
            "text": "let b = f(old, foo!(old));",
        }]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = g(1);"), "got: {post}");
    assert!(post.contains("let c = new;"), "got: {post}");
}

#[test]
fn sequence_outer_scope_new_before_reports_capture_risk() {
    // Slice 3 remainder: a pre-existing `new` binder in the window's own
    // scope declines the follow arm — the window renames, the post-window
    // use strands `off-target`, and the colliding definition surfaces as
    // `capture-risk` (`partial`, still builds: the rename shadows
    // legally, but the agent must resolve the twin binders by hand).
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let new = 0;\n    let old = 2;\n    let b = 3;\n    let c = old;\n    let _ = (new, b, c);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([
            {
                "file": "src/lib.rs",
                "line": 2,
                "reason": "capture-risk",
                "text": "let new = 0;",
            },
            {
                "file": "src/lib.rs",
                "line": 5,
                "reason": "off-target",
                "text": "let c = old;",
            },
        ]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 0;"), "got: {post}");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let c = old;"), "got: {post}");
    // No `cargo_check`: the stranded post-window use dangles by design
    // (the arm declined; the spelled-`old` partial twin pins the same
    // posture).
}

#[test]
fn sequence_outer_scope_new_after_reports_capture_risk() {
    // A later same-level `let new` double-binds the scope: same decline
    // and `capture-risk` at the later definition.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = 2;\n    let b = 3;\n    let c = old;\n    let new = 9;\n    let _ = (b, c, new);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([
            {
                "file": "src/lib.rs",
                "line": 4,
                "reason": "off-target",
                "text": "let c = old;",
            },
            {
                "file": "src/lib.rs",
                "line": 5,
                "reason": "capture-risk",
                "text": "let new = 9;",
            },
        ]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let c = old;"), "got: {post}");
    assert!(post.contains("let new = 9;"), "got: {post}");
    // No `cargo_check`: the stranded post-window use dangles by design
    // (same posture as the before-window twin above).
}

#[test]
fn sequence_sibling_scope_new_still_verifies() {
    // A `new` binder in a descendant scope never collides: the arm still
    // fires, post-window completes, `verified`.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = 2;\n    let b = 3;\n    let _sib = { let new = 1; new };\n    let c = old;\n    let _ = (b, c);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let c = new;"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn single_let_outer_new_before_reports_capture_risk() {
    // Single-`let` parity with the sequence guard: a pre-existing `new`
    // binder in the renamed statement's own scope declines follow — the
    // definition renames, the post-binder use strands `off-target`, and
    // the colliding definition surfaces as `capture-risk` (`partial`).
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let new = 0;\n    let old = 2;\n    let c = old;\n    let _ = (new, c);\n}\n",
    );
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E;\n+ let new = E;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([
            {
                "file": "src/lib.rs",
                "line": 2,
                "reason": "capture-risk",
                "text": "let new = 0;",
            },
            {
                "file": "src/lib.rs",
                "line": 4,
                "reason": "off-target",
                "text": "let c = old;",
            },
        ]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 0;"), "got: {post}");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let c = old;"), "got: {post}");
}

#[test]
fn single_let_outer_new_after_reports_capture_risk() {
    // A later same-level `let new` double-binds the scope: same decline
    // and `capture-risk` at the later definition.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = 2;\n    let c = old;\n    let new = 9;\n    let _ = (c, new);\n}\n",
    );
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E;\n+ let new = E;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([
            {
                "file": "src/lib.rs",
                "line": 3,
                "reason": "off-target",
                "text": "let c = old;",
            },
            {
                "file": "src/lib.rs",
                "line": 4,
                "reason": "capture-risk",
                "text": "let new = 9;",
            },
        ]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let c = old;"), "got: {post}");
    assert!(post.contains("let new = 9;"), "got: {post}");
}

#[test]
fn single_let_fn_param_new_reports_capture_risk() {
    // A `new` fn param owns the whole body scope: same decline, with the
    // signature line named as the colliding definition.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run(new: u32) {\n    let old = 2;\n    let c = old;\n    let _ = (new, c);\n}\n",
    );
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E;\n+ let new = E;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([
            {
                "file": "src/lib.rs",
                "line": 1,
                "reason": "capture-risk",
                "text": "pub fn run(new: u32) {",
            },
            {
                "file": "src/lib.rs",
                "line": 3,
                "reason": "off-target",
                "text": "let c = old;",
            },
        ]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let c = old;"), "got: {post}");
}

#[test]
fn single_let_destructuring_outer_new_reports_capture_risk() {
    // The single-leaf destructuring twin shares `BinderScope::Let`, so it
    // shares the guard and the `capture-risk` evidence.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let new = 0;\n    let (a, old) = (1, 2);\n    let c = old;\n    let _ = (new, a, c);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let (a, old) = E;\n+ let (a, new) = E;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["orphans"],
        serde_json::json!([
            {
                "file": "src/lib.rs",
                "line": 2,
                "reason": "capture-risk",
                "text": "let new = 0;",
            },
            {
                "file": "src/lib.rs",
                "line": 4,
                "reason": "off-target",
                "text": "let c = old;",
            },
        ]),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 0;"), "got: {post}");
    assert!(post.contains("let (a, new) = (1, 2);"), "got: {post}");
    assert!(post.contains("let c = old;"), "got: {post}");
}

#[test]
fn single_let_no_outer_new_still_verifies() {
    // No pre-existing `new`: byte-identical behaviour (`verified` +
    // follow, no new orphans).
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let old = 2;\n    let c = old;\n    let _ = c;\n}\n",
    );
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- let old = E;\n+ let new = E;\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("let new = 2;"), "got: {post}");
    assert!(post.contains("let c = new;"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn closure_param_outer_new_still_verifies() {
    // Non-collision control: a renamed closure param shadows an outer
    // `let new` only inside the closure body, so outer uses never rebind
    // and no `capture-risk` fires (`verified` + body follow).
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub fn run() {\n    let new = 0;\n    let f = |old: u32| old + 1;\n    let _ = (f, new);\n}\n",
    );
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- |old: u32| E\n+ |new: u32| E\n";
    let v = check_json(dir.path(), patch);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), patch);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("|new: u32| new + 1"), "got: {post}");
    assert!(post.contains("let new = 0;"), "got: {post}");
    cargo_check(dir.path());
}
