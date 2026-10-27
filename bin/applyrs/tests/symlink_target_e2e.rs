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
