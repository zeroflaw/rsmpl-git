//! T160: mixed `...` in fn param/body slots fails fast with
//! `RSMPL_PATTERN_CONFLICT` instead of silently corrupting or ignoring.
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
    fs::create_dir_all(dir.path().join("src")).unwrap();
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
fn mixed_param_ellipsis_fails_fast_text() {
    // T160 P1: `- fn foo(..., c: T)` + `+ fn foo(c: T)` used to emit a
    // literal `fn foo(c: T) {}` with exit 0 (unbound metavar leak). Now
    // the parser rejects the mixed `...` shape before matching.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(a: i32, b: i32, c: i32) {}\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=t\n# lang=rust\n# type T\n- fn foo(..., c: T) {\n+ fn foo(c: T) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("must stand alone"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), before, "worktree untouched");
}

#[test]
fn mixed_body_ellipsis_fails_fast_json() {
    // T160 P4: a plus body of `{ ..., foo(); }` used to silently
    // preserve the body (exit 0) without verifying `foo();`. Now the
    // parser rejects it; the stdout-only JSON envelope carries the code.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=t\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ..., foo();\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(!out.status.success(), "must fail");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("RSMPL_PATTERN_CONFLICT"), "got: {stdout}");
    assert!(stdout.contains("exactly `...`"), "got: {stdout}");
}
