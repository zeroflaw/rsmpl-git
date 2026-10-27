//! T293: engine-verbatim structural fail-fast reasons (SPEC 7.3, 8, 12.4,
//! 25.2, 25.3).
//!
//! All seven reasons below are pinned verbatim (text mode on stderr,
//! explicit nulls in the JSON envelope). The worktree stays untouched
//! in every case. The bare `invalid hunk header: {line}` arm carries no
//! pin: it is unreachable via the CLI (the header parser only runs on
//! `@@`-led lines — SPEC 8 / 25.2 document it as defensive-only).
//! The rename round-trip is pinned by
//! `cli_e2e::differing_diff_git_paths_apply_as_rename` (T377: renames
//! apply since v0.1 rename support; the T29 fail-fast is retired).

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
fn empty_body_no_pattern_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n";
    let reason = "RSmPL hunk has no pattern body lines";
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
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn misplaced_directive_after_body_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n# lang=rust\n+ fn bar(...) {\n      ...\n  }\n";
    let reason = "directive line must appear immediately after hunk header, before body lines";
    let suggestion = "move the `#` line above the first `-`/`+`/` ` line";
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
    assert!(stderr.contains(suggestion), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
}

#[test]
fn ordinary_hunk_elided_new_side_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@\n fn main() {}\n";
    let reason = "ordinary hunk requires both -old,len +new,len ranges: @@ -1,1 @@";
    let suggestion =
        "write both ranges (e.g. `@@ -1,2 +1,2 @@`) or add a `#` directive line to make it an RSmPL hunk";
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
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains(suggestion), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
}

#[test]
fn invalid_hunk_header_missing_close_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let reason = "invalid hunk header (missing closing @@): @@ -1,1 rule=r";
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
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("old_range: line 2"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["range"], serde_json::json!("line 2"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn invalid_hunk_header_ranges_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 +1,1 +1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let reason = "invalid hunk header ranges: @@ -1,1 +1,1 +1,1 @@ rule=r";
    let content = "fn main() {}\n";
    let _ = reason;
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
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("old_range: line 2"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["range"], serde_json::json!("line 2"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn invalid_hunk_range_verbatim() {
    let patch = "diff --git a.rs\n@@ -foo @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let reason = "invalid hunk range: -foo";
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
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("old_range: line 2"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["range"], serde_json::json!("line 2"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn hunk_header_outside_file_section_verbatim() {
    let patch = "@@ -1,1 +1,1 @@\n foo\n";
    let reason = "hunk header outside file section";
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
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content);
    let v = check_json(patch, "a.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}
