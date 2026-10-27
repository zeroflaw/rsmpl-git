//! T364: mode-only changes and empty files lower to appliable sections
//! instead of silently dropping (exit 0, empty lowering, zero effect).
//!
//! A pure mode change used to lower to nothing (content `old == new`);
//! an empty create lowered to `None` (`new.is_empty`) and an empty
//! delete likewise. Now they lower as headers-only sections -- exactly
//! what real git emits -- and `git apply` creates/removes the empty
//! file or flips the mode. Real-apply pins throughout (T361 doctrine:
//! `--check` is blind to silent drops).

mod common;
use common::*;

const EMPTY_CREATE_PATCH: &str =
    "diff --git a/empty.txt b/empty.txt\nnew file mode 100644\n--- /dev/null\n+++ b/empty.txt\n";
const EMPTY_CREATE_BARE_PATCH: &str = "diff --git a/bare.txt b/bare.txt\nnew file mode 100644\n";
const EMPTY_DELETE_PATCH: &str =
    "diff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n";
const EMPTY_DELETE_BARE_PATCH: &str =
    "diff --git a/gone2.txt b/gone2.txt\ndeleted file mode 100644\n";

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn empty_create_real_apply_creates_empty_file() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "x\n");
    let out = run_stdin(dir.path(), EMPTY_CREATE_PATCH, &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    let meta = std::fs::metadata(dir.path().join("empty.txt")).expect("empty.txt must exist");
    assert_eq!(meta.len(), 0, "must be empty");
}

#[test]
fn empty_create_bare_headers_real_apply_creates_empty_file() {
    // Bare headers (no `---`/`+++`/hunks) also parse and create.
    let dir = init_repo();
    let out = run_stdin(dir.path(), EMPTY_CREATE_BARE_PATCH, &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    let meta = std::fs::metadata(dir.path().join("bare.txt")).expect("bare.txt must exist");
    assert_eq!(meta.len(), 0, "must be empty");
}

#[test]
fn empty_delete_real_apply_removes_file() {
    let dir = init_repo();
    write(dir.path(), "gone.txt", "");
    let out = run_stdin(dir.path(), EMPTY_DELETE_PATCH, &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    assert!(
        std::fs::symlink_metadata(dir.path().join("gone.txt")).is_err(),
        "gone.txt must be gone"
    );
}

#[test]
fn empty_delete_bare_headers_real_apply_removes_file() {
    // Bare headers (no `---`/`+++`/hunks) also parse and remove.
    let dir = init_repo();
    write(dir.path(), "gone2.txt", "");
    let out = run_stdin(dir.path(), EMPTY_DELETE_BARE_PATCH, &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    assert!(
        std::fs::symlink_metadata(dir.path().join("gone2.txt")).is_err(),
        "gone2.txt must be gone"
    );
}

#[test]
fn mode_and_content_real_apply_changes_both() {
    let dir = init_repo();
    write(dir.path(), "f.txt", "old\n");
    const PATCH: &str = "diff --git a/f.txt b/f.txt\nold mode 100644\nnew mode 100755\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old\n+new\n";
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    assert_eq!(read(dir.path(), "f.txt"), "new\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("f.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "exec bit must be set");
    }
}

#[cfg(unix)]
#[test]
fn mode_only_real_apply_flips_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = init_repo();
    write(dir.path(), "run.sh", "#!/bin/sh\n");
    std::fs::set_permissions(
        dir.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    const PATCH: &str = "diff --git a/run.sh b/run.sh\nold mode 100755\nnew mode 100644\n";
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
    let mode = std::fs::metadata(dir.path().join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0, "exec bit must be cleared");
    assert_eq!(read(dir.path(), "run.sh"), "#!/bin/sh\n");
}
