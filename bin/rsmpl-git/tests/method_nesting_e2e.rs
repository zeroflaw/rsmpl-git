//! T167: method-call receiver nesting (SPEC 9.3, 17.8).
//!
//! Receiver nesting shares the candidate start offset, so the call
//! matcher sorts longest-first on a shared start (mirrors T143):
//! the outer match wins, and `# repeat` folds the receiver binding
//! bottom-up.
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
    fs::write(dir.join(rel), text).unwrap()
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
fn method_receiver_nesting_outer_wins_e2e() {
    // T167: without `# repeat` the outer match rewrites once
    // (was `overlapping edits cannot be merged`).
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m() { x.to_string().to_string(); }\n",
    );
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=m\n# allow=many\n# expr R\n- R.to_string()\n+ R.to_owned()\n";
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
        "fn m() { x.to_string().to_owned(); }\n"
    );
}

#[test]
fn method_receiver_nesting_repeat_folds_e2e() {
    // T167: with `# repeat` the receiver binding folds bottom-up.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m() { x.to_string().to_string(); }\n",
    );
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=m\n# allow=many\n# repeat\n# expr R\n- R.to_string()\n+ R.to_owned()\n";
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
    assert_eq!(payload["rules"][0]["matches"], 1);
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn m() { x.to_owned().to_owned(); }\n"
    );
}
