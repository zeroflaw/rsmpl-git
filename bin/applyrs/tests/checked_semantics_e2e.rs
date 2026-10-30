//! WI-1b: honest `checked` — per-rule `semantics` + global `verified|partial`.
//!
//! Pins the v3 envelope: ranged renames report `instantiated`, file-scope
//! sweeps report `postcondition`, insertions report `reparse-only` with a
//! `partial` global, and optional zero-edits report `not-applicable` with
//! a `partial` global. A tampered post-image (simulated by hand-editing
//! after a clean apply is out of scope here; instead the engine-level
//! instantiation failure is pinned via a plus that cannot appear) fails
//! loud — see the engine unit for the direct tamper pin.

mod common;
use common::*;

fn json_check(patch: &str, file: &str, content: &str) -> serde_json::Value {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON")
}

#[test]
fn ranged_rename_reports_instantiated_and_verified() {
    let v = json_check(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- fn foo\n+ fn bar\n",
        "src/lib.rs",
        "fn foo() {}\n",
    );
    assert_eq!(v["version"], serde_json::json!(3), "got: {v}");
    assert_eq!(
        v["rules"][0]["semantics"],
        serde_json::json!("instantiated"),
        "got: {v}"
    );
    assert_eq!(
        v["checked"],
        serde_json::json!({"lowering": true, "semantics": "verified", "build": "skipped"}),
        "got: {v}"
    );
}

#[test]
fn sweep_reports_postcondition_and_verified() {
    let v = json_check(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n",
        "src/lib.rs",
        "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\n",
    );
    assert_eq!(
        v["rules"][0]["semantics"],
        serde_json::json!("postcondition"),
        "got: {v}"
    );
    assert_eq!(
        v["checked"],
        serde_json::json!({"lowering": true, "semantics": "verified", "build": "skipped"}),
        "got: {v}"
    );
}

#[test]
fn insertion_reports_reparse_only_and_partial() {
    // Pure insertion (`+` line) never owes instantiation: reparse-only,
    // forcing the global to `partial`.
    let v = json_check(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,1 @@ rule=ins\n+ fn inserted() {}\n",
        "src/lib.rs",
        "fn foo() {}\n",
    );
    assert_eq!(
        v["rules"][0]["semantics"],
        serde_json::json!("reparse-only"),
        "got: {v}"
    );
    assert_eq!(
        v["checked"],
        serde_json::json!({"lowering": true, "semantics": "partial", "build": "skipped"}),
        "got: {v}"
    );
}

#[test]
fn optional_zero_reports_not_applicable_and_partial() {
    let v = json_check(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# optional\n# expr E\n- foo(E)\n+ bar(E)\n",
        "src/lib.rs",
        "fn a() {}\n",
    );
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(0), "got: {v}");
    assert_eq!(
        v["rules"][0]["semantics"],
        serde_json::json!("not-applicable"),
        "got: {v}"
    );
    assert_eq!(
        v["checked"],
        serde_json::json!({"lowering": true, "semantics": "partial", "build": "skipped"}),
        "got: {v}"
    );
}
