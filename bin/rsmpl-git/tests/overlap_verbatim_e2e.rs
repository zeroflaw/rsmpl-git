//! T279: engine-verbatim overlap-conflict pins (SPEC 10.6/23.5/25.7).
//!
//! Any overlapping edit pair — semantic-semantic, ordinary-semantic, or an
//! insertion strictly inside a deleted/replaced range — fails with the same
//! `GIT_HUNK_CONFLICT` shape (rule/range null, suggestion present), unlike
//! the ordinary-hunk content conflict (`rule: ""`, range present, suggestion
//! null). Disjoint edits merge normally (10.6 option 1). Worktree untouched
//! on every conflict path.

mod common;
use common::*;

const REASON: &str = "overlapping edits cannot be merged";
const SUGGESTION: &str =
    "apply the overlapping rewrites in separate runs, or make the ranges disjoint";

fn check_text(patch: &str, file: &str, content: &str) {
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
    assert!(stderr.contains("GIT_HUNK_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("file: src/lib.rs"), "got: {stderr}");
    assert!(stderr.contains(REASON), "got: {stderr}");
    assert!(stderr.contains(SUGGESTION), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

fn check_json(patch: &str, file: &str, content: &str) {
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
    assert_eq!(v["error"]["code"], serde_json::json!("GIT_HUNK_CONFLICT"));
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert_eq!(v["error"]["reason"], serde_json::json!(REASON));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(SUGGESTION));
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

#[test]
fn semantic_semantic_overlap_rejected_verbatim() {
    let content = "fn f(v: S) {\n    let a = v.old_method(1);\n}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=m2c\n# expr Receiver\n# expr Arg\n- Receiver.old_method(Arg)\n+ new_function(Receiver, Arg)\n\ndiff --git src/lib.rs\n@@ -2,1 @@ rule=letrw\n# stmt S\n# expr E\n- let a = E;\n+ let c = E;\n";
    check_text(patch, "src/lib.rs", content);
    check_json(patch, "src/lib.rs", content);
}

#[test]
fn ordinary_semantic_overlap_rejected_verbatim() {
    let content = "fn foo() {}\nfn bar() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,2 +1,2 @@\n-fn foo() {}\n-fn bar() {}\n+fn foo() {}\n+fn bar2() {}\n\ndiff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n# lang=rust\n- fn foo(...) {\n+ fn foo2(...) {\n      ...\n  }\n";
    check_text(patch, "src/lib.rs", content);
    check_json(patch, "src/lib.rs", content);
}

#[test]
fn insertion_inside_deleted_range_rejected_verbatim() {
    let content = "fn top() {}\nfn helper(x: i32) -> i32 {\n    x + 1\n}\nfn tail() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -3,0 @@ rule=paste\n# use=H\n+ H\n\ndiff --git src/lib.rs\n@@ -2,3 @@ rule=cut\n# item I\n# bind=H\n- I\n";
    check_text(patch, "src/lib.rs", content);
    check_json(patch, "src/lib.rs", content);
}

#[test]
fn disjoint_edits_merge_success_control() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\nfn bar() {}\n");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n# lang=rust\n- fn foo(...) {\n+ fn foo2(...) {\n      ...\n  }\n\ndiff --git src/lib.rs\n@@ -2,1 @@ rule=r2\n# lang=rust\n- fn bar(...) {\n+ fn bar2(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("fn foo2() {}"), "got: {got}");
    assert!(got.contains("fn bar2() {}"), "got: {got}");
}
