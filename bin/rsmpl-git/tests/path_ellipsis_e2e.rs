//! T164: path ellipsis (SPEC 16.5) on call callees.
//!
//! Split into its own file to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git"))
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let st = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(st.success());
    dir
}

fn write(dir: &Path, rel: &str, text: &str) {
    fs::write(dir.join(rel), text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

fn run_stdin(dir: &Path, patch: &str, extra: &[&str]) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(binary())
        .args(extra)
        .arg("--repo")
        .arg(dir)
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
    child.wait_with_output().unwrap()
}

#[test]
fn path_ellipsis_rewrites_zero_one_and_many_segments() {
    // T164 dogfood probe: `- crate::...::old(E)` rewrites every spelling
    // of the path (zero, one, and many middle segments); the last-segment
    // gate still protects `other`.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m() {\n    crate::old(1);\n    crate::a::old(2);\n    crate::a::b::old(3);\n    crate::a::other(4);\n}\n",
    );
    let patch = "diff --git a.rs\n@@ -1,6 @@ rule=r\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::new(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "text: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn m() {\n    crate::new(1);\n    crate::new(2);\n    crate::new(3);\n    crate::a::other(4);\n}\n"
    );
}

#[test]
fn path_ellipsis_plus_side_stays_concrete_only() {
    // A `+` path `...` has no absorbed path segments to replay (only arg
    // remainders replay, T157), so the plus/minus gate fails fast with
    // `RSMPL_PATTERN_CONFLICT` instead of emitting a guess.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn m() { crate::a::old(1); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::...::new(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    assert_eq!(read(dir.path(), "a.rs"), "fn m() { crate::a::old(1); }\n");
}

#[test]
fn path_ellipsis_json_check_reports_three_matches() {
    // JSON envelope path: the dogfood probe reports all three sites.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m() { crate::old(1); crate::a::old(2); crate::a::b::old(3); }\n",
    );
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::new(E)\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "json: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(payload["check"], true);
    assert_eq!(payload["rules"][0]["matches"], 3);
}
