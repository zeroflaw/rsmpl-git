//! T284: engine-verbatim literal-token quoting rejections
//! (SPEC 15.4 / 24.1 / 25.3).
//!
//! Parse-time triple (raw-split in pattern, raw-split in directive,
//! unbalanced backticks): directive-parse stage, no `rule:`/`file:`
//! lines in text mode, explicit nulls in JSON. Later-stage quad (empty,
//! multi-token, invalid raw, invalid literal): pattern-parse stage,
//! `rule:`/`file:` set, null range/suggestion. All exit 2 with the
//! worktree untouched; all carry a null suggestion.

mod common;
use common::*;

const BASE: &str = "fn foo() {}\n";

fn check_text_null_envelope(patch: &str, reason: &str) {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BASE);
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
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), BASE);
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BASE);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    assert_eq!(read(dir.path(), "src/lib.rs"), BASE);
}

fn check_text_ruled_envelope(patch: &str, reason: &str) {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BASE);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: x"), "got: {stderr}");
    assert!(stderr.contains("file: src/lib.rs"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), BASE);
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BASE);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("x"));
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    assert_eq!(read(dir.path(), "src/lib.rs"), BASE);
}

#[test]
fn raw_split_in_pattern_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# type T\n- Result<T, r#`type`>\n+ X\n";
    let reason = "invalid raw-identifier quoting `r#` + backtick in pattern ` Result<T, r#`type`>` (use `` `r#...` ``)";
    check_text_null_envelope(patch, reason);
}

#[test]
fn raw_split_in_directive_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# expr r#`E`\n- foo(E)\n+ bar(E)\n";
    let reason = "invalid raw-identifier quoting in directive `# expr r#`E``";
    check_text_null_envelope(patch, reason);
}

#[test]
fn unbalanced_backtick_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# type T\n- Result<T, `E>\n+ X\n";
    let reason = "unbalanced backtick quoting in pattern ` Result<T, `E>`";
    check_text_null_envelope(patch, reason);
}

#[test]
fn empty_quote_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# type T\n- Result<T, ``>\n+ X\n";
    let reason = "empty backtick quote ```` in pattern";
    check_text_ruled_envelope(patch, reason);
}

#[test]
fn multi_token_quote_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# type T\n- Result<T, `foo bar`>\n+ X\n";
    let reason = "backtick quote must hold a single token, got `foo bar`";
    check_text_ruled_envelope(patch, reason);
}

#[test]
fn invalid_raw_ident_quote_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# type T\n- Result<T, `r#123`>\n+ X\n";
    let reason = "invalid raw identifier in backtick quote `r#123`";
    check_text_ruled_envelope(patch, reason);
}

#[test]
fn invalid_literal_quote_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# type T\n- Result<T, `123`>\n+ X\n";
    let reason = "invalid literal token in backtick quote `123`";
    check_text_ruled_envelope(patch, reason);
}
