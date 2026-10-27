//! T269: minus-only call-shape hunks delete the matched call expression.
//!
//! They never reach the C6 empty-`+` gate (general-expr only, SPEC 18.6):
//! a statement site keeps its `;` (valid Rust, applies cleanly), while a
//! nested deletion that would produce invalid syntax fails at the 24.2
//! rewritten-file reparse gate with `RSMPL_PARSE_CONFLICT`.

mod common;
use common::*;

const STMTS: &str = "fn foo() -> i32 { 1 }\nfn bar() {\n    foo();\n}\n";

#[test]
fn minus_only_call_deletes_statement_site_leaving_semicolon() {
    let dir = init_repo();
    write(dir.path(), "a.rs", STMTS);
    let patch = "diff --git a.rs\n@@ -3,1 @@ rule=del\n# lang=rust\n- foo(...)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("-    foo();"), "got: {stdout}");
    assert!(stdout.contains("+    ;"), "got: {stdout}");
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(1), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[3, 3]]),
        "got: {v}"
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "a.rs");
    assert!(got.contains("    ;\n"), "got: {got}");
    assert!(!got.contains("foo();"), "got: {got}");
}

#[test]
fn minus_only_call_nested_site_fails_reparse() {
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn foo() -> i32 { 1 }\nfn bar() {\n    foo();\n    let x = foo();\n}\n",
    );
    let before = read(dir.path(), "a.rs");
    let patch = "diff --git a.rs\n@@ -4,1 @@ rule=del\n# lang=rust\n- foo(...)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "nested deletion must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("rewritten file does not parse as Rust: expected an expression"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "a.rs"), before, "untouched");
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(!out.status.success(), "json must fail");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT"),
        "got: {v}"
    );
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("expected an expression"),
        "got: {v}"
    );
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}
