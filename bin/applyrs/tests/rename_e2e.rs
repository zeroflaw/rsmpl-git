//! T377: file rename sections (`rename from/to`, rename detection on)
//! lower back to rename sections and apply via `git apply`.
//!
//! A rename section seeds the new side from the OLD file's content, so
//! ordinary hunks (old-side coordinates) and RSmPL hunks both apply
//! transparently; pure renames lower headers-only (exactly git's shape).

mod common;
use common::*;

const RENAME_MODIFY: &str = "diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -2,1 +2,1 @@\n-line2\n+LINE2\n";

#[test]
fn rename_modify_real_apply_moves_and_edits() {
    // T377: rename+modify checks clean and real-applies: new path
    // carries the edit, old path is gone.
    let dir = init_repo();
    write(dir.path(), "old.txt", "line1\nline2\nline3\n");
    let out = run_stdin(dir.path(), RENAME_MODIFY, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), RENAME_MODIFY, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "new.txt"), "line1\nLINE2\nline3\n");
    assert!(
        std::fs::symlink_metadata(dir.path().join("old.txt")).is_err(),
        "old path must be gone"
    );
}

#[test]
fn pure_rename_checks_and_applies_headers_only() {
    // T377: a headers-only rename (git's pure-rename shape, no hunks)
    // lowers instead of silently dropping (the content-equal ladder
    // arm would emit nothing).
    const PURE: &str =
        "diff --git a/keep.txt b/kept.txt\nrename from keep.txt\nrename to kept.txt\n";
    let dir = init_repo();
    write(dir.path(), "keep.txt", "keep\n");
    let out = run_stdin(dir.path(), PURE, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), PURE, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "kept.txt"), "keep\n");
    assert!(
        std::fs::symlink_metadata(dir.path().join("keep.txt")).is_err(),
        "old path must be gone"
    );
}

#[test]
fn rsmpl_hunk_inside_rename_applies_against_old_content() {
    // T377: an RSmPL hunk in a rename section rewrites old-side
    // content and lowers as an ordinary rename+modify.
    const SEMANTIC_RENAME: &str = "diff --git a/a.rs b/b.rs\nrename from a.rs\nrename to b.rs\n@@ -1,3 +1,3 @@ rule=rename_calls\n# lang=rust\n# allow=many\n# expr E\n- old_call(E)\n+ new_call(E)\n";
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn foo() {\n    old_call(1);\n}\n");
    let out = run_stdin(dir.path(), SEMANTIC_RENAME, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), SEMANTIC_RENAME, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "b.rs"),
        "fn foo() {\n    new_call(1);\n}\n"
    );
    assert!(
        std::fs::symlink_metadata(dir.path().join("a.rs")).is_err(),
        "old path must be gone"
    );
}
