//! T276 (WI-4 slice 1): engine-verbatim `# symbol`-in-wrong-shape (SPEC 18.6 C14) reasons.
//!
//! SPEC 18.6 C14 enumerates the admitted positions (call shapes plus
//! general whole-path uses) and the named offending slot for the rest.
//! The macro hunk below pins the shape label; the sequence hunk pins the
//! remaining General label; the prefix hunk pins the slot-naming repair
//! shape. General whole-path uses (`let x = Old(E)`, bare `Old` types)
//! compile since WI-4 slice 1 — their pins live in `pattern_compilation`
//! and the `matcher_symbol` / `matcher_general_apply` unit tests.
//! Text output omits the null `range:`/`suggestion:` lines; the JSON
//! envelope carries explicit nulls.

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
fn symbol_in_sequence_names_shape_verbatim() {
    // The only General shape still rejected: sequences ride concrete
    // paths in slice 1.
    let patch = "diff --git a.rs\n@@ -1,2 @@ rule=sym_seq\n# scope=file\n# symbol Old=crate::foo\n# symbol New=crate::bar\n# fallback=syntax\n# expr E\n- let a = Old(E);\n- let b = E;\n+ let a = New(E);\n+ let b = E;\n";
    let (stderr, v) = check_both(
        patch,
        "a.rs",
        "fn f() {\n    let a = 1;\n    let b = 2;\n}\n",
    );
    assert!(
        stderr.contains(
            "this general statement-sequence hunk cannot use `# symbol` — use a call-shape or general whole-path hunk, or a concrete path pattern"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# symbol` is supported in call shapes (call-callee, call-argument, method-receiver) and general whole-path positions (general-expression, general `let`-statement, general-type) in v0.1 — symbols match by exact full-path text (SPEC 19.3); this general statement-sequence hunk cannot use `# symbol` — use a call-shape or general whole-path hunk, or a concrete path pattern"),
        "got: {v}"
    );
}

#[test]
fn symbol_prefix_slot_names_repair_verbatim() {
    // Whole-path positions only: a prefix use names its slot (C14).
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=sym_pre\n# scope=file\n# symbol Old=crate::foo\n# symbol New=crate::bar\n# fallback=syntax\n# expr E\n- let x = Old::compute(E);\n+ let x = New::compute(E);\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn f() {\n    let x = 1;\n}\n");
    assert!(
        stderr.contains(
            "`Old` names a path prefix (`Old::…`) — prefix symbols ride concrete paths in slice 1 (write the full path)"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`Old` names a path prefix (`Old::…`) — prefix symbols ride concrete paths in slice 1 (write the full path)"),
        "got: {v}"
    );
}

#[test]
fn symbol_in_macro_names_shape_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=sym_mac\n# scope=file\n# symbol S=crate::foo\n# fallback=syntax\n# macro M\n- foo!(M)\n+ bar!(M)\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn f() { foo!(x); }\n");
    assert!(
        stderr.contains(
            "this macro hunk cannot use `# symbol` — use a call-shape or general whole-path hunk, or a concrete path pattern"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("`# symbol` is supported in call shapes (call-callee, call-argument, method-receiver) and general whole-path positions (general-expression, general `let`-statement, general-type) in v0.1 — symbols match by exact full-path text (SPEC 19.3); this macro hunk cannot use `# symbol` — use a call-shape or general whole-path hunk, or a concrete path pattern"),
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
