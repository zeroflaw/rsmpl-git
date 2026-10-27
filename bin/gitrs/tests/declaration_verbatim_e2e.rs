//! T302: engine-verbatim fail-fast reasons for declaration-name,
//! standalone-`...`, and repeat-both gates (SPEC 9.6 / 16 / 17.8 / 25.3).
//!
//! The four declaration reasons fire at directive-parse time: bare
//! `reason:` in text mode, explicit nulls in the JSON envelope. The
//! standalone-wildcard reason is a later-stage rule+file failure with
//! null range/suggestion. The repeat-both reason is a later-stage
//! rule+file failure with a set suggestion and null range. The worktree
//! stays untouched in every case.

mod common;
use common::*;

fn check_text(patch: &str, file: &str, content: &str) -> String {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn check_json(patch: &str, file: &str, content: &str) -> serde_json::Value {
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
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    v
}

fn assert_bare_envelope(stderr: &str, reason: &str) {
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
}

fn assert_bare_json(v: &serde_json::Value, reason: &str) {
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
}

#[test]
fn missing_directive_name_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr\n- foo(E)\n+ bar(E)\n";
    let reason = "metavariable directive missing name: # expr";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert_bare_envelope(&stderr, reason);
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_bare_json(&v, reason);
}

#[test]
fn invalid_directive_name_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr 1bad\n- foo(1bad)\n+ bar(1bad)\n";
    let reason = "invalid name `1bad` (must start with letter or _)";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert_bare_envelope(&stderr, reason);
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_bare_json(&v, reason);
}

#[test]
fn empty_symbol_path_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# symbol Old=\n- Old(E)\n+ New(E)\n";
    let reason = "symbol path must not be empty: # symbol Old=";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert_bare_envelope(&stderr, reason);
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_bare_json(&v, reason);
}

#[test]
fn empty_capture_name_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# item I\n# bind=\n- I\n";
    let reason = "empty metavariable/capture name";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert_bare_envelope(&stderr, reason);
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_bare_json(&v, reason);
}

#[test]
fn standalone_wildcard_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E...E)\n+ bar(E)\n";
    let reason = "`...` must be a standalone wildcard, got ` foo(E...E)`";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"), "got: {v}");
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn repeat_requires_both_sides_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# repeat\n# expr E\n- foo(E)\n";
    let reason =
        "`# repeat` requires both `-` and `+` patterns (insertions/deletions already cover nested matches)";
    let suggestion = "remove `# repeat`, or write a rewrite hunk";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains(suggestion), "got: {stderr}");
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"), "got: {v}");
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
}
