//! T311: engine-verbatim capture-miss suggestions (SPEC 25.6).
//!
//! Two branches of `capture.rs::get`: known-empty suggests
//! `add a hunk with `# bind=<Name>`` (literal `<Name>` placeholder,
//! not the missing name); known non-empty suggests
//! `available captures: <sorted ", "-joined>`. Both share the
//! `rule:`/`file:` envelope with null range and a SET suggestion
//! (text omits the null line, JSON explicit null). The worktree stays
//! untouched in every case.

mod common;
use common::*;

fn check(patch: &str, rule: &str, reason: &str, suggestion: &str) {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() {}\nfn b() {}\nfn c() {}\n");
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_CAPTURE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(&format!("rule: {rule}")), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(!stderr.contains("capture:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(
        stderr.contains(&format!("suggestion: {suggestion}")),
        "got: {stderr}"
    );
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, "fn a() {}\nfn b() {}\nfn c() {}\n");
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
        serde_json::json!("RSMPL_CAPTURE_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["rule"], serde_json::json!(rule));
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, "fn a() {}\nfn b() {}\nfn c() {}\n");
}

#[test]
fn empty_known_suggests_bind_placeholder_text_json() {
    check(
        "diff --git a.rs\n@@ -1,0 @@ rule=paste_missing\n# use=Missing\n+ Missing\n",
        "paste_missing",
        "capture `Missing` is not bound by any hunk",
        "add a hunk with `# bind=<Name>`",
    );
}

#[test]
fn nonempty_single_suggests_known_name_text_json() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=cut_ok\n# item I\n# bind=Known\n- I\n\ndiff --git a.rs\n@@ -2,0 @@ rule=paste_missing\n# use=Missing\n+ Missing\n",
        "paste_missing",
        "capture `Missing` is not bound by any hunk",
        "available captures: Known",
    );
}

#[test]
fn nonempty_multi_suggests_sorted_join_text() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() {}\nfn b() {}\nfn c() {}\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=cut_z\n# item I\n# bind=Zulu\n- I\n\ndiff --git a.rs\n@@ -2,1 @@ rule=cut_a\n# item I\n# bind=Alpha\n- I\n\ndiff --git a.rs\n@@ -3,0 @@ rule=paste_missing\n# use=Missing\n+ Missing\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_CAPTURE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: paste_missing"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(
        stderr.contains("capture `Missing` is not bound by any hunk"),
        "got: {stderr}"
    );
    assert!(
        stderr.contains("suggestion: available captures: Alpha, Zulu"),
        "got: {stderr}"
    );
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, "fn a() {}\nfn b() {}\nfn c() {}\n");
}

#[test]
fn spec_25_6_carries_both_suggestion_shapes() {
    let spec = std::fs::read_to_string(workspace_root().join("SPEC.md")).unwrap();
    assert!(
        spec.contains("suggestion: add a hunk with `# bind=<Name>`"),
        "25.6 empty-known fence drifted"
    );
    assert!(
        spec.contains("suggestion: available captures: Known"),
        "25.6 non-empty fence drifted"
    );
    assert!(
        !spec.contains("capture: Helper"),
        "stale capture: line back"
    );
}
