//! T212: a leading blank context line belongs to the ordinary hunk
//! body. Deleting a line right after a blank line applies byte-exact;
//! before the fix the hunk lowered to empty (`(no changes)`/`check:true`,
//! exit 0) while leaving the worktree untouched.

mod common;
use common::*;

#[test]
fn delete_after_blank_line_applies() {
    // T212: the hunk's first body line (`" "`) is a blank context line.
    let dir = init_repo();
    write(dir.path(), "f.md", "l1\n\nl3-STUB\nl4\n");
    let patch = "diff --git f.md\n@@ -2,3 +2,2 @@\n \n-l3-STUB\n l4\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("-l3-STUB"), "got: {stdout}");
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "f.md"), "l1\n\nl4\n");
}

#[test]
fn delete_after_blank_line_json() {
    // T212: same shape via `--format json` -- the report must name the
    // file and carry the deletion, and JSON apply must mutate the file.
    let dir = init_repo();
    write(dir.path(), "f.md", "l1\n\nl3-STUB\nl4\n");
    let patch = "diff --git f.md\n@@ -2,3 +2,2 @@\n \n-l3-STUB\n l4\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["files"][0]["path"], serde_json::json!("f.md"), "got: {v}");
    assert!(v["diff"].as_str().unwrap().contains("-l3-STUB"), "got: {v}");
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "f.md"), "l1\n\nl4\n");
}
