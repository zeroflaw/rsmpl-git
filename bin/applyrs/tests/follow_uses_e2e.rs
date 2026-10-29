//! WI-6a: `# follow=uses` same-file rename completion (SPEC 14.8).
//!
//! One hunk rewrites the definition plus its same-file use sites (type
//! paths, struct literals, values, calls, `use` trees) while method idents
//! and field accesses stay untouched. Shadowing and macro-opacity veto
//! loud without `# fallback=syntax`, skip + report with it; any other
//! shape carrying `# follow=uses` fails at the SPEC 18.6 gate.

mod common;
use common::*;

const STRUCT_BEFORE: &str = "use m::Old;\nstruct Old {\n    field: i32,\n}\nfn f(x: Old) -> Old {\n    let y: Old = Old { field: 1 };\n    let z = Old;\n    z\n}\n";

const STRUCT_PATCH: &str =
    "diff --git src/lib.rs\n@@ -2,3 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";

const STRUCT_WANT: &str = "use m::New;\nstruct New {\n    field: i32,\n}\nfn f(x: New) -> New {\n    let y: New = New { field: 1 };\n    let z = New;\n    z\n}\n";

#[test]
fn struct_follow_rewrites_definition_and_all_use_sites() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", STRUCT_BEFORE);
    let out = run_stdin(dir.path(), STRUCT_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), STRUCT_WANT);
}

#[test]
fn struct_rename_without_follow_stays_definition_only() {
    // No behavior change when the directive is absent.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", STRUCT_BEFORE);
    let patch = "diff --git src/lib.rs\n@@ -2,3 @@ rule=rename\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("struct New"), "got:\n{got}");
    assert!(got.contains("x: Old"), "uses untouched, got:\n{got}");
}

const FN_BEFORE: &str = "fn old(x: i32) -> i32 {\n    x + 1\n}\nstruct S;\nfn g(s: S) -> i32 {\n    let a = old(1);\n    let b = m::old(2);\n    let c = s.old();\n    a + b + c\n}\n";

const FN_PATCH: &str = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_fn\n# follow=uses\n- fn old(...) {\n+ fn new(...) {\n      ...\n  }\n";

#[test]
fn fn_follow_rewrites_calls_but_not_methods() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FN_BEFORE);
    let out = run_stdin(dir.path(), FN_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("fn new(x: i32)"), "def renamed, got:\n{got}");
    assert!(got.contains("let a = new(1);"), "got:\n{got}");
    assert!(got.contains("m::new(2)"), "got:\n{got}");
    assert!(
        got.contains("s.old()"),
        "method ident untouched, got:\n{got}"
    );
    assert!(!got.contains("old(1)"), "got:\n{got}");
}

const SHADOW_BEFORE: &str = "mod a {\n    pub struct Old;\n}\nmod b {\n    pub struct Old;\n}\n";

const SHADOW_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";

#[test]
fn shadowed_definition_vetoes_without_fallback() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", SHADOW_BEFORE);
    let out = run_stdin(dir.path(), SHADOW_PATCH, &["--check"]);
    assert!(!out.status.success(), "veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("shadowed"), "got: {stderr}");
}

#[test]
fn shadowed_definition_skips_with_fallback_and_reports() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", SHADOW_BEFORE);
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
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
    let bindings = &v["rules"][0]["bindings"];
    let joined = bindings.to_string();
    assert!(
        joined.contains("follow_skipped"),
        "fallback skip reported, got: {v}"
    );
}

#[test]
fn follow_on_call_shape_is_rejected() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {\n    foo(1);\n}\n");
    let patch =
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# follow=uses\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "gate rejection expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("# follow=uses"), "got: {stderr}");
}

#[test]
fn macro_mention_vetoes_without_fallback() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct Old;\nfn f() {\n    m!(Old);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "opacity veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("opaque"), "got: {stderr}");
}
