//! T283: engine-verbatim fail-fast reason for a valued `# repeat=<value>`
//! directive (SPEC 9.6 / 17.8 / 25.3).
//!
//! Unlike the 17.8 shape-specific repeat reasons (rule + file envelope),
//! a valued spelling fails at directive-parse time: no `rule:`/`file:`
//! lines in text mode, explicit nulls in the JSON envelope, with the
//! suggestion line set. The worktree stays untouched in every case.

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
fn repeat_value_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# expr E\n# repeat=many\n- foo(E)\n+ bar(E)\n";
    let reason = "invalid repeat value `many` (write bare `# repeat`): # repeat=many";
    let suggestion = "write bare `# repeat` with no value";
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
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, "fn f() {\n    foo(1);\n}\n");
    let v = check_json(patch, "src/lib.rs", "fn f() {\n    foo(1);\n}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
}
