//! T318: repo-file read-failure verbatim pins (feeds T246).
//!
//! Loading a patch target that cannot be read (here a directory stands
//! where the target file should be, so `read_to_string` fails
//! deterministically with `Is a directory (os error 21)` even as root)
//! aborts with `RSMPL_IO_CONFLICT` before any matching/lowering,
//! carrying `reason: cannot read <abs repo-joined>: <os error>` with
//! `file:` echoing the repo-relative target (null rule/range/
//! suggestion omitted from text, explicit nulls in JSON). Contrast the
//! patch-file twin (`cannot read patch file <arg>: ...`, pinned via
//! SPEC 25.8's existing fence): that reason carries the `patch file`
//! infix and echoes the `--file` argument, while the repo twin echoes
//! the absolute repo-joined path with no `patch file` infix.

mod common;
use common::*;

const PATCH: &str = "diff --git blocked.rs\n@@ -1,1 @@ rule=x\n# scope=file\n# allow=many\n# expr E\n- f(E)\n+ g(E)\n";
const REASON_HEAD: &str = "cannot read ";

fn setup() -> (tempfile::TempDir, String) {
    let dir = init_repo();
    std::fs::create_dir_all(dir.path().join("blocked.rs")).unwrap();
    let abs = dir.path().join("blocked.rs").to_str().unwrap().to_string();
    (dir, abs)
}

#[test]
fn repo_read_fails_text_apply_untouched() {
    let (dir, abs) = setup();
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("file: blocked.rs"), "got: {stderr}");
    assert!(stderr.contains(REASON_HEAD), "got: {stderr}");
    assert!(stderr.contains(&abs), "got: {stderr}");
    assert!(!stderr.contains("patch file"), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(dir.path().join("blocked.rs").is_dir());
}

#[test]
fn repo_read_fails_json_envelope_untouched() {
    let (dir, abs) = setup();
    let out = run_stdin(dir.path(), PATCH, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert_eq!(v["error"]["file"], serde_json::json!("blocked.rs"));
    let reason = v["error"]["reason"].as_str().unwrap_or_default();
    assert!(reason.contains(REASON_HEAD), "got: {v}");
    assert!(reason.contains(&abs), "got: {v}");
    assert!(!reason.contains("patch file"), "got: {v}");
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    assert!(dir.path().join("blocked.rs").is_dir());
}

#[test]
fn repo_read_fails_check_mode_untouched() {
    let (dir, abs) = setup();
    let out = run_stdin(dir.path(), PATCH, &["--check"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("file: blocked.rs"), "got: {stderr}");
    assert!(stderr.contains(REASON_HEAD), "got: {stderr}");
    assert!(stderr.contains(&abs), "got: {stderr}");
    assert!(dir.path().join("blocked.rs").is_dir());
}

#[test]
fn spec_carries_repo_read_fence() {
    let root = workspace_root();
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md readable");
    for needle in [
        "cannot read <abs repo-joined>",
        "no `patch file` infix",
        "cannot read stdin",
    ] {
        assert!(spec.contains(needle), "SPEC.md must carry `{needle}`");
    }
}
