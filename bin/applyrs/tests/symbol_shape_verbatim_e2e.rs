//! T276: engine-verbatim `# symbol`-in-wrong-shape (SPEC 18.6 C14) reasons.
//!
//! SPEC 18.6 C14 now carries the four concrete shape names the engine
//! emits (`general-expression`, `` general `let`-statement ``,
//! `general-type`, `macro`) instead of a `<shape>` placeholder, the
//! correction that the `use a call-shape hunk or a concrete path
//! pattern` tail is part of `reason:` (suggestion is null), and the
//! verbatim method-ident rejection with its `(symbols verify
//! receiver/callee paths, SPEC 19.3)` parenthetical. Text output omits
//! the null `range:`/`suggestion:` lines; the JSON envelope carries
//! explicit nulls.

mod common;
use common::*;

fn check_both(patch: &str, file: &str, content: &str) -> (String, serde_json::Value) {
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
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(
        !stderr.contains("suggestion:"),
        "null suggestion must be omitted from text, got: {stderr}"
    );
    assert!(
        !stderr.contains("old_range:"),
        "null range must be omitted from text, got: {stderr}"
    );
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
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
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::Value::Null,
        "got: {v}"
    );
    assert_eq!(v["error"]["range"], serde_json::Value::Null, "got: {v}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    (stderr, v)
}

#[test]
fn symbol_in_general_expr_names_shape_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=sym_expr\n# scope=file\n# symbol S=crate::foo\n# fallback=syntax\n# expr E\n- E + 1\n+ E\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn f() { let x = a + 1; }\n");
    assert!(
        stderr.contains(
            "this general-expression hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# symbol` is only supported in call shapes in v0.1 (call-callee, call-argument, and method-receiver symbols match by exact full-path text, SPEC 19.3); this general-expression hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern"),
        "got: {v}"
    );
}

#[test]
fn symbol_in_let_names_shape_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=sym_let\n# scope=file\n# symbol S=crate::foo\n# fallback=syntax\n# expr E\n- let x = E;\n+ let y = E;\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn f() {\n    let x = 1;\n}\n");
    assert!(
        stderr.contains(
            "this general `let`-statement hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# symbol` is only supported in call shapes in v0.1 (call-callee, call-argument, and method-receiver symbols match by exact full-path text, SPEC 19.3); this general `let`-statement hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern"),
        "got: {v}"
    );
}

#[test]
fn symbol_in_type_names_shape_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=sym_ty\n# scope=file\n# symbol S=crate::foo\n# fallback=syntax\n# type T\n- Vec<T>\n+ Box<[T]>\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn f(x: Vec<u8>) {}\n");
    assert!(
        stderr.contains(
            "this general-type hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# symbol` is only supported in call shapes in v0.1 (call-callee, call-argument, and method-receiver symbols match by exact full-path text, SPEC 19.3); this general-type hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern"),
        "got: {v}"
    );
}

#[test]
fn symbol_in_macro_names_shape_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=sym_mac\n# scope=file\n# symbol S=crate::foo\n# fallback=syntax\n# macro M\n- foo!(M)\n+ bar!(M)\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn f() { foo!(x); }\n");
    assert!(
        stderr.contains(
            "this macro hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# symbol` is only supported in call shapes in v0.1 (call-callee, call-argument, and method-receiver symbols match by exact full-path text, SPEC 19.3); this macro hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern"),
        "got: {v}"
    );
}

#[test]
fn symbol_naming_method_ident_is_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=sym_mid\n# scope=file\n# symbol old=crate::foo\n# fallback=syntax\n# expr R\n- R.old()\n+ R.new()\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn f() { o.old(); }\n");
    assert!(
        stderr.contains(
            "`# symbol old` names the method ident; method idents already match exactly — remove that `# symbol` line (symbols verify receiver/callee paths, SPEC 19.3)"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# symbol old` names the method ident; method idents already match exactly — remove that `# symbol` line (symbols verify receiver/callee paths, SPEC 19.3)"),
        "got: {v}"
    );
}
