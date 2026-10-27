//! T310: engine-verbatim whitespace-in-metavar-name fence (SPEC 9.4/25.3).
//!
//! A space-separated name list (`# expr E F`) is a pattern error, not two
//! declarations: `RSMPL_PATTERN_CONFLICT` with the verbatim
//! `invalid metavariable name ... in: ...` reason on a parse-time bare
//! envelope (text: bare `reason:` only; JSON: explicit nulls). The gate
//! fires per comma-segment before charset validation, so it is distinct
//! from the Section 9.6 `invalid name` gates (single bad identifier after
//! the split). The comma form (`# expr E, F`) is the fix. A closing
//! rot-gate pins the SPEC 9.4 wording. Worktree untouched throughout.

mod common;
use common::*;

fn check_text(patch: &str, reason: &str) {
    let dir = init_repo();
    let content = "fn f() {}\n";
    write(dir.path(), "a.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content, "worktree must be untouched");
}

fn check_json(patch: &str, reason: &str) {
    let dir = init_repo();
    let content = "fn f() {}\n";
    write(dir.path(), "a.rs", content);
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
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content, "worktree must be untouched");
}

#[test]
fn space_separated_names_rejected_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E F\n- foo(E)\n+ bar(E)\n";
    let reason = "invalid metavariable name `E F` in: # expr E F";
    check_text(patch, reason);
    check_json(patch, reason);
}

#[test]
fn space_separated_names_echo_constraint_tail_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E F = foo\n- foo(E)\n+ bar(E)\n";
    let reason = "invalid metavariable name `E F` in: # expr E F = foo";
    check_text(patch, reason);
    check_json(patch, reason);
}

#[test]
fn spec_94_carries_whitespace_fence_verbatim() {
    // Rot-gate: SPEC 9.4 must carry the fence exactly as the engine emits
    // it, and must keep pointing the fix at the comma form.
    let root = workspace_root();
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md readable");
    assert!(
        spec.contains("invalid metavariable name `E F` in: # expr E F"),
        "SPEC 9.4 must carry the verbatim whitespace-in-name fence"
    );
    assert!(
        spec.contains("Write `# expr E, F`,"),
        "SPEC 9.4 must keep the comma-form fix"
    );
    assert!(
        spec.contains("distinct from the Section 9.6 `invalid name` gates"),
        "SPEC 9.4 must keep the 9.6-contrast note"
    );
}
