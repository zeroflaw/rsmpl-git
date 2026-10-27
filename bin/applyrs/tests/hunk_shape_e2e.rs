//! T365 + T367: hunk-header shape fail-fast pins (SPEC 25.2).
//!
//! A hunk header with leading whitespace is rejected (T365: without the
//! gate it lands in metadata and lowers to an empty no-op with exit 0),
//! and ordinary hunk headers must agree with the body counts (T367:
//! without the gate a miscount surfaces downstream as a misleading
//! `GIT_HUNK_CONFLICT`). Both fail fast with `RSMPL_PARSE_CONFLICT`
//! (exit 2); text + JSON envelopes pinned, worktree untouched.

mod common;
use common::*;

fn check_rejected(patch: &str, file: &str, content: &str) -> (String, serde_json::Value) {
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
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .expect("failure stdout must be a JSON envelope");
    assert_eq!(
        read(dir.path(), file),
        content,
        "worktree must be untouched"
    );
    (stderr, v)
}

#[test]
fn indented_hunk_header_rejected() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n @@ -1,1 +1,1 @@\n-fn a() {}\n+fn b() {}\n";
    let (stderr, v) = check_rejected(patch, "src/lib.rs", "fn a() {}\n");
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("leading whitespace"), "got: {stderr}");
    assert!(stderr.contains("column 0"), "got: {stderr}");
    assert!(stderr.contains("old_range: line 4"), "got: {stderr}");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert_eq!(v["error"]["range"], serde_json::json!("line 4"));
    assert!(v["error"]["suggestion"].is_string(), "got: {v}");
}

#[test]
fn ordinary_hunk_count_mismatch_rejected() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,7 +1,7 @@\n fn a() {}\n-fn b() {}\n+fn b2() {}\n";
    let (stderr, v) = check_rejected(patch, "src/lib.rs", "fn a() {}\nfn b() {}\n");
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains(
            "ordinary hunk body has 2 old-side and 2 new-side lines but header claims -1,7 +1,7"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert!(v["error"]["range"].is_string(), "got: {v}");
    assert!(
        v["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("fix the header counts"),
        "got: {v}"
    );
}

#[test]
fn well_formed_ordinary_hunk_applies() {
    // Control: truthful headers still apply for real (T361 doctrine).
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "mod old;\n\nfn main() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n mod old;\n+mod new;\n\n fn main() {}\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "mod old;\nmod new;\n\nfn main() {}\n"
    );
}
