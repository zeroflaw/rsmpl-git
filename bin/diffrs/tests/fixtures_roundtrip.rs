//! T433 fixture-driven diffrs full cycle: each
//! `test-fixtures/diffrs/NN-name/` holds `repo/` (pre-state) and
//! `expected/` (post-state). There is no `patch.rsmpl` — the semantic
//! patch is the tool output, not the input. Per fixture the test runs
//! native `git diff --unified=0` -> `diffrs` semantic ->
//! `applyrs --output` native, then real-applies the final native patch
//! with bare `git apply --unidiff-zero` onto a fresh pre-state tree and
//! byte-compares against `expected/`.
//!
//! Patch bytes are never compared directly: header styles differ by
//! design (diffrs emits single-path `diff --git <path>` with no `index`
//! lines, T374; git emits `a/ b/` prefixes), so end-state equivalence
//! is the cycle equality (same lesson as T361: `--check` alone is
//! blind, byte-compare the applied tree).
//!
//! Requires a workspace build so the sibling `applyrs` binary sits next
//! to `diffrs` (`cargo test --workspace`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diffrs"))
}

fn sibling_applyrs() -> PathBuf {
    let me = binary();
    let file = if cfg!(windows) {
        "applyrs.exe".to_string()
    } else {
        "applyrs".to_string()
    };
    me.parent().map(|p| p.join(&file)).unwrap()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn git_ok(dir: &Path, args: &[&str], what: &str) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "{what}: git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn copy_tree(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn sync_expected_deletions(work: &Path, repo_dir: &Path, expected_dir: &Path) {
    let mut pre = Vec::new();
    collect_files(repo_dir, repo_dir, &mut pre);
    for rel in pre {
        if !expected_dir.join(&rel).exists() {
            let victim = work.join(&rel);
            if victim.is_file() {
                std::fs::remove_file(&victim).unwrap();
            }
        }
    }
}

fn assert_deletions_gone(
    result: &Path,
    repo_dir: &Path,
    expected_dir: &Path,
    name: &str,
    stage: &str,
) {
    let mut pre = Vec::new();
    collect_files(repo_dir, repo_dir, &mut pre);
    for rel in pre {
        if !expected_dir.join(&rel).exists() {
            assert!(
                !result.join(&rel).exists(),
                "[{name}] {stage}: deleted file {} still present",
                rel.display()
            );
        }
    }
}

fn collect_files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            collect_files(&entry.path(), base, out);
        } else {
            out.push(entry.path().strip_prefix(base).unwrap().to_path_buf());
        }
    }
}

fn assert_tree_matches(result: &Path, expected: &Path, name: &str, stage: &str) {
    let mut files = Vec::new();
    collect_files(expected, expected, &mut files);
    assert!(!files.is_empty(), "[{name}] expected/ is empty");
    for rel in files {
        let want = std::fs::read(expected.join(&rel)).unwrap();
        let got = std::fs::read(result.join(&rel))
            .unwrap_or_else(|_| panic!("[{name}] {stage}: missing output file {}", rel.display()));
        assert!(want == got, "[{name}] {stage}: {} differs", rel.display());
    }
}

#[test]
fn native_to_semantic_to_native_round_trip() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let fixtures = workspace_root().join("test-fixtures").join("diffrs");
    let mut names: Vec<String> = std::fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no diffrs fixtures found");

    for name in names {
        let dir = fixtures.join(&name);
        if !dir.is_dir() {
            continue;
        }

        // 1. Pre-state repo + post-state worktree change.
        let work = tempfile::tempdir().unwrap();
        copy_tree(&dir.join("repo"), work.path());
        git_ok(work.path(), &["init", "-q"], &name);
        git_ok(work.path(), &["add", "-A"], &name);
        git_ok(
            work.path(),
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
            &name,
        );
        copy_tree(&dir.join("expected"), work.path());
        // Deleted-file post-states: drop worktree files absent from expected/
        // (full-tree fixture convention; identical sets are a no-op).
        sync_expected_deletions(work.path(), &dir.join("repo"), &dir.join("expected"));

        // 2. Native input: zero-context git diff, `index` lines stripped
        // (diffrs never emits them, T374 — strip for a fair byte count).
        let native = git(
            work.path(),
            &["diff", "--unified=0", "--no-color", "--no-ext-diff"],
        );
        assert!(native.status.success(), "[{name}] git diff failed");
        let native_input: String = String::from_utf8_lossy(&native.stdout)
            .lines()
            .filter(|l| !l.starts_with("index "))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(!native_input.is_empty(), "[{name}] fixture has no change");

        // 3. Semantic compression.
        let out = Command::new(binary())
            .arg("--repo")
            .arg(work.path())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "[{name}] diffrs failed");
        let semantic = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(!semantic.trim().is_empty(), "[{name}] diffrs is empty");

        // 4. Back to the pre-change tree: semantic must check clean.
        git_ok(work.path(), &["stash", "-q"], &name);
        let checked = Command::new(sibling_applyrs())
            .arg("--patch")
            .arg(&semantic)
            .arg("--check")
            .arg("--repo")
            .arg(work.path())
            .output()
            .unwrap();
        assert_eq!(
            checked.status.code(),
            Some(0),
            "[{name}] semantic fails --check: {}",
            String::from_utf8_lossy(&checked.stderr)
        );

        // 5. Lower the semantic patch back to native.
        let lowered = work.path().join("roundtrip.patch");
        let out = Command::new(sibling_applyrs())
            .arg("--patch")
            .arg(&semantic)
            .arg("--repo")
            .arg(work.path())
            .arg("--output")
            .arg(&lowered)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "[{name}] --output failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        // 6. The final step matches the input: bare `git apply` of the
        // lowered native patch onto a fresh pre-state tree reproduces
        // `expected/` byte-for-byte.
        let verify = tempfile::tempdir().unwrap();
        copy_tree(&dir.join("repo"), verify.path());
        git_ok(verify.path(), &["init", "-q"], &name);
        let patch_bytes = std::fs::read(&lowered).unwrap();
        let mut child = Command::new("git")
            .args(["apply", "--unidiff-zero"])
            .current_dir(verify.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(&patch_bytes)
            .unwrap();
        let applied = child.wait_with_output().unwrap();
        assert!(
            applied.status.success(),
            "[{name}] git apply of lowered patch failed: {}",
            String::from_utf8_lossy(&applied.stderr)
        );
        assert_tree_matches(
            verify.path(),
            &dir.join("expected"),
            &name,
            "native round-trip",
        );
        assert_deletions_gone(
            verify.path(),
            &dir.join("repo"),
            &dir.join("expected"),
            &name,
            "native round-trip",
        );

        // 7. Per-fixture shape pins.
        if name == "01-call-sweep" {
            assert!(
                semantic.contains("rule="),
                "sweep must compress, got: {semantic}"
            );
            assert!(semantic.contains("- foo("), "got: {semantic}");
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "02-ordinary-edit" {
            assert!(
                !semantic.contains("rule="),
                "ordinary edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
        } else if name == "03-deleted-file-move" {
            assert!(
                semantic.contains("rule=move_big"),
                "deleted-file move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big"),
                "deleted-file move must paste, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn big("),
                "minus lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn big("),
                "plus lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "04-struct-move" {
            assert!(
                semantic.contains("rule=move_Big"),
                "struct move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_Big"),
                "struct move must paste, got: {semantic}"
            );
            assert!(
                !semantic.contains("-struct Big"),
                "minus lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+struct Big"),
                "plus lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        }
    }
}
