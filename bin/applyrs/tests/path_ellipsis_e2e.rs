//! T164: path ellipsis (SPEC 16.5) on call callees.
//!
//! Split into its own file to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

mod common;
use common::*;

#[test]
fn path_ellipsis_rewrites_zero_one_and_many_segments() {
    // T164 dogfood probe: `- crate::...::old(E)` rewrites every spelling
    // of the path (zero, one, and many middle segments); the last-segment
    // gate still protects `other`.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m() {\n    crate::old(1);\n    crate::a::old(2);\n    crate::a::b::old(3);\n    crate::a::other(4);\n}\n",
    );
    let patch = "diff --git a.rs\n@@ -1,6 @@ rule=r\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::new(E)\n";
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
        "fn m() {\n    crate::new(1);\n    crate::new(2);\n    crate::new(3);\n    crate::a::other(4);\n}\n"
    );
}

#[test]
fn path_ellipsis_plus_side_stays_concrete_only() {
    // A `+` path `...` has no absorbed path segments to replay (only arg
    // remainders replay, T157), so the plus/minus gate fails fast with
    // `RSMPL_PATTERN_CONFLICT` instead of emitting a guess.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn m() { crate::a::old(1); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::...::new(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    assert_eq!(read(dir.path(), "a.rs"), "fn m() { crate::a::old(1); }\n");
}

#[test]
fn path_ellipsis_json_check_reports_three_matches() {
    // JSON envelope path: the dogfood probe reports all three sites.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m() { crate::old(1); crate::a::old(2); crate::a::b::old(3); }\n",
    );
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::new(E)\n";
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
        serde_json::json!({"lowering": true, "semantics": true, "build": "skipped"})
    );
    assert_eq!(payload["rules"][0]["matches"], 3);
}

#[test]
fn path_ellipsis_plus_side_path_needle_is_path_specific() {
    // T166: a `+` path `...` trips the T157 count gate even though the
    // minus side HAS a `...` (in the path, not the args) — the diagnostic
    // must name the path case, not the arg-replay case.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn m() { crate::a::old(1); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::...::new(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    assert!(
        err.contains("plus side must be concrete"),
        "path needle expected, got: {err}"
    );
}

#[test]
fn arg_ellipsis_excess_keeps_arg_needle() {
    // T166 negative control: a pure argument-`...` excess (no path `...`
    // anywhere) keeps the existing T157 arg-flavored needle.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn m() { foo(1); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E, ...)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    assert!(
        err.contains("no matching `- ...` to replay"),
        "arg needle expected, got: {err}"
    );
}
