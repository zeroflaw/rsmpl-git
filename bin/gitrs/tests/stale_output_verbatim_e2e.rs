//! T317: stale-`--output` removal-failure verbatim pins (feeds T246).
//!
//! An empty lowering removes a pre-existing `--output` file (mirror
//! invariant, SPEC D.2.3). When removal fails the run aborts with
//! `RSMPL_IO_CONFLICT` before `--check`/apply, carrying
//! `reason: cannot remove stale <path> (empty lowering): <os error>`
//! with `file:` echoing the `--output` path (null rule/range/
//! suggestion omitted from text, explicit nulls in JSON). The sibling
//! `cannot write ...` shape (unwritable target) is pinned by T268;
//! this file pins the removal twin.
//!
//! Trigger: `# optional` no-match hunk (empty lowering) + a directory
//! at the `--output` path, so `remove_file` fails deterministically
//! with `Is a directory (os error 21)` even as root.

mod common;
use common::*;

const BASE: &str = "fn foo() {}\n";
const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# optional\n# allow=many\n# expr E\n- does_not_exist_xyz(E)\n+ also_missing(E)\n";
const REASON_HEAD: &str = "cannot remove stale ";
const REASON_MID: &str = " (empty lowering): ";

fn setup() -> (tempfile::TempDir, String) {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BASE);
    let out = dir.path().join("outdir");
    std::fs::create_dir_all(&out).unwrap();
    let out_arg = out.to_str().unwrap().to_string();
    (dir, out_arg)
}

#[test]
fn stale_removal_fails_text_apply_untouched() {
    let (dir, out_arg) = setup();
    let out = run_stdin(dir.path(), PATCH, &["--output", &out_arg]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains(&format!("file: {out_arg}")),
        "got: {stderr}"
    );
    assert!(stderr.contains(REASON_HEAD), "got: {stderr}");
    assert!(stderr.contains(REASON_MID), "got: {stderr}");
    assert!(stderr.contains(&out_arg), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), BASE);
    assert!(dir.path().join("outdir").is_dir());
}

#[test]
fn stale_removal_fails_json_envelope_untouched() {
    let (dir, out_arg) = setup();
    let out = run_stdin(
        dir.path(),
        PATCH,
        &["--format", "json", "--output", &out_arg],
    );
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert_eq!(v["error"]["file"], serde_json::json!(&out_arg));
    let reason = v["error"]["reason"].as_str().unwrap_or_default();
    assert!(reason.contains(REASON_HEAD), "got: {v}");
    assert!(reason.contains(REASON_MID), "got: {v}");
    assert!(reason.contains(&out_arg), "got: {v}");
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    assert_eq!(read(dir.path(), "src/lib.rs"), BASE);
    assert!(dir.path().join("outdir").is_dir());
}

#[test]
fn stale_removal_fails_check_mode_untouched() {
    let (dir, out_arg) = setup();
    let out = run_stdin(dir.path(), PATCH, &["--output", &out_arg, "--check"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains(&format!("file: {out_arg}")),
        "got: {stderr}"
    );
    assert!(stderr.contains(REASON_HEAD), "got: {stderr}");
    assert!(stderr.contains(REASON_MID), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), BASE);
    assert!(dir.path().join("outdir").is_dir());
}

#[test]
fn spec_carries_stale_removal_fence() {
    let root = workspace_root();
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md readable");
    for needle in ["cannot remove stale", "(empty lowering):", "cannot write"] {
        assert!(spec.contains(needle), "SPEC.md must carry `{needle}`");
    }
}
