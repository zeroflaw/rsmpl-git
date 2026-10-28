//! T281: engine-verbatim fail-fast reasons for `# mode=tokens` on a
//! non-macro hunk and for a `+`-side metavariable bound nowhere
//! (SPEC 25.3).
//!
//! Both strings below are pinned verbatim (rule + file lines in text
//! mode, explicit nulls in the JSON envelope). The worktree stays
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
fn mode_tokens_on_non_macro_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# mode=tokens\n- foo()\n+ bar()\n";
    let reason = "`# mode=tokens` is only supported for macro token-tree patterns in v0.1";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn f() {\n    foo();\n}\n");
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
    assert_eq!(got, "fn f() {\n    foo();\n}\n");
    let v = check_json(patch, "src/lib.rs", "fn f() {\n    foo();\n}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("x"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn unbound_plus_metavariable_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=y\n# expr E\n# expr F\n- foo(E)\n+ bar(F)\n";
    let reason = "metavariable(s) used in `+` pattern without binding: F";
    let suggestion = "bind them in `-`/context or declare `# use=`";
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
    assert_eq!(v["error"]["rule"], serde_json::json!("y"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
    assert!(v["error"]["range"].is_null(), "got: {v}");
}
