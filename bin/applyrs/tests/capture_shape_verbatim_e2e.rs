//! T289: engine-verbatim capture-shape diagnostics (SPEC 20.3/25.6).
//!
//! The four `# bind=`/`# use=` shape rejections plus the
//! bind+`allow=many` envelope: all `RSMPL_CAPTURE_CONFLICT` with
//! `rule:`/`file:` lines, null range/suggestion in both modes
//! (suggestion null — contrast the missing-capture fence in 25.6,
//! which sets one). The worktree stays untouched in every case.

mod common;
use common::*;

const CONTENT: &str = "fn foo() {}\n";

fn check(patch: &str, reason: &str) {
    let dir = init_repo();
    write(dir.path(), "a.rs", CONTENT);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_CAPTURE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, CONTENT, "worktree must be untouched");
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
        serde_json::json!("RSMPL_CAPTURE_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, CONTENT, "worktree must be untouched");
}

#[test]
fn only_one_bind_per_hunk_is_verbatim() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# item I\n# bind=A\n# bind=B\n- I\n",
        "only one `# bind=<Name>` per hunk is supported in v0.1",
    );
}

#[test]
fn bind_only_on_item_capture_cut_is_verbatim() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n# bind=H\n- foo()\n+ bar()\n",
        "v0.1 `# bind=` is only supported on item-capture cut hunks (`- I`)",
    );
}

#[test]
fn use_only_on_pure_insertion_is_verbatim() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n# use=H\n- foo()\n+ bar()\n",
        "v0.1 `# use=` is only supported on pure insertion hunks (`+ Name`)",
    );
}

#[test]
fn duplicate_bind_echoes_name_verbatim() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# item I\n# bind=H\n- I\n\ndiff --git a.rs\n@@ -1,1 @@ rule=r\n# item I\n# bind=H\n- I\n",
        "duplicate `# bind=H` (capture names must be unique)",
    );
}

#[test]
fn bind_many_on_non_item_cut_is_verbatim() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n# bind=H\n# allow=many\n- foo(E)\n+ bar(E)\n",
        "v0.1 `# bind=` is only supported on item-capture cut hunks (`- I`)",
    );
}
