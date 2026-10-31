//! T291: engine-verbatim ordinary-hunk mismatch reasons and the
//! degenerate-empty-pattern Raw rejection (SPEC 25.7, SPEC 25.3).
//!
//! The two reasons below are pinned verbatim (text mode on stderr,
//! explicit nulls in the JSON envelope). The worktree stays untouched
//! in every case. The beyond-EOF arms moved to engine unit tests in
//! ordinary.rs: T367 count validation plus the Pass 0 range check make
//! them unreachable via the CLI parser. The `bad range` / `invalid
//! diff prefix` / `target file not found in tree` arms carry no pins:
//! they are unreachable via the CLI (SPEC 25.7 documents them as
//! defensive-only).

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
fn context_mismatch_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 +1,1 @@\n WRONG-CTX\n";
    let reason = "context mismatch at line 1: expected `WRONG-CTX`";
    let content = "fn main() {}\n";
    let dir = init_repo();
    write(dir.path(), "a.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("GIT_HUNK_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains("old_range: 1..2"), "got: {stderr}");
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("fix_context"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(v["error"]["code"], serde_json::json!("GIT_HUNK_CONFLICT"));
    assert_eq!(v["error"]["rule"], serde_json::json!(""));
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
    assert_eq!(v["error"]["range"], serde_json::json!("1..2"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_context"));
    assert_eq!(repair["line"], serde_json::json!(1));
    assert_eq!(repair["expected"], serde_json::json!("WRONG-CTX"));
    assert_eq!(repair["actual"], serde_json::json!("fn main() {}"));
}

#[test]
fn unsupported_shape_empty_pattern_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n-\n+\n";
    let reason = "unsupported pattern shape in v0.1 (only fn rename, struct/enum/trait/union/type/const/static/mod/impl item rename, type-alias/const-static evolution, `use`-path prefix rename, call rewrite, let-stmt/general-expr/type rewrite, item delete/insert, macro path rewrite)";
    let content = "fn main() {}\n";
    let dir = init_repo();
    write(dir.path(), "a.rs", content);
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
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
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

#[test]
fn removal_mismatch_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-WRONG\n+fn b2() {}\n";
    let reason = "removal mismatch at line 2: expected `WRONG`";
    let content = "fn a() {}\nfn b() {}\n";
    let dir = init_repo();
    write(dir.path(), "a.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("GIT_HUNK_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains("old_range: 1..3"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("fix_context"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(v["error"]["code"], serde_json::json!("GIT_HUNK_CONFLICT"));
    assert_eq!(v["error"]["range"], serde_json::json!("1..3"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_context"));
    assert_eq!(repair["line"], serde_json::json!(2));
    assert_eq!(repair["expected"], serde_json::json!("WRONG"));
    assert_eq!(repair["actual"], serde_json::json!("fn b() {}"));
}

#[test]
fn context_mismatch_self_corrects_first_try() {
    // WI-10-context-repair: the agent loop — fail, read `actual` from the
    // envelope, swap the hunk line, pass `--check` without re-reading.
    let patch = "diff --git a.rs\n@@ -1,1 +1,2 @@\n WRONG-CTX\n+// fixed\n";
    let content = "fn main() {}\n";
    let v = check_json(patch, "a.rs", content);
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_context"));
    let expected = repair["expected"]
        .as_str()
        .expect("repair must carry expected");
    let actual = repair["actual"].as_str().expect("repair must carry actual");
    let fixed = patch.replace(expected, actual);
    let dir = init_repo();
    write(dir.path(), "a.rs", content);
    let out = run_stdin(dir.path(), &fixed, &["--check"]);
    assert!(
        out.status.success(),
        "self-corrected context must pass check first try, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn removal_mismatch_self_corrects_first_try() {
    let patch = "diff --git a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-WRONG\n+fn b2() {}\n";
    let content = "fn a() {}\nfn b() {}\n";
    let v = check_json(patch, "a.rs", content);
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_context"));
    let expected = repair["expected"]
        .as_str()
        .expect("repair must carry expected");
    let actual = repair["actual"].as_str().expect("repair must carry actual");
    let fixed = patch.replace(expected, actual);
    let dir = init_repo();
    write(dir.path(), "a.rs", content);
    let out = run_stdin(dir.path(), &fixed, &["--check"]);
    assert!(
        out.status.success(),
        "self-corrected removal must pass check first try, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
