//! T271: engine-verbatim fail-fast reasons for the `# repeat` shape gate,
//! the `# format` allowlist, and `# returns=none` on non-functions.
//!
//! SPEC 17.8/22/14.2 now carry these diagnostics verbatim (rule + file
//! lines in text mode, explicit nulls in the JSON envelope). `` `v1` ``
//! in the repeat reasons names the general-matcher generation (the
//! SPEC 18.6 C-table namespace), not the SPEC v0.1 version.

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
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    v
}

#[test]
fn repeat_on_let_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# repeat\n# allow=many\n# expr E\n- let x = E;\n+ let y = E;\n";
    let v = check_json(patch, "src/lib.rs", "fn f() {\n    let x = foo(1);\n}\n");
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# repeat` is not supported on `let`-statement patterns in v1 (folding an expression rule inside a bound pattern or type would be surprising); emit one ordered hunk per nesting level"),
        "got: {v}"
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::Value::Null,
        "got: {v}"
    );
}

#[test]
fn repeat_on_type_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# repeat\n# type T\n- Vec<T>\n+ Box<[T]>\n";
    let v = check_json(patch, "src/lib.rs", "fn f(x: Vec<u8>) {}\n");
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# repeat` is not supported on type patterns in v1; emit one ordered hunk per nesting level"),
        "got: {v}"
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::Value::Null,
        "got: {v}"
    );
}

#[test]
fn repeat_on_function_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# repeat\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let v = check_json(patch, "src/lib.rs", "fn foo() {}\n");
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# repeat` requires a call/method-call or general-expression rewrite pattern (other shapes are future work)"),
        "got: {v}"
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::Value::Null,
        "got: {v}"
    );
}

#[test]
fn repeat_with_bind_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=cut\n# repeat\n# item I\n# bind=H\n- I\n";
    let v = check_json(patch, "src/lib.rs", "fn a() {}\n");
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!(
            "`# repeat` cannot be combined with `# bind=`/`# use=` (captures move verbatim)"
        ),
        "got: {v}"
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!("remove `# repeat` from the cut/paste hunk"),
        "got: {v}"
    );
}

#[test]
fn returns_none_on_item_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# returns=none\n# lang=rust\n- struct Old\n+ struct New\n";
    let v = check_json(patch, "src/lib.rs", "struct Old;\n");
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# returns=none` requires a function-item pattern"),
        "got: {v}"
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::Value::Null,
        "got: {v}"
    );
}

#[test]
fn format_rustfmt_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# format=rustfmt\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("only `# format=preserve` is supported in v0.1"),
        "got: {stderr}"
    );
    assert!(
        stderr.contains("remove the directive and run `cargo fmt` after apply"),
        "got: {stderr}"
    );
    let v = check_json(patch, "src/lib.rs", "fn foo() {}\n");
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("only `# format=preserve` is supported in v0.1"),
        "got: {v}"
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!("remove the directive and run `cargo fmt` after apply"),
        "got: {v}"
    );
}
