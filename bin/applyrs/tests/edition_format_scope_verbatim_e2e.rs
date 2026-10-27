//! T314: engine-verbatim edition/format_scope rejections (SPEC 9.6 / 25.3).
//!
//! SPEC 9.6 named `# edition=*` and `# format_scope=*` as rejected but
//! carried no verbatim fences. Both fail parse-time (exit 2, bare +
//! suggestion envelope — text carries the `reason:` + `suggestion:`
//! lines only with null rule/file/range omitted; JSON carries explicit
//! null `rule`/`file`/`range` plus the string suggestion). The worktree
//! stays untouched in every failure case.

mod common;
use common::*;

const BASE: &str = "fn foo() {}\n";

fn fail_text_json(patch: &str, reason: &str, suggestion: &str) {
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
    assert!(stderr.contains(suggestion), "got: {stderr}");
    for absent in ["rule:", "file:", "old_range:"] {
        assert!(
            !stderr.contains(absent),
            "null `{absent}` must be omitted from text, got: {stderr}"
        );
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
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!(suggestion),
        "got: {v}"
    );
    for field in ["rule", "file", "range"] {
        assert!(v["error"][field].is_null(), "got: {v}");
    }
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, BASE, "worktree must be untouched");
}

#[test]
fn edition_rejected_with_toolchain_default_suggestion() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# edition=2021\n- foo(E)\n+ bar(E)\n";
    fail_text_json(
        patch,
        "unsupported directive `# edition=2021` (edition is the toolchain default in v0.1): # edition=2021",
        "remove the directive \u{2014} edition is the toolchain default in v0.1",
    );
}

#[test]
fn format_scope_rejected_with_preserve_only_suggestion() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# format_scope=file\n- foo(E)\n+ bar(E)\n";
    fail_text_json(
        patch,
        "unsupported directive `# format_scope=file` (only `# format=preserve` is supported in v0.1): # format_scope=file",
        "remove the directive \u{2014} only `# format=preserve` is supported",
    );
}

#[test]
fn edition_value_echoes_offending_spelling() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# edition=2018\n- foo(E)\n+ bar(E)\n";
    fail_text_json(
        patch,
        "unsupported directive `# edition=2018` (edition is the toolchain default in v0.1): # edition=2018",
        "remove the directive \u{2014} edition is the toolchain default in v0.1",
    );
}

#[test]
fn format_scope_value_echoes_offending_spelling() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# format_scope=line\n- foo(E)\n+ bar(E)\n";
    fail_text_json(
        patch,
        "unsupported directive `# format_scope=line` (only `# format=preserve` is supported in v0.1): # format_scope=line",
        "remove the directive \u{2014} only `# format=preserve` is supported",
    );
}

#[test]
fn spec_96_names_both_unsupported_directive_fences() {
    let spec = std::fs::read_to_string(workspace_root().join("SPEC.md")).unwrap();
    for needle in [
        "unsupported directive `# edition=2021` (edition is the toolchain default in v0.1): # edition=2021",
        "remove the directive \u{2014} edition is the toolchain default in v0.1",
        "unsupported directive `# format_scope=file` (only `# format=preserve` is supported in v0.1): # format_scope=file",
        "remove the directive \u{2014} only `# format=preserve` is supported",
    ] {
        assert!(spec.contains(needle), "SPEC 9.6 missing {needle:?}");
    }
}
