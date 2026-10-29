//! WI-2 follow-up: absolute `diffrs --format json` hunks-budget pin.
//!
//! SPEC D.4 budget: bytes beyond `patch` ≤ `150 + 150·H + 10·T`
//! (`H` = hunks, `T` = total sites). Multi-file fixture so the pin
//! applies on a realistic report, not a toy single hunk.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diffrs"))
}

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn commit_all(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    );
}

fn write(dir: &Path, rel: &str, text: &str) {
    if let Some(parent) = Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(dir.join(parent)).unwrap();
        }
    }
    std::fs::write(dir.join(rel), text).unwrap()
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
fn json_hunks_within_absolute_budget() {
    let dir = init_repo();
    for i in 0..6 {
        let mut text = String::from("fn foo() {\n");
        for j in 0..10 {
            text.push_str(&format!("    old_call({j});\n"));
        }
        text.push_str("}\n");
        write(dir.path(), &format!("src/f{i:02}.rs"), &text);
    }
    commit_all(dir.path());
    for i in 0..6 {
        let rel = format!("src/f{i:02}.rs");
        let path = dir.path().join(&rel);
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("old_call", "new_call");
        std::fs::write(&path, text).unwrap();
    }
    let out = run(dir.path(), &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is JSON");
    let patch = v["patch"].as_str().unwrap();
    assert!(!patch.trim().is_empty(), "got: {v}");
    let hunks = v["hunks"].as_array().unwrap();
    assert!(!hunks.is_empty(), "got: {v}");
    let h = hunks.len();
    let t: usize = hunks
        .iter()
        .map(|e| e["sites"].as_array().map(|a| a.len()).unwrap_or(0))
        .sum();
    let overhead = stdout.len() - patch.len();
    let budget = 150 + 150 * h + 10 * t;
    assert!(
        overhead <= budget,
        "overhead {overhead} B exceeds budget {budget} B (H={h} T={t}): {v}"
    );
    for e in hunks {
        assert!(e.get("rule").is_some(), "got: {v}");
        assert!(e.get("file").is_some(), "got: {v}");
        assert!(e.get("detector").is_some(), "got: {v}");
        assert!(e.get("totality").is_some(), "got: {v}");
        assert!(e.get("saved_bytes").is_some(), "got: {v}");
    }
}
