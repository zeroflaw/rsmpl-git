//! T326: multi-file atomicity — a failure in any file section aborts
//! the whole patch (SPEC 23.1).
//!
//! Matching, planning, and lowering are all-or-nothing across file
//! sections: a match failure in one section discards successful matches
//! in the others. No file is written and nothing is applied (exit 2,
//! worktree untouched in both `--check` and apply modes); the error
//! envelope names the failing hunk (`rule` + `file`).

mod common;
use common::*;

const A_GOOD: &str = "diff --git a/a.rs b/a.rs\n@@ -1,1 @@ rule=good_a\n# lang=rust\n- fn a(...) {\n+ fn aa(...) {\n      ...\n  }\n";
const B_GOOD: &str = "diff --git a/b.rs b/b.rs\n@@ -1,1 @@ rule=good_b\n# lang=rust\n- fn b(...) {\n+ fn bb(...) {\n      ...\n  }\n";
const B_BAD: &str =
    "diff --git a/b.rs b/b.rs\n@@ -1,1 @@ rule=bad_b\n# lang=rust\n# expr E\n- nope(E)\n+ yup(E)\n";
const A_ORDINARY: &str = "diff --git a/a.rs b/a.rs\n@@ -2,1 +2,1 @@\n-line2\n+line2x\n";

const A_SRC: &str = "fn a() {}\n";
const B_SRC: &str = "fn b() {}\n";
const A_ORD_SRC: &str = "fn a() {}\nline2\n";

#[test]
fn fail_second_section_aborts_whole_patch() {
    let dir = init_repo();
    write(dir.path(), "a.rs", A_SRC);
    write(dir.path(), "b.rs", B_SRC);
    let patch = format!("{A_GOOD}{B_BAD}");
    let out = run_stdin(dir.path(), &patch, &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: bad_b"), "got: {stderr}");
    assert!(stderr.contains("file: b.rs"), "got: {stderr}");
    assert_eq!(read(dir.path(), "a.rs"), A_SRC);
    assert_eq!(read(dir.path(), "b.rs"), B_SRC);
}

#[test]
fn fail_first_section_aborts_whole_patch() {
    let dir = init_repo();
    write(dir.path(), "a.rs", A_SRC);
    write(dir.path(), "b.rs", B_SRC);
    let patch = format!("{B_BAD}{A_GOOD}");
    let out = run_stdin(dir.path(), &patch, &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: bad_b"), "got: {stderr}");
    assert_eq!(read(dir.path(), "a.rs"), A_SRC);
    assert_eq!(read(dir.path(), "b.rs"), B_SRC);
}

#[test]
fn ordinary_good_plus_semantic_bad_aborts_apply() {
    let dir = init_repo();
    write(dir.path(), "a.rs", A_ORD_SRC);
    write(dir.path(), "b.rs", B_SRC);
    let patch = format!("{A_ORDINARY}{B_BAD}");
    let out = run_stdin(dir.path(), &patch, &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert_eq!(read(dir.path(), "a.rs"), A_ORD_SRC);
    assert_eq!(read(dir.path(), "b.rs"), B_SRC);
}

#[test]
fn ordinary_good_plus_semantic_bad_aborts_check() {
    let dir = init_repo();
    write(dir.path(), "a.rs", A_ORD_SRC);
    write(dir.path(), "b.rs", B_SRC);
    let patch = format!("{A_ORDINARY}{B_BAD}");
    let out = run_stdin(dir.path(), &patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert_eq!(read(dir.path(), "a.rs"), A_ORD_SRC);
    assert_eq!(read(dir.path(), "b.rs"), B_SRC);
}

#[test]
fn two_good_sections_apply_together() {
    let dir = init_repo();
    write(dir.path(), "a.rs", A_SRC);
    write(dir.path(), "b.rs", B_SRC);
    let patch = format!("{A_GOOD}{B_GOOD}");
    let out = run_stdin(dir.path(), &patch, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "fn aa() {}\n");
    assert_eq!(read(dir.path(), "b.rs"), "fn bb() {}\n");
}

#[test]
fn spec_23_1_carries_atomicity_sentence() {
    let spec = std::fs::read_to_string(workspace_root().join("SPEC.md")).unwrap();
    assert!(
        spec.contains("all-or-nothing across file sections"),
        "23.1 atomicity sentence drifted"
    );
}
