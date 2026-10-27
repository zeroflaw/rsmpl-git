//! T353: symlink-target reads (history dogfood: 53372c4).
//!
//! A patch target that is a symlink reads as its *target string* (git
//! mode-120000 blob content), never followed: following a link-to-dir
//! fails with EISDIR (`RSMPL_IO_CONFLICT`) and a link-to-file would
//! verify against the wrong bytes (`GIT_HUNK_CONFLICT`). Deleting a
//! symlink therefore checks clean and lowers with
//! `deleted file mode 120000` (which `git apply` type-checks; a 100644
//! mode fails `wrong type`). Unix-only: creating symlinks needs
//! `std::os::unix::fs::symlink`.

#![cfg(unix)]

mod common;
use common::*;
use std::os::unix::fs::symlink;

const LINK_TO_DIR_PATCH: &str = "diff --git gone\n--- a/gone\n+++ /dev/null\n@@ -1,1 +1,0 @@\n-real\n\\ No newline at end of file\n";
const LINK_TO_FILE_PATCH: &str = "diff --git alias.rs\n--- a/alias.rs\n+++ /dev/null\n@@ -1,1 +1,0 @@\n-real.rs\n\\ No newline at end of file\n";
const LINK_CREATE_PATCH: &str = "diff --git fresh\nnew file mode 120000\n--- /dev/null\n+++ b/fresh\n@@ -0,0 +1,1 @@\n+real\n\\ No newline at end of file\n";
const LINK_RETARGET_PATCH: &str = "diff --git gone\n--- a/gone\n+++ b/gone\n@@ -1,1 +1,1 @@\n-real\n\\ No newline at end of file\n+other\n\\ No newline at end of file\n";

#[test]
fn symlink_to_dir_delete_checks_clean_with_120000_mode() {
    let dir = init_repo();
    std::fs::create_dir(dir.path().join("real")).unwrap();
    symlink("real", dir.path().join("gone")).unwrap();
    let out = run_stdin(dir.path(), LINK_TO_DIR_PATCH, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("deleted file mode 120000"), "got: {stdout}");
}

#[test]
fn symlink_to_file_delete_verifies_link_string() {
    let dir = init_repo();
    write(dir.path(), "real.rs", "fn real() {}\n");
    symlink("real.rs", dir.path().join("alias.rs")).unwrap();
    // If the read followed the link, `-real.rs` would mismatch the
    // target's `fn real() {}` content (GIT_HUNK_CONFLICT).
    let out = run_stdin(dir.path(), LINK_TO_FILE_PATCH, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn symlink_delete_apply_removes_link_keeps_target() {
    let dir = init_repo();
    std::fs::create_dir(dir.path().join("real")).unwrap();
    symlink("real", dir.path().join("gone")).unwrap();
    let out = run_stdin(dir.path(), LINK_TO_DIR_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::symlink_metadata(dir.path().join("gone")).is_err());
    assert!(dir.path().join("real").is_dir());
}

#[test]
fn symlink_create_real_apply_materializes_link() {
    // T355: a `120000` create must lower as `120000` with exact target
    // bytes — `--check` alone masked the 100644 downgrade.
    let dir = init_repo();
    std::fs::create_dir(dir.path().join("real")).unwrap();
    let out = run_stdin(dir.path(), LINK_CREATE_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let target = std::fs::read_link(dir.path().join("fresh")).expect("fresh must be a symlink");
    assert_eq!(target.to_string_lossy(), "real");
}

#[test]
fn file_to_symlink_typechange_real_apply_swaps_type() {
    // T357: git's delete+create pair for one path must lower as the
    // pair (differing modes) — a plain modify keeps the old type.
    const TYPECHANGE: &str = "diff --git a/f b/f\ndeleted file mode 100644\nindex 1111111..0000000\n--- a/f\n+++ /dev/null\n@@ -1 +0,0 @@\n-hello\ndiff --git a/f b/f\nnew file mode 120000\nindex 0000000..2222222\n--- /dev/null\n+++ b/f\n@@ -0,0 +1 @@\n+target\n\\ No newline at end of file\n";
    let dir = init_repo();
    write(dir.path(), "f", "hello\n");
    let out = run_stdin(dir.path(), TYPECHANGE, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let target = std::fs::read_link(dir.path().join("f")).expect("f must become a symlink");
    assert_eq!(target.to_string_lossy(), "target");
}

#[test]
fn symlink_to_file_typechange_real_apply_swaps_type() {
    // T357 reverse direction: 120000 delete + 100644 create.
    const TYPECHANGE: &str = "diff --git a/f b/f\ndeleted file mode 120000\nindex 2222222..0000000\n--- a/f\n+++ /dev/null\n@@ -1 +0,0 @@\n-target\n\\ No newline at end of file\ndiff --git a/f b/f\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/f\n@@ -0,0 +1 @@\n+hello\n";
    let dir = init_repo();
    symlink("target", dir.path().join("f")).unwrap();
    let out = run_stdin(dir.path(), TYPECHANGE, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let md = std::fs::symlink_metadata(dir.path().join("f")).unwrap();
    assert!(!md.file_type().is_symlink(), "f must become a regular file");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f")).unwrap(),
        "hello\n"
    );
}

#[test]
fn symlink_retarget_real_apply_keeps_exact_target() {
    // T355: retargeting must not append `\n` (was `'other\n'`, dangling).
    let dir = init_repo();
    std::fs::create_dir(dir.path().join("real")).unwrap();
    std::fs::create_dir(dir.path().join("other")).unwrap();
    symlink("real", dir.path().join("gone")).unwrap();
    let out = run_stdin(dir.path(), LINK_RETARGET_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let target = std::fs::read_link(dir.path().join("gone")).expect("gone must stay a symlink");
    assert_eq!(target.to_string_lossy(), "other");
}
