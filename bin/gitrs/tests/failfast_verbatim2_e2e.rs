//! T273: engine-verbatim fail-fast reasons for `+ ...` replay, path
//! containment, and `use`-prefix identity (SPEC 16.3/25.3/25.8/27.6/14.9).
//!
//! The engine strings below are pinned verbatim (rule + file lines in
//! text mode, explicit nulls in the JSON envelope). The worktree stays
//! untouched in every case.

mod common;
use common::*;

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

#[test]
fn plus_ellipsis_without_minus_replay_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E, ...)\n";
    let reason = "`...` in `+` has no matching `- ...` to replay";
    let suggestion = "drop the `+ ...` or add a `- ...` absorbing the replayed args";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn f() {\n    foo(1);\n}\n");
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
    assert!(stderr.contains(suggestion), "got: {stderr}");
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, "fn f() {\n    foo(1);\n}\n");
    let v = check_json(patch, "src/lib.rs", "fn f() {\n    foo(1);\n}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
    assert!(v["error"]["range"].is_null(), "got: {v}");
}

#[test]
fn absolute_target_path_rejected_verbatim() {
    let patch = "diff --git /tmp/evil.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n+ use foo;\n";
    let reason =
        "target path is absolute (/tmp/evil.rs); use a repo-relative path without leading /";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn f() {}\n");
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let v = check_json(patch, "src/lib.rs", "fn f() {}\n");
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn escaping_target_path_rejected_verbatim() {
    let patch = "diff --git ../evil.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n+ use foo;\n";
    let reason = "target path escapes the repo (../evil.rs); use a repo-relative path without ../";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn f() {}\n");
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let v = check_json(patch, "src/lib.rs", "fn f() {}\n");
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn use_prefix_identical_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# allow=many\n- use crate::old\n+ use crate::old\n";
    let reason =
        "`use`-path rename minus and plus prefixes are identical (`crate::old`); write the new path on the `+` side";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "use crate::old::A;\n");
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
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, "use crate::old::A;\n");
    let v = check_json(patch, "src/lib.rs", "use crate::old::A;\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
}
