//! WI-9f: seam insertion between touching item deletes composes (2322538).
//!
//! `delete_lines` absorbs one following blank per item, so adjacent item
//! deletes meet exactly at a line start (`[a,b)` + `[b,c)`). An ordinary
//! insertion anchored at that seam point composes (boundary — `overlaps()`
//! uses strict inequality), but pre-fix `coalesce_deletes` union-merged the
//! touching pair first, swallowing the seam and turning the insert into a
//! strictly-inside overlap (`GIT_HUNK_CONFLICT`). The coalesce now skips
//! touching-merges across a seam-anchored insertion; strict overlaps still
//! merge (and a strictly-inside insert still vetoes — second pin).

mod common;
use common::*;

const PRE: &str = "use top::keep;\nfn gone_one() {\n    one();\n}\n\nfn gone_two() {\n    two();\n}\n\nfn kept() {\n    kept();\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -2,4 +2,0 @@ rule=r1\n# allow=many\n- I\n\ndiff --git src/lib.rs\n@@ -5,0 +6,1 @@\n+// between\n\ndiff --git src/lib.rs\n@@ -6,4 +3,0 @@ rule=r2\n# allow=many\n- I\n";

const POST: &str = "use top::keep;\n// between\nfn kept() {\n    kept();\n}\n";

#[test]
fn seam_insertion_between_touching_deletes_composes() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", PRE);
    let out = run_stdin(dir.path(), PATCH, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, POST, "got: {got}");
}

#[test]
fn strictly_inside_insertion_still_vetoes_with_touching_deletes() {
    // Same deletes, but the insertion lands strictly inside r1's range
    // (after old line 3, inside `gone_one`) — still `GIT_HUNK_CONFLICT`.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", PRE);
    let patch = "diff --git src/lib.rs\n@@ -2,4 +2,0 @@ rule=r1\n# allow=many\n- I\n\ndiff --git src/lib.rs\n@@ -3,0 +4,1 @@\n+// between\n\ndiff --git src/lib.rs\n@@ -6,4 +3,0 @@ rule=r2\n# allow=many\n- I\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("GIT_HUNK_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("overlapping edits cannot be merged"),
        "got: {stderr}"
    );
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(got, PRE, "worktree must be untouched");
}
