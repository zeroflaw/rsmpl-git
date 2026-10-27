//! T327: scope-mode behavior pins — `contained` / `anchor` / `overlap` /
//! `exact` diverge as SPEC 17.3 documents (only `file` was pinned before).
//!
//! Fixture A: `fn foo` spanning lines 1-4 with a mid-body `@@ -2,2 @@`
//! range — `contained`/`anchor`/`exact` fail (standard 25.4 zero-match
//! fence), `overlap` applies. Fixture B: multiline call spanning lines
//! 2-4 — first-line `@@ -2,1 @@` matches under `anchor` (and
//! `overlap`) but not `contained`/`exact`; middle-line `@@ -3,1 @@`
//! matches only under `overlap`; full-span `@@ -2,3 @@` matches under
//! `exact`. Failures leave the worktree untouched.

mod common;
use common::*;

const FN_SRC: &str = "fn foo() {\n    let x = old(1);\n    old(2);\n}\n";
const FN_BAR: &str = "fn bar() {\n    let x = old(1);\n    old(2);\n}\n";

const CALL_SRC: &str = "fn foo() {\n    old(\n        1,\n    );\n}\n";
const CALL_NEW: &str = "fn foo() {\n    new(\n        1,\n    );\n}\n";

const SINGLE_SRC: &str = "fn foo() {\n    let x = old(1);\n    old(2);\n}\n";
const SINGLE_NEW: &str = "fn foo() {\n    let x = old(1);\n    new(2);\n}\n";

fn fn_patch(scope: &str) -> String {
    format!(
        "diff --git src/lib.rs\n@@ -2,2 @@ rule=r1\n# scope={scope}\n# lang=rust\n\
         - fn foo(...) {{\n+ fn bar(...) {{\n      ...\n  }}\n"
    )
}

fn call_patch(range: &str, scope: &str) -> String {
    format!(
        "diff --git src/lib.rs\n@@ -{range} @@ rule=r1\n# scope={scope}\n# expr E\n\
         - old(E)\n+ new(E)\n"
    )
}

fn assert_no_match(out: &std::process::Output, src: &str) {
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: r1"), "got: {stderr}");
    assert!(stderr.contains("file: src/lib.rs"), "got: {stderr}");
    assert!(stderr.contains("no match found"), "got: {stderr}");
    let _ = src;
}

#[test]
fn fn_contained_mid_range_fails() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FN_SRC);
    let out = run_stdin(dir.path(), &fn_patch("contained"), &[]);
    assert_no_match(&out, FN_SRC);
    assert_eq!(read(dir.path(), "src/lib.rs"), FN_SRC);
}

#[test]
fn fn_anchor_mid_range_fails() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FN_SRC);
    let out = run_stdin(dir.path(), &fn_patch("anchor"), &[]);
    assert_no_match(&out, FN_SRC);
    assert_eq!(read(dir.path(), "src/lib.rs"), FN_SRC);
}

#[test]
fn fn_overlap_mid_range_applies() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FN_SRC);
    let out = run_stdin(dir.path(), &fn_patch("overlap"), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), FN_BAR);
}

#[test]
fn fn_exact_mid_range_fails() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FN_SRC);
    let out = run_stdin(dir.path(), &fn_patch("exact"), &[]);
    assert_no_match(&out, FN_SRC);
    assert_eq!(read(dir.path(), "src/lib.rs"), FN_SRC);
}

#[test]
fn call_first_line_anchor_applies_but_contained_fails() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", CALL_SRC);
    let out = run_stdin(dir.path(), &call_patch("2,1", "contained"), &["--check"]);
    assert_no_match(&out, CALL_SRC);
    assert_eq!(read(dir.path(), "src/lib.rs"), CALL_SRC);

    let dir = init_repo();
    write(dir.path(), "src/lib.rs", CALL_SRC);
    let out = run_stdin(dir.path(), &call_patch("2,1", "anchor"), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), CALL_NEW);
}

#[test]
fn call_middle_line_matches_only_overlap() {
    for scope in ["contained", "anchor", "exact"] {
        let dir = init_repo();
        write(dir.path(), "src/lib.rs", CALL_SRC);
        let out = run_stdin(dir.path(), &call_patch("3,1", scope), &["--check"]);
        assert_no_match(&out, CALL_SRC);
        assert_eq!(read(dir.path(), "src/lib.rs"), CALL_SRC);
    }

    let dir = init_repo();
    write(dir.path(), "src/lib.rs", CALL_SRC);
    let out = run_stdin(dir.path(), &call_patch("3,1", "overlap"), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), CALL_NEW);
}

#[test]
fn call_exact_full_span_applies() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", CALL_SRC);
    let out = run_stdin(dir.path(), &call_patch("2,3", "exact"), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), CALL_NEW);
}

#[test]
fn single_line_call_exact_matches_line_range() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", SINGLE_SRC);
    let out = run_stdin(dir.path(), &call_patch("3,1", "exact"), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), SINGLE_NEW);
}

#[test]
fn spec_17_3_carries_scope_matrix() {
    let spec = std::fs::read_to_string(workspace_root().join("SPEC.md")).unwrap();
    for needle in [
        "node not fully inside",
        "anchor inside",
        "any overlap suffices",
        "full span",
    ] {
        assert!(spec.contains(needle), "17.3 matrix drifted: {needle}");
    }
}
