//! T305: engine-verbatim item-rename attribute-merge reasons (SPEC 14.8).
//!
//! SPEC 14.6 carries the fn/general ambiguous-merge fences verbatim, but
//! the item matcher emits its own wording for the plus-writes-attrs case
//! (`plus side writes ...`, suggestion null). The differ case reuses the
//! 14.6 second fence byte-identical (suggestion set). Both are
//! `RSMPL_PATTERN_CONFLICT`; the worktree stays untouched.

mod common;
use common::*;

const OMITTED: &str = "plus side writes attributes the minus side omitted (ambiguous merge); write the attributes on both sides or neither";
const DIFFER: &str =
    "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges";
const DIFFER_SUGGESTION: &str = "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`";

fn omitted_patch() -> &'static str {
    "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- struct Old\n+ #[derive(Clone)]\n+ struct New\n"
}

fn differ_patch() -> &'static str {
    "diff --git a.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- #[derive(Clone)]\n- struct Old\n+ #[derive(Debug)]\n+ struct New\n"
}

fn check_text(patch: &str, content: &str, reason: &str, want_suggestion: bool) {
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
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert_eq!(
        stderr.contains("suggestion:"),
        want_suggestion,
        "got: {stderr}"
    );
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content, "worktree must be untouched");
}

fn check_json(
    patch: &str,
    content: &str,
    reason: &str,
    suggestion: Option<&str>,
) -> serde_json::Value {
    let dir = init_repo();
    write(dir.path(), "a.rs", content);
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
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
    assert_eq!(v["error"]["rule"], serde_json::json!("r"), "got: {v}");
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    match suggestion {
        Some(s) => assert_eq!(v["error"]["suggestion"], serde_json::json!(s), "got: {v}"),
        None => assert!(v["error"]["suggestion"].is_null(), "got: {v}"),
    }
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content, "worktree must be untouched");
    v
}

#[test]
fn plus_omitted_rejected_verbatim_text() {
    check_text(omitted_patch(), "struct Old;\n", OMITTED, false);
}

#[test]
fn plus_omitted_rejected_verbatim_json() {
    check_json(omitted_patch(), "struct Old;\n", OMITTED, None);
}

#[test]
fn differ_rejected_verbatim_text() {
    check_text(
        differ_patch(),
        "#[derive(Clone)]\nstruct Old;\n",
        DIFFER,
        true,
    );
    let dir = init_repo();
    write(dir.path(), "a.rs", "#[derive(Clone)]\nstruct Old;\n");
    let out = run_stdin(dir.path(), differ_patch(), &["--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains(DIFFER_SUGGESTION), "got: {stderr}");
}

#[test]
fn differ_rejected_verbatim_json() {
    check_json(
        differ_patch(),
        "#[derive(Clone)]\nstruct Old;\n",
        DIFFER,
        Some(DIFFER_SUGGESTION),
    );
}

#[test]
fn mirrored_attrs_apply_control() {
    let patch = "diff --git a.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- #[derive(Clone)]\n- struct Old\n+ #[derive(Clone)]\n+ struct New\n";
    let dir = init_repo();
    write(dir.path(), "a.rs", "#[derive(Clone)]\nstruct Old;\n");
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, "#[derive(Clone)]\nstruct New;\n", "got: {got}");
}

#[test]
fn attrs_minus_bare_target_skips_control() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- #[derive(Clone)]\n- struct Old\n+ #[derive(Clone)]\n+ struct New\n";
    let dir = init_repo();
    write(dir.path(), "a.rs", "struct Old;\n");
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("no match found"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, "struct Old;\n", "worktree must be untouched");
}
