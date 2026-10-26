//! End-to-end tests driving the `rsmpl-git` binary in temp worktrees.

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

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn inline_fn_rename_applies() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run(dir.path(), &["--patch", patch]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read(dir.path(), "src/lib.rs");
    assert!(text.contains("fn bar(x: i32)"), "got: {text}");
}

#[test]
fn check_and_emit_diff_do_not_modify() {
    let dir = init_repo();
    let before = "fn a() {\n    foo(1);\n    foo(2);\n}\n";
    write(&dir.path().join(""), "src/lib.rs", before);
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, patch).unwrap();

    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--emit-diff",
            "--check",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("diff --git a/src/lib.rs b/src/lib.rs"),
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("# lang=rust"),
        "lowered diff must not contain directives"
    );
    assert!(
        stdout.contains("-    foo(1);") || stdout.contains("-    foo(2);"),
        "got: {stdout}"
    );
    // --check must not modify the file.
    assert_eq!(read(dir.path(), "src/lib.rs"), before);
}

#[test]
fn no_match_exits_two_with_structured_error() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run(dir.path(), &["--patch", patch]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
}
