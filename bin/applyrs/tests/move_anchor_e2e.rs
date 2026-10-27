//! T325: insertion-anchor + cut-absorption move mechanics.
//!
//! SPEC 10.5: `-0,0` pastes at the start of the file, `-N,0` pastes
//! after line N, and `-len-1,0` is the EOF spelling (equivalent to
//! `-len,0`); anything further fails `RSMPL_RANGE_CONFLICT`. Cuts
//! absorb one adjacent blank separator (following preferred, else
//! preceding at EOF); the capture excludes the separator. SPEC 20.2:
//! same-anchor multi-pastes land in document order (first hunk topmost)
//! with exactly one blank line between items.

mod common;
use common::*;

const START_SOURCE: &str = "fn a() {}\nfn b() {}\n";
const START_PATCH: &str = "diff --git src/lib.rs\n@@ -2,1 @@ rule=cut\n# item I\n# bind=H\n- I\n@@ -0,0 @@ rule=paste\n# use=H\n+ H\n";
const START_EXPECTED: &str = "fn b() {}\nfn a() {}\n";

const MID_SOURCE: &str = "fn a() {}\n\nfn c() {}\n";
const MID_PATCH: &str = "diff --git src/lib.rs\n@@ -3,1 @@ rule=cut\n# item I\n# bind=H\n- I\n@@ -1,0 @@ rule=paste\n# use=H\n+ H\n";
const MID_EXPECTED: &str = "fn a() {}\nfn c() {}\n";

const ABSORB_SOURCE: &str = "fn a() {}\n\nfn b() {}\n\nfn c() {}\n";
const ABSORB_PATCH: &str = "diff --git src/lib.rs\n@@ -3,1 @@ rule=cut\n# item I\n# bind=H\n- I\n@@ -6,0 @@ rule=paste\n# use=H\n+ H\n";
const ABSORB_EXPECTED: &str = "fn a() {}\n\nfn c() {}\nfn b() {}\n";

const DOUBLE_SOURCE: &str = "fn a() {}\nfn b() {}\nfn c() {}\n";
const DOUBLE_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=cut1\n# item I\n# bind=H1\n- I\n@@ -2,1 @@ rule=cut2\n# item I\n# bind=H2\n- I\n@@ -3,0 @@ rule=paste1\n# use=H1\n+ H1\n@@ -3,0 @@ rule=paste2\n# use=H2\n+ H2\n";
const DOUBLE_EXPECTED: &str = "fn c() {}\nfn a() {}\n\nfn b() {}\n";

const EOF_PASTE_N: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=cut\n# item I\n# bind=H\n- I\n@@ -2,0 @@ rule=paste\n# use=H\n+ H\n";
const EOF_PASTE_NP1: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=cut\n# item I\n# bind=H\n- I\n@@ -3,0 @@ rule=paste\n# use=H\n+ H\n";
const EOF_SOURCE: &str = "fn a() {}\nfn b() {}\n";
const EOF_EXPECTED: &str = "fn b() {}\nfn a() {}\n";

const OOB_SOURCE: &str = "fn a() {}\nfn b() {}\n";
const OOB_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=cut\n# item I\n# bind=H\n- I\n@@ -4,0 @@ rule=paste\n# use=H\n+ H\n";

#[test]
fn start_of_file_paste_moves_item_to_top() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", START_SOURCE);
    let out = run_stdin(dir.path(), START_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), START_EXPECTED);
}

#[test]
fn mid_file_paste_lands_after_anchor_line() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", MID_SOURCE);
    let out = run_stdin(dir.path(), MID_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), MID_EXPECTED);
}

#[test]
fn cut_absorbs_following_blank_eof_paste_appends() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", ABSORB_SOURCE);
    let out = run_stdin(dir.path(), ABSORB_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), ABSORB_EXPECTED);
}

#[test]
fn same_anchor_double_paste_keeps_order_with_blank_join() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", DOUBLE_SOURCE);
    let out = run_stdin(dir.path(), DOUBLE_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), DOUBLE_EXPECTED);
}

#[test]
fn len_plus_one_anchor_is_eof_spelling() {
    // `-2,0` and `-3,0` on a 2-line file both land at EOF.
    for patch in [EOF_PASTE_N, EOF_PASTE_NP1] {
        let dir = init_repo();
        write(dir.path(), "src/lib.rs", EOF_SOURCE);
        let out = run_stdin(dir.path(), patch, &[]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "patch {patch:?} stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(read(dir.path(), "src/lib.rs"), EOF_EXPECTED);
    }
}

#[test]
fn out_of_bounds_anchor_fails_range_conflict() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", OOB_SOURCE);
    let out = run_stdin(dir.path(), OOB_PATCH, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_RANGE_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("old range 4,0 exceeds file length (2 lines)"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), OOB_SOURCE);
}
