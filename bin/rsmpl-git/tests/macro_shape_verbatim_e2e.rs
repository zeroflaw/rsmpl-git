//! T290: engine-verbatim fail-fast reasons for non-`NAME!(...)`
//! macro patterns and non-verbatim `# macro=token_tree` bodies
//! (SPEC 21.3).
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
fn non_macro_shape_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,3 @@ rule=m\n# macro=token_tree\n- foo(E)\n+ bar(E)\n";
    let reason = "v0.1 macro patterns must have `NAME!(...)` shape, got `foo(E)`";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {\n    old!(1, 2);\n}\n");
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
    assert_eq!(got, "fn a() {\n    old!(1, 2);\n}\n");
    let v = check_json(patch, "src/lib.rs", "fn a() {\n    old!(1, 2);\n}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("m"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn token_tree_nonverbatim_body_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,3 @@ rule=m\n# macro=token_tree\n# expr E\n- old!(E)\n+ new!(E)\n";
    let reason = "v0.1 `# macro=token_tree` supports only `NAME!(...)` with verbatim body";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {\n    old!(1, 2);\n}\n");
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
    assert_eq!(got, "fn a() {\n    old!(1, 2);\n}\n");
    let v = check_json(patch, "src/lib.rs", "fn a() {\n    old!(1, 2);\n}\n");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("m"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}
