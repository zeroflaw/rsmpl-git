//! T313: engine-verbatim unknown-directive variants (SPEC 9.6 / 25.3).
//!
//! SPEC 9.6/25.3 pinned only the canonical `k=v` fence
//! (`` unknown directive `#foo=bar`: # foo=bar ``, suggestion
//! `check SPEC 9.3 ...`). Two more reachable variants fail the same way
//! (`RSMPL_PATTERN_CONFLICT`, exit 2, parse-time bare envelope — text
//! carries the bare `reason:` line only, JSON carries explicit null
//! `rule`/`file`/`range`): the typo near-miss with its set `use ...`
//! suggestion, and the bare/multi-token first-token echo with the generic
//! 9.3 pointer. The worktree stays untouched in every failure case.

mod common;
use common::*;

const BASE: &str = "fn foo() {}\n";

fn fail_text_json(patch: &str, reason: &str, suggestion: Option<&str>) {
    let dir = init_repo();
    write(dir.path(), "a.rs", BASE);
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
    for absent in ["rule:", "file:", "old_range:"] {
        assert!(
            !stderr.contains(absent),
            "null `{absent}` must be omitted from text, got: {stderr}"
        );
    }
    match suggestion {
        Some(s) => assert!(stderr.contains(s), "got: {stderr}"),
        None => assert!(stderr.contains("check SPEC 9.3"), "got: {stderr}"),
    }
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, BASE, "worktree must be untouched");
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
    for field in ["rule", "file", "range"] {
        assert!(v["error"][field].is_null(), "got: {v}");
    }
    match suggestion {
        Some(s) => assert_eq!(v["error"]["suggestion"], serde_json::json!(s), "got: {v}"),
        None => assert_eq!(
            v["error"]["suggestion"],
            serde_json::json!("check SPEC 9.3 for the supported directive list"),
            "got: {v}"
        ),
    }
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, BASE, "worktree must be untouched");
}

#[test]
fn typo_nearmiss_suggests_canonical() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# alow=many\n- foo(E)\n+ bar(E)\n";
    fail_text_json(
        patch,
        "unknown directive `#alow=many` (did you mean `#allow=...`?): # alow=many",
        Some("use `#allow=...`"),
    );
}

#[test]
fn bare_unknown_echoes_first_token() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# foo\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    fail_text_json(patch, "unknown directive `#foo`: # foo", None);
}

#[test]
fn multitoken_unknown_echoes_first_token() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# foo bar\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    fail_text_json(patch, "unknown directive `#foo`: # foo bar", None);
}

#[test]
fn spec_96_names_all_three_variants() {
    let spec = std::fs::read_to_string(workspace_root().join("SPEC.md")).unwrap();
    for needle in [
        "unknown directive `#foo=bar`: # foo=bar",
        "unknown directive `#alow=many` (did you mean `#allow=...`?): # alow=many",
        "unknown directive `#foo`: # foo",
        "invalid directive: {line}",
    ] {
        assert!(spec.contains(needle), "SPEC 9.6 missing {needle:?}");
    }
}
