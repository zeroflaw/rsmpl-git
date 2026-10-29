//! T491 bench-freshness pins, split from `fixtures_roundtrip.rs` to hold
//! that file's 2000-line budget (fixtures gate); helpers are duplicated,
//! not shared (T246 precedent: `live_path_e2e.rs`).
//!
//! Requires a workspace build so the sibling `applyrs` binary sits next
//! to `diffrs` (`cargo test --workspace`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diffrs"))
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

#[test]
fn example_bench_readme_table_matches_recomputed() {
    // T435 (T328-equivalent for diffrs): pin committed native.patch /
    // sem.patch freshness + bench-table numbers against rot. Staged
    // collection (add -A + diff --cached + diffrs --staged) so added files
    // are covered. Recompute
    // each bench hermetically (tmpdir, never rewriting the bench dir)
    // with the same byte/line/pct formula as example-benches/run.sh,
    // then assert the stored files are byte-identical and both READMEs
    // carry the exact recomputed row.
    let root = workspace_root();
    let example_readme =
        std::fs::read_to_string(root.join("example-benches").join("README.md")).unwrap();
    let top_readme = std::fs::read_to_string(root.join("README.md")).unwrap();
    let benches = root.join("example-benches").join("diffrs");
    let mut names: Vec<String> = std::fs::read_dir(&benches)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no diffrs benches found");
    let mut seen = 0;
    for name in names {
        let dir = benches.join(&name);
        if !dir.is_dir() {
            continue;
        }
        seen += 1;
        let tmp = tempfile::tempdir().unwrap();
        copy_tree(&dir.join("repo"), tmp.path());
        git_ok(tmp.path(), &["init", "-q"], &name);
        git_ok(tmp.path(), &["add", "-A"], &name);
        git_ok(
            tmp.path(),
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
        copy_tree(&dir.join("after"), tmp.path());
        git_ok(tmp.path(), &["add", "-A"], &name);
        let native = git(
            tmp.path(),
            &[
                "diff",
                "--cached",
                "--unified=0",
                "--no-color",
                "--no-ext-diff",
            ],
        );
        assert!(native.status.success(), "[{name}] git diff failed");
        let fresh_native: String = String::from_utf8_lossy(&native.stdout)
            .lines()
            .filter(|l| !l.starts_with("index "))
            .map(|l| format!("{l}\n"))
            .collect();
        let out = Command::new(binary())
            .arg("--repo")
            .arg(tmp.path())
            .arg("--staged")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "[{name}] diffrs failed");
        let fresh_sem = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            !fresh_sem.trim().is_empty(),
            "[{name}] diffrs output is empty"
        );
        let stored_native = std::fs::read(dir.join("native.patch")).unwrap();
        assert!(
            fresh_native.as_bytes() == stored_native.as_slice(),
            "[{name}] committed native.patch is stale (rebuild with example-benches/run.sh)"
        );
        let stored_sem = std::fs::read(dir.join("sem.patch")).unwrap();
        assert!(
            fresh_sem.as_bytes() == stored_sem.as_slice(),
            "[{name}] committed sem.patch is stale (rebuild with example-benches/run.sh)"
        );
        let nb = fresh_native.len();
        let nl = fresh_native.bytes().filter(|&b| b == b'\n').count();
        let sb = fresh_sem.len();
        let sl = fresh_sem.bytes().filter(|&b| b == b'\n').count();
        let pct = format!("{:.0}%", (1.0 - sb as f64 / nb as f64) * 100.0);
        let want = format!("{nb} B / {nl} ln | {sb} B / {sl} ln | {pct}");
        for (label, readme) in [
            ("example-benches/README.md", &example_readme),
            ("README.md", &top_readme),
        ] {
            let ok = readme
                .lines()
                .any(|line| line.contains(&name) && line.contains(&want));
            assert!(
                ok,
                "[{name}] {label} table row missing or stale, want line containing:\n{name} ... {want}"
            );
        }
    }
    assert!(seen > 0, "no diffrs bench dirs found");
}
