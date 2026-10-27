//! T309: engine-verbatim bare-side mismatch reasons (SPEC 14.8/14.9).
//!
//! The item and `use` classifiers reject a pair where one side is a
//! recognized bare rename but the other side is a different shape.
//! All four are `RSMPL_PATTERN_CONFLICT` with rule+file lines in text
//! mode (null range/suggestion omitted) and explicit nulls in JSON.
//! Suggestion is null throughout; the worktree stays untouched.

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
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    assert!(
        v["error"]["suggestion"].is_null(),
        "null suggestion expected, got: {v}"
    );
    assert!(v["error"]["range"].is_null(), "got: {v}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    v
}

fn check_text(patch: &str, file: &str, content: &str, reason: &str) {
    let dir = init_repo();
    write(dir.path(), file, content);
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
    assert!(
        !stderr.contains("suggestion:"),
        "null suggestion must be omitted in text, got: {stderr}"
    );
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

#[test]
fn item_plus_side_must_be_bare_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- struct Old\n+ foo(E)\n";
    let reason =
        "item rename plus side must be a bare `<kind> <Name>` pattern matching the minus kind";
    check_text(patch, "src/lib.rs", "struct Old {}\n", reason);
    let v = check_json(patch, "src/lib.rs", "struct Old {}\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn item_minus_side_must_be_bare_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- foo(E)\n+ struct New\n";
    let reason =
        "item rename minus side must be a bare `<kind> <Name>` pattern matching the plus kind";
    check_text(patch, "src/lib.rs", "struct Old {}\n", reason);
    let v = check_json(patch, "src/lib.rs", "struct Old {}\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn use_plus_side_must_be_bare_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- use crate::old\n+ foo(E)\n";
    let reason =
        "`use`-path rename plus side must be a bare `use <path>` prefix matching the minus side";
    check_text(patch, "src/lib.rs", "use crate::old::Thing;\n", reason);
    let v = check_json(patch, "src/lib.rs", "use crate::old::Thing;\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn use_minus_side_must_be_bare_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- foo(E)\n+ use crate::new\n";
    let reason =
        "`use`-path rename minus side must be a bare `use <path>` prefix matching the plus side";
    check_text(patch, "src/lib.rs", "use crate::old::Thing;\n", reason);
    let v = check_json(patch, "src/lib.rs", "use crate::old::Thing;\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}
