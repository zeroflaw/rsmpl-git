//! T308: engine-verbatim git-spawn failure pins (SPEC 25.7/31.7).
//!
//! When `git` is missing from `PATH`, the `git apply` / `git apply --check`
//! child fails to spawn after planning + lowering have succeeded. The step
//! fails with a context-less `GIT_HUNK_CONFLICT` (bare `reason:` text, JSON
//! explicit nulls, exit 2) — no `rule:`/`file:`/`range:` lines, unlike the
//! content-mismatch (`rule: ""`, range present) and overlap (`file:` present)
//! shapes. The `<os error>` suffix is platform-dependent; pins assert the
//! verbatim prefix. Sibling feed/wait arms share the envelope by
//! construction but never surface via the CLI (defensive backstops, SPEC
//! 31.7), so they carry no pin here. Worktree untouched on both paths.

use std::io::Write as _;
use std::process::{Command, Stdio};

mod common;
use common::*;

const PREFIX: &str = "failed to run `git apply` (is git installed?): ";

fn spawn_no_git(patch: &str, extra: &[&str]) -> (tempfile::TempDir, std::process::Output) {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let mut child = Command::new(binary())
        .args(extra)
        .arg("--repo")
        .arg(dir.path())
        .env("PATH", "/nonexistent-gitrs-bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (dir, out)
}

fn rename_patch() -> &'static str {
    "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n"
}

const CONTENT: &str = "fn foo(x: i32) -> i32 {\n    x + 1\n}\n";

#[test]
fn spawn_failure_rejected_verbatim_text() {
    let (dir, out) = spawn_no_git(rename_patch(), &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "apply-mode failure stdout must be empty, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("GIT_HUNK_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(PREFIX), "got: {stderr}");
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), CONTENT);
}

#[test]
fn spawn_failure_rejected_verbatim_json() {
    let (dir, out) = spawn_no_git(rename_patch(), &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["version"], serde_json::json!(1), "got: {v}");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("GIT_HUNK_CONFLICT"),
        "got: {v}"
    );
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap_or_default()
            .starts_with(PREFIX),
        "got: {v}"
    );
    for key in ["rule", "file", "range", "suggestion"] {
        assert!(
            v["error"][key].is_null(),
            "spawn envelope {key} must be null, got: {v}"
        );
    }
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !stderr.contains("GIT_HUNK_CONFLICT"),
        "JSON failure must stay stdout-only, got: {stderr}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), CONTENT);
}
