//! T348: same-file multi-section pins (SPEC 10 original coordinates).
//!
//! Repeated `diff --git` sections for one path share original-source
//! coordinates (never rebased): close-range ordinary + semantic splits
//! merge exactly like one merged section, a rebased second section
//! fails content verification with `GIT_HUNK_CONFLICT`, and an
//! out-of-bounds second section reports `RSMPL_RANGE_CONFLICT` (same
//! fence as a single section — Pass 0 used to skip it and surface
//! `GIT_HUNK_CONFLICT` instead).

mod common;
use common::*;

#[test]
fn two_ordinary_sections_close_ranges_merge() {
    let dir = init_repo();
    write(
        dir.path(),
        "f.txt",
        "l01\nl02\nl03\nl04\nl05\nl06\nl07\nl08\nl09\nl10\n",
    );
    let patch = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -2,1 +2,1 @@\n-l02\n+L02\n\ndiff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -4,1 +4,1 @@\n-l04\n+L04\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "f.txt");
    assert!(got.contains("L02\n"), "got: {got}");
    assert!(got.contains("L04\n"), "got: {got}");
    assert!(!got.contains("l02\n"), "got: {got}");
    assert!(!got.contains("l04\n"), "got: {got}");
}

#[test]
fn two_semantic_sections_close_ranges_merge() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\nfn bar() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,1 @@ rule=r1\n# lang=rust\n- fn foo(...) {\n+ fn foo2(...) {\n      ...\n  }\n\ndiff --git a/src/lib.rs b/src/lib.rs\n@@ -2,1 @@ rule=r2\n# lang=rust\n- fn bar(...) {\n+ fn bar2(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("fn foo2()"), "got: {got}");
    assert!(got.contains("fn bar2()"), "got: {got}");
}

#[test]
fn rebased_second_section_fails_content_conflict() {
    // An insertion in section 1 does not shift section 2: citing the
    // post-insert (rebased) line fails verification with
    // `GIT_HUNK_CONFLICT`, while the original-coordinate twin succeeds.
    let dir = init_repo();
    let content = "l01\nl02\nl03\nl04\nl05\n";
    write(dir.path(), "f.txt", content);
    let rebased = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -2,0 +3,1 @@\n+X\n\ndiff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -5,1 +6,1 @@\n-l04\n+L04\n";
    let out = run_stdin(dir.path(), rebased, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("GIT_HUNK_CONFLICT"), "got: {stderr}");
    assert_eq!(read(dir.path(), "f.txt"), content);
    // Original-coordinate twin: insertion after line 2, edit of line 4.
    let original = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -2,0 +3,1 @@\n+X\n\ndiff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -4,1 +4,1 @@\n-l04\n+L04\n";
    let out = run_stdin(dir.path(), original, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn second_section_oob_is_range_conflict_text() {
    let dir = init_repo();
    write(dir.path(), "f.txt", "l01\nl02\nl03\n");
    let patch = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-l01\n+L01\n\ndiff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -90,2 +90,2 @@\n-xxx\n-zzz\n+yyy\n+www\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_RANGE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("90,2 exceeds file length"), "got: {stderr}");
    assert!(
        !stderr.contains("GIT_HUNK_CONFLICT"),
        "must not surface the old mis-coded fence, got: {stderr}"
    );
    assert_eq!(read(dir.path(), "f.txt"), "l01\nl02\nl03\n");
}

#[test]
fn second_section_oob_is_range_conflict_json() {
    let dir = init_repo();
    write(dir.path(), "f.txt", "l01\nl02\nl03\n");
    let patch = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-l01\n+L01\n\ndiff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -90,2 +90,2 @@\n-xxx\n-zzz\n+yyy\n+www\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
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
        serde_json::json!("RSMPL_RANGE_CONFLICT")
    );
    assert_eq!(read(dir.path(), "f.txt"), "l01\nl02\nl03\n");
}
