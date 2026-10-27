//! T303: engine-verbatim fail-fast reasons for the empty-directive /
//! second-invalid-name, self-cycle capture, and outside-section / empty-patch
//! gates (SPEC 9.6 / 20.3 / 25.2 / 25.6).
//!
//! The directive reasons fire at parse time: bare `reason:` in text mode,
//! explicit nulls in the JSON envelope. The self-cycle capture reason is a
//! later-stage rule+file failure with null range/suggestion (contrast the
//! missing-capture fence, which sets `suggestion:`). The outside-section
//! twins are parse-time bare failures; the empty-patch failure is bare
//! plus a set suggestion. The worktree stays untouched in every case.

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

fn assert_bare_envelope(stderr: &str, code: &str, reason: &str) {
    assert!(stderr.contains(code), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
}

fn assert_bare_json(v: &serde_json::Value, code: &str, reason: &str) {
    assert_eq!(v["error"]["code"], serde_json::json!(code), "got: {v}");
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn empty_directive_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n#\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let reason = "empty directive: #";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert_bare_envelope(&stderr, "RSMPL_PATTERN_CONFLICT", reason);
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_bare_json(&v, "RSMPL_PATTERN_CONFLICT", reason);
}

#[test]
fn invalid_name_charset_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr foo-bar\n- foo(E)\n+ bar(E)\n";
    let reason = "invalid name `foo-bar` (only alphanumeric + _ allowed)";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert_bare_envelope(&stderr, "RSMPL_PATTERN_CONFLICT", reason);
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_bare_json(&v, "RSMPL_PATTERN_CONFLICT", reason);
}

#[test]
fn self_cycle_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# item I\n# bind=H\n# use=H\n- I\n";
    let reason = "hunk both binds and uses `H` (self-cycle)";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert!(
        stderr.contains("RSMPL_CAPTURE_CONFLICT"),
        "got: {stderr}"
    );
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_CAPTURE_CONFLICT"),
        "got: {v}"
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"), "got: {v}");
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn file_metadata_outside_section_verbatim() {
    let patch = "--- a/a.rs\n";
    let reason = "file metadata outside file section: --- a/a.rs";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert_bare_envelope(&stderr, "RSMPL_PARSE_CONFLICT", reason);
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_bare_json(&v, "RSMPL_PARSE_CONFLICT", reason);
}

#[test]
fn unexpected_line_outside_section_verbatim() {
    let patch = "hello\n";
    let reason = "unexpected line outside file section: hello";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert_bare_envelope(&stderr, "RSMPL_PARSE_CONFLICT", reason);
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_bare_json(&v, "RSMPL_PARSE_CONFLICT", reason);
}

#[test]
fn empty_patch_verbatim() {
    let patch = "";
    let reason = "empty patch: no `diff --git` file sections";
    let suggestion = "provide at least one `diff --git a/.. b/..` file section with a hunk (see SPEC 6 minimal recipe); check for an empty patch input (stdin, --file, or --patch)";
    let stderr = check_text(patch, "a.rs", "fn f() {}\n");
    assert!(
        stderr.contains("RSMPL_PARSE_CONFLICT"),
        "got: {stderr}"
    );
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains(suggestion), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    let v = check_json(patch, "a.rs", "fn f() {}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT"),
        "got: {v}"
    );
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
}
