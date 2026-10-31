//! WI-10a: `applyrs --context N` presentation context.
//! The verified lowering stays 0-context (canonical form); N only renders
//! context around the same changes so `--output` artifacts apply with
//! vanilla `git apply`. Pins: context-3 output passes vanilla
//! `git apply --check`, and the post-tree (`files[].post_sha`) is
//! identical with `--context 0` vs `--context 3`.

mod common;
use common::*;
use std::process::Command;

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_alpha\n# scope=file\n- fn alpha(...) {\n+ fn beta(...) {\n      ...\n   }\n";

fn fixture() -> tempfile::TempDir {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn alpha() {\n    1\n}\n\nfn gamma() {\n    2\n}\n\nfn delta() {\n    3\n}\n",
    );
    write(dir.path(), "change.rsmpl", PATCH);
    dir
}

#[test]
fn context_output_applies_under_vanilla_git_apply() {
    let dir = fixture();
    let out = dir.path().join("out.patch");
    let patch = dir.path().join("change.rsmpl");
    let res = run(
        dir.path(),
        &[
            "--file",
            patch.to_str().unwrap(),
            "--check",
            "--context",
            "3",
            "--output",
            out.to_str().unwrap(),
        ],
    );
    assert!(
        res.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let text = std::fs::read_to_string(&out).unwrap();
    // Context lines ride the hunk (vanilla shape), not just -/+ lines.
    assert!(text.contains("\n     1\n"), "got: {text}");
    // Vanilla `git apply` (no --unidiff-zero) accepts the artifact.
    let check = Command::new("git")
        .args(["apply", "--check", out.to_str().unwrap()])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "vanilla apply failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn context_keeps_post_tree_identical() {
    let dir = fixture();
    let mut shas = Vec::new();
    let mut lines = Vec::new();
    for ctx in ["0", "3"] {
        let patch = dir.path().join("change.rsmpl");
        let res = run(
            dir.path(),
            &[
                "--file",
                patch.to_str().unwrap(),
                "--format",
                "json",
                "--diff-max-lines",
                "0",
                "--check",
                "--context",
                ctx,
            ],
        );
        assert!(
            res.status.success(),
            "ctx {ctx} stderr: {}",
            String::from_utf8_lossy(&res.stderr)
        );
        let v: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
        shas.push(v["files"].clone());
        lines.push(v["diff_lines"].clone());
    }
    assert_eq!(shas[0], shas[1], "context must not move the post-tree");
    assert!(
        lines[1].as_u64().unwrap_or(0) > lines[0].as_u64().unwrap_or(0),
        "context 3 should render more diff lines: {lines:?}"
    );
}
