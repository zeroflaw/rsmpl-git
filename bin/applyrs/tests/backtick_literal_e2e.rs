//! T162: backtick literals (SPEC 15.4) on call/method-call shapes.
//!
//! Split into its own file to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

mod common;
use common::*;

#[test]
fn backtick_literal_arg_rewrites_only_literal_site() {
    // T162 P15b: `- foo(`E`)` with `# expr E` matches only the literal
    // `foo(E)` site; `foo(x)` is untouched. Before the fix the plus side
    // emitted raw backticks and the reparse backstop failed with
    // `RSMPL_PARSE_CONFLICT`.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m(E: i32) { let v = foo(E); let w = foo(x); }\n",
    );
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr E\n- foo(`E`)\n+ bar(`E`)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "text: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn m(E: i32) { let v = bar(E); let w = foo(x); }\n"
    );
}

#[test]
fn backtick_literal_no_site_fails_safe_with_match_conflict() {
    // T162 P15: with no literal `foo(E)` site the hunk must fail with
    // `RSMPL_MATCH_CONFLICT` (no match), not `RSMPL_PARSE_CONFLICT`.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn m() { let v = foo(x); }\n");
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr E\n- foo(`E`)\n+ bar(`E`)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(!err.contains("RSMPL_PARSE_CONFLICT"), "got: {err}");
    assert_eq!(read(dir.path(), "a.rs"), "fn m() { let v = foo(x); }\n");
}

#[test]
fn backtick_literal_method_arg_rewrites_only_literal_site() {
    // Method-call counterpart: `- r.old(`Arg`)` matches only the
    // literal-arg site.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m(r: R, Arg: i32) { let v = r.old(Arg); let w = r.old(x); }\n",
    );
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr Arg\n# expr R2\n- R2.old(`Arg`)\n+ R2.new(`Arg`)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "text: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn m(r: R, Arg: i32) { let v = r.new(Arg); let w = r.old(x); }\n"
    );
}

#[test]
fn backtick_literal_json_success_carries_no_backticks() {
    // JSON envelope path: the lowered diff must contain no backticks.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn m(E: i32) { let v = foo(E); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(`E`)\n+ bar(`E`)\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "json: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(
        payload["checked"],
        serde_json::json!({"lowering": true, "semantics": "verified", "build": "skipped"})
    );
    assert_eq!(payload["rules"][0]["matches"], 1);
}

#[test]
fn backtick_literal_macro_arg_rewrites_only_literal_site() {
    // Macro-invocation counterpart (same SPEC 15.4 promise): `- foo!(`E`)`
    // matches only the literal-arg site. Found in-batch while fixing the
    // call path — the macro minus path parsed raw hunk text, so the
    // literal classified as `Concrete("`E`")` and never matched.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m() { let v = foo!(E); let w = foo!(x); }\n",
    );
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr E\n- foo!(`E`)\n+ bar!(`E`)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "text: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn m() { let v = bar!(E); let w = foo!(x); }\n"
    );
}
