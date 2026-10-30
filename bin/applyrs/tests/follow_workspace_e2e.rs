//! WI-7a: git-aware `# follow=uses` workspace set (SPEC 14.8).
//!
//! Inside a git worktree the follow search covers tracked +
//! untracked-but-not-ignored `.rs` files and never touches ignored ones;
//! outside a worktree it falls back to the lexical walk. The JSON report
//! carries `follow_files`/`follow_source` on follow runs only.

mod common;
use common::*;

use std::process::Command;

fn commit_all(dir: &std::path::Path) {
    let st = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(st.success());
    let st = Command::new("git")
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ])
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(st.success());
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON")
}

const PATCH: &str =
    "diff --git src/a.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";

#[test]
fn ignored_file_is_neither_rewritten_nor_vetoing() {
    // P0 bug fixed: the old lexical walk searched `.gitignore`d files.
    let dir = init_repo();
    write(dir.path(), ".gitignore", "ignored/\n");
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "ignored/ignored.rs",
        "fn f(x: Old) -> Old {\n    x\n}\n",
    );
    commit_all(dir.path());
    let out = run_stdin(
        dir.path(),
        PATCH,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/a.rs"), "pub struct New;\n");
    assert_eq!(
        read(dir.path(), "ignored/ignored.rs"),
        "fn f(x: Old) -> Old {\n    x\n}\n",
        "ignored file must stay byte-identical"
    );
    let v = json_of(&out);
    assert_eq!(v["follow_source"], "git", "got: {v}");
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(!files.contains(&"ignored/ignored.rs"), "got: {v}");
}

#[test]
fn untracked_nonignored_use_is_rewritten_and_reported() {
    // `--others --exclude-standard`: untracked but not ignored stays in
    // the search set, whether or not it is committed.
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    commit_all(dir.path());
    write(
        dir.path(),
        "src/b.rs",
        "use crate::Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        PATCH,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/a.rs"), "pub struct New;\n");
    assert_eq!(
        read(dir.path(), "src/b.rs"),
        "use crate::New;\nfn f(x: New) -> New {\n    x\n}\n"
    );
    let v = json_of(&out);
    assert_eq!(v["follow_source"], "git", "got: {v}");
    assert_eq!(v["follow_files"], 2, "got: {v}");
}

#[test]
fn non_git_dir_falls_back_to_walk() {
    // No worktree: the lexical walk still finds every `.rs` file.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(dir.path(), "src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n");
    let out = run_stdin(
        dir.path(),
        PATCH,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/a.rs"), "pub struct New;\n");
    assert_eq!(
        read(dir.path(), "src/b.rs"),
        "fn f(x: New) -> New {\n    x\n}\n"
    );
    let v = json_of(&out);
    assert_eq!(v["follow_source"], "walk", "got: {v}");
    assert_eq!(v["follow_files"], 2, "got: {v}");
}

#[test]
fn non_follow_runs_carry_no_follow_keys() {
    // Report minimalism: plain renames report no workspace provenance.
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    let patch = "diff --git src/a.rs\n@@ -1,1 @@ rule=rename\n- struct Old\n+ struct New\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    assert!(v.get("follow_files").is_none(), "got: {v}");
    assert!(v.get("follow_source").is_none(), "got: {v}");
}
