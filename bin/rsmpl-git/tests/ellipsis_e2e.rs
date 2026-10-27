//! T157/T159: argument-ellipsis replay (per-`...` remainders, not the whole
//! arg list) and leading/middle-`...` backtracking alignment.
//!
//! Split from `cli_e2e.rs` to keep every test file under the 2000-LOC
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
fn ellipsis_replay_applies_byte_exact() {
    // T157: `foo(E, ...)` replays the absorbed remainder, not the whole
    // arg list; empty remainders swallow the comma; `# repeat` folds.
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1, 2, 3);\n    foo(1);\n    foo(foo(1));\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,5 @@ rule=r\n# allow=many\n# repeat\n# expr E\n- foo(E, ...)\n+ bar(E, ...)\n";
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
    let text = read(dir.path(), "src/lib.rs");
    assert!(text.contains("bar(1, 2, 3);"), "got: {text}");
    assert!(text.contains("bar(1);"), "got: {text}");
    assert!(text.contains("bar(bar(1));"), "got: {text}");
}

#[test]
fn ellipsis_plus_without_minus_fails_fast_json() {
    // T157: `+ ...` with no `- ...` is a compile-time pattern conflict
    // (was a silent arg-list duplication).
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1, 2);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# allow=many\n# expr E\n# expr F\n- foo(E, F)\n+ bar(E, F, ...)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(!out.status.success(), "must fail");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("RSMPL_PATTERN_CONFLICT"), "got: {stdout}");
    assert!(stdout.contains("no matching `- ...`"), "got: {stdout}");
}

#[test]
fn ellipsis_leading_backtrack_applies() {
    // T159: `- foo(..., E)` on `foo(1, 2, 3)` binds the last arg via
    // backtracking (was a first-fit-only `no match found`).
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1, 2, 3);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# allow=many\n# expr E\n- foo(..., E)\n+ bar(E)\n";
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
    let text = read(dir.path(), "src/lib.rs");
    assert!(text.contains("bar(3);"), "got: {text}");
}
