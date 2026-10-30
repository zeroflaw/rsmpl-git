//! T287: engine-verbatim param-`...`-mixing + matcher-stage partial-overlap pins.
//!
//! (a) Mixing `...` with other parameter-list tokens fails fast with one
//! verbatim `RSMPL_PATTERN_CONFLICT` reason (rule + file, null
//! range/suggestion). (b) The matcher-stage `partially overlapping
//! matches cannot be merged` reason shares one spelling across the call,
//! general-expression, general-type, and macro matchers
//! (`RSMPL_MATCH_CONFLICT`, rule + file, null range/suggestion) —
//! contrasting with the plan-stage `overlapping edits cannot be merged`
//! (`GIT_HUNK_CONFLICT`, file-only + suggestion, SPEC 10.6/23.5/25.7).
//! Any two AST nodes from one parse are nested or disjoint, so real Rust
//! inputs hit outermost-wins skip or disjoint-merge: the partial-overlap
//! reason is a defensive backstop with no real-Rust trigger, pinned here
//! by a source-spelling rot-gate plus a nested-input control that proves
//! nesting does NOT hit it. Worktree untouched on every conflict path.

mod common;
use common::*;

const PARAM_REASON: &str = "`...` in a function parameter list must stand alone: write `(...)` to match any parameters or fully-concrete parameters (SPEC 14.7)";
const PARTIAL_REASON: &str = "partially overlapping matches cannot be merged";

fn check_text(patch: &str, file: &str, content: &str) {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
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
    assert!(stderr.contains(PARAM_REASON), "got: {stderr}");
    assert!(!stderr.contains("range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

fn check_json(patch: &str, file: &str, content: &str) {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
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
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert_eq!(v["error"]["reason"], serde_json::json!(PARAM_REASON));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

#[test]
fn param_ellipsis_mixing_rejected_verbatim() {
    let content = "fn foo(x: i32) {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(x: i32, ...) {\n+ fn bar(x: i32, ...) {\n      ...\n  }\n";
    check_text(patch, "src/lib.rs", content);
    check_json(patch, "src/lib.rs", content);
}

#[test]
fn partial_overlap_spelling_shared_across_matchers() {
    // Rot-gate: the four matcher sites + SPEC 17.6 must carry one spelling.
    // (No real-Rust trigger exists — AST nodes nest or stay disjoint —
    // so this pins the wording, not a runtime path.)
    let root = workspace_root();
    for rel in [
        "crates/rsmpl_engine/src/matcher_expr.rs",
        "crates/rsmpl_engine/src/matcher_general_apply.rs",
        "crates/rsmpl_engine/src/matcher_macro.rs",
    ] {
        let text = std::fs::read_to_string(root.join(rel)).expect("engine source readable");
        assert!(
            text.contains(PARTIAL_REASON),
            "{rel} must carry the verbatim partial-overlap reason"
        );
    }
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md readable");
    assert!(
        spec.contains(PARTIAL_REASON),
        "SPEC 17.6 must carry the verbatim partial-overlap fence"
    );
    // And it must differ from the plan-stage overlap string.
    assert!(
        spec.contains("overlapping edits cannot be merged"),
        "SPEC must keep the plan-stage overlap fence for contrast"
    );
    assert_ne!(
        PARTIAL_REASON, "overlapping edits cannot be merged",
        "matcher-stage and plan-stage reasons must stay distinct"
    );
}

#[test]
fn nested_outer_wins_does_not_hit_partial_overlap() {
    // Control: nested same-rule matches resolve outermost-wins (no error),
    // proving nesting never reaches the partial-overlap backstop.
    let dir = init_repo();
    let content = "fn f() {\n    let x = Box::new(Box::new(1));\n}\n";
    write(dir.path(), "src/lib.rs", content);
    let patch = "diff --git src/lib.rs\n@@ -2,1 @@ rule=unbox\n# allow=many\n# expr E\n- Box::new(E)\n+ E\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "nested input must apply outermost-wins, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains(PARTIAL_REASON),
        "nested input must not hit the partial-overlap backstop"
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(out.status.success());
    let got = read(dir.path(), "src/lib.rs");
    assert!(
        got.contains("Box::new(1)"),
        "outer match wins without repeat, got: {got}"
    );
}
