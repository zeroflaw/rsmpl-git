//! WI-1d: `rule=`-only hunk detection end-to-end (SPEC 7.1).
//!
//! No `#` lines at all: `rule=` alone makes the hunk RSmPL with an empty
//! directive list. Directive-free terse renames, inferred calls, and item
//! renames real-apply; directive-only (no rule) still works; no-rule
//! no-directive stays ordinary.

mod common;
use common::*;

#[test]
fn rule_only_inferred_call_applies() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn f() { foo(1); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "fn f() { bar(1); }\n");
}

#[test]
fn rule_only_terse_fn_rename_applies() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn foo() {}\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=rename_foo\n- fn foo\n+ fn bar\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "fn bar() {}\n");
}

#[test]
fn rule_only_item_rename_applies() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "struct Old;\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "struct New;\n");
}

#[test]
fn directive_only_without_rule_still_rsmpl() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn f() { foo(1); foo(2); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@\n# allow=many\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "fn f() { bar(1); bar(2); }\n");
}

#[test]
fn no_rule_no_directive_stays_ordinary() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn foo() {}\nfn bar() {}\n");
    let patch = "diff --git a.rs\n@@ -1,2 +1,2 @@\n-fn foo() {}\n-fn bar() {}\n+fn foo() {}\n+fn bar2() {}\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "fn foo() {}\nfn bar2() {}\n");
}

#[test]
fn rule_with_text_body_fails_as_pattern_not_text() {
    // `rule=` claims semantic intent: the text edit is compiled as a
    // pattern and must not silently apply as ordinary text.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn foo() {}\nfn bar() {}\n");
    let patch = "diff --git a.rs\n@@ -1,2 +1,2 @@ rule=r\n-fn foo() {}\n-fn bar() {}\n+fn foo() {}\n+fn bar2() {}\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        !out.status.success(),
        "rule+text must not --check clean, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn rule_only_text_failure_hints_remove_rule() {
    // SPEC 7.1: a `rule=`-only text hunk is reclassified as semantic, so
    // the parse failure must name the marker, not just the symptom.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn foo() {}\nfn bar() {}\n");
    let patch = "diff --git a.rs\n@@ -1,2 +1,2 @@ rule=r\n-fn foo() {}\n-fn bar() {}\n+fn foo() {}\n+fn bar2() {}\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        err.contains("remove `rule=` for plain text hunks"),
        "hint missing, got: {err}"
    );
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json envelope");
    let sug = v["error"]["suggestion"].as_str().unwrap_or("");
    assert!(
        sug.contains("remove `rule=` for plain text hunks"),
        "json hint missing, got: {v}"
    );
}

#[test]
fn directive_failure_carries_no_rule_hint() {
    // The hint fires only for directive-free `rule=` hunks; a hunk that
    // already carries `#` directives fails without it.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn foo() {}\n");
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n- fn foo(...) {\n+ fn foo(...) {\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json envelope");
    let sug = v["error"]["suggestion"].as_str().unwrap_or("").to_string();
    assert!(
        !sug.contains("remove `rule=` for plain text hunks"),
        "hint must not fire with directives, got: {v}"
    );
}
