//! T275: engine-verbatim zero-match diagnostics (SPEC 25.4): the T67
//! candidate preview appends to `reason:`, the zero-discovery retry
//! appends to `suggestion:`.
//!
//! All strings below are pinned verbatim in text mode and in the JSON
//! envelope (rule/file/range set on these paths — no nulls). The
//! worktree stays untouched in every case.

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

const BASE_SUGGESTION: &str =
    "read `src/lib.rs` to list valid ranges; check the pattern or add `# optional`";

#[test]
fn same_kind_preview_is_verbatim_text_and_json() {
    let content = "fn foo() {}\nfn bar() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=rename_fn\n# lang=rust\n- fn baz(...) {\n+ fn qux(...) {\n      ...\n  }\n";
    let reason = "no match found — nearest fn bar at line 2: 'fn bar() {}'";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: rename_fn"), "got: {stderr}");
    assert!(stderr.contains("file: src/lib.rs"), "got: {stderr}");
    assert!(stderr.contains("old_range: 1..3"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains(BASE_SUGGESTION), "got: {stderr}");
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, content, "worktree must be untouched");
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(BASE_SUGGESTION));
    assert_eq!(v["error"]["rule"], serde_json::json!("rename_fn"));
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert_eq!(v["error"]["range"], serde_json::json!("1..3"));
}

#[test]
fn nearest_source_preview_is_verbatim_text_and_json() {
    let content = "fn f() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_calls\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let reason = "no match found — nearest source at line 1: 'fn f() {}'";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: rename_calls"), "got: {stderr}");
    assert!(stderr.contains("old_range: 1..2"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains(BASE_SUGGESTION), "got: {stderr}");
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, content, "worktree must be untouched");
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(BASE_SUGGESTION));
    assert_eq!(v["error"]["range"], serde_json::json!("1..2"));
}

#[test]
fn exactly_one_elsewhere_appends_to_suggestion() {
    let content = "fn m() {\n    let x = 1;\n    foo(1);\n}\n";
    let patch =
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=rename_calls\n# expr E\n- foo(E)\n+ bar(E)\n";
    let reason = "no match found — nearest source at line 2: 'let x = 1;'";
    let suggestion = "read `src/lib.rs` to list valid ranges; check the pattern or add `# optional` — exactly one match exists elsewhere in the file; retry with `# scope=file` + `@@ -1,1 @@`";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("old_range: 2..3"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains(suggestion), "got: {stderr}");
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, content, "worktree must be untouched");
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
    assert_eq!(v["error"]["range"], serde_json::json!("2..3"));
}

#[test]
fn multiple_elsewhere_appends_to_suggestion() {
    let content = "fn m() {\n    let x = 1;\n    foo(1);\n    foo(2);\n}\n";
    let patch =
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=rename_calls\n# expr E\n- foo(E)\n+ bar(E)\n";
    let reason = "no match found — nearest source at line 2: 'let x = 1;'";
    let suggestion = "read `src/lib.rs` to list valid ranges; check the pattern or add `# optional` — multiple matches exist outside this range; widen the range (or add `# scope=file`) and add `# allow=many` if intended";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("old_range: 2..3"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains(suggestion), "got: {stderr}");
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, content, "worktree must be untouched");
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
    assert_eq!(v["error"]["range"], serde_json::json!("2..3"));
}
