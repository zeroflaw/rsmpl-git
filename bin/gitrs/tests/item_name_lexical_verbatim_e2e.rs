//! T312: engine-verbatim item-name lexical rejections (SPEC 14.8).
//!
//! SPEC 14.8 prose named `$Meta`/raw-identifier edges but neither
//! rejection was verbatim-pinned. Both fail at pattern-parse time:
//! `RSMPL_PATTERN_CONFLICT` (exit 2, `rule:`/`file:` set in text mode,
//! explicit `range: null` / `suggestion: null` in JSON, worktree
//! untouched). `{word}` echoes the offending spelling (`r#123` probed).

mod common;
use common::*;

fn check_ruled_null_envelope(patch: &str, reason: &str) {
    let file = "src/lib.rs";
    let base = "struct Old;\n";
    let dir = init_repo();
    write(dir.path(), file, base);
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
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert_eq!(read(dir.path(), file), base);
    let dir = init_repo();
    write(dir.path(), file, base);
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
    assert_eq!(read(dir.path(), file), base);
}

#[test]
fn meta_minus_rejected_verbatim() {
    check_ruled_null_envelope(
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- struct $Meta\n+ struct New\n",
        "`$Meta` abstracted names are not supported for item renames in v1 (write concrete names)",
    );
}

#[test]
fn meta_plus_rejected_verbatim() {
    check_ruled_null_envelope(
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- struct Old\n+ struct $Meta\n",
        "`$Meta` abstracted names are not supported for item renames in v1 (write concrete names)",
    );
}

#[test]
fn invalid_raw_ident_rejected_verbatim() {
    check_ruled_null_envelope(
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- struct r#123\n+ struct New\n",
        "invalid item name `r#123` (expected an identifier)",
    );
}

#[test]
fn spec_14_8_carries_both_lexical_fences() {
    let spec = std::fs::read_to_string(workspace_root().join("SPEC.md")).unwrap();
    assert!(
        spec.contains("`$Meta` abstracted names are not supported for item renames in v1 (write concrete names)"),
        "14.8 $Meta fence drifted"
    );
    assert!(
        spec.contains("invalid item name `r#123` (expected an identifier)"),
        "14.8 invalid-name fence drifted"
    );
}
