//! T282: engine-verbatim fail-fast reasons for the five unsupported
//! metavariable kinds (`identifier`/`path`/`lifetime`/`field` share one
//! template; `attributes` has its own reason pointing at SPEC 14.6).
//!
//! All strings below are pinned verbatim (rule + file lines in text
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
fn unsupported_kinds_share_one_template() {
    for (kind, name) in [
        ("identifier", "I"),
        ("path", "P"),
        ("lifetime", "L"),
        ("field", "F"),
    ] {
        let patch =
            format!("diff --git a.rs\n@@ -1,1 @@ rule=r\n# {kind} {name}\n- foo(E)\n+ bar(E)\n");
        let reason = format!(
            "`# {kind} {name}` is not supported in v0.1 (see SPEC 15.1 for the implemented kinds)"
        );
        let dir = init_repo();
        write(dir.path(), "a.rs", "fn foo() {}\n");
        let out = run_stdin(dir.path(), &patch, &["--check"]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "kind {kind}: stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
        assert!(stderr.contains(&reason), "got: {stderr}");
        assert!(stderr.contains("rule: r"), "got: {stderr}");
        assert!(stderr.contains("file: a.rs"), "got: {stderr}");
        assert!(!stderr.contains("old_range:"), "got: {stderr}");
        assert!(!stderr.contains("suggestion:"), "got: {stderr}");
        let got = read(dir.path(), "a.rs");
        assert_eq!(got, "fn foo() {}\n");
        let v = check_json(&patch, "a.rs", "fn foo() {}\n");
        assert_eq!(
            v["error"]["code"],
            serde_json::json!("RSMPL_PATTERN_CONFLICT")
        );
        assert_eq!(v["error"]["rule"], serde_json::json!("r"));
        assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
        assert_eq!(v["error"]["reason"], serde_json::json!(reason));
        assert!(v["error"]["range"].is_null(), "got: {v}");
        assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    }
}

#[test]
fn attributes_rejection_is_distinct() {
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# attributes A\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let reason = "`# attributes A` is not supported in v0.1 (attribute lists cannot be abstracted; write the `#[...]` lines literally on both sides to constrain them, or omit them \u{2014} attributes are preserved by default, SPEC 14.6)";
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn foo() {}\n");
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
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, "fn foo() {}\n");
    let v = check_json(patch, "a.rs", "fn foo() {}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}
