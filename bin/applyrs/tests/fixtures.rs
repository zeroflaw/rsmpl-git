//! Fixture-driven e2e: each `test-fixtures/applyrs/NN-name/` holds `repo/`,
//! `patch.rsmpl`, and either `expected/` (exact post-state) or
//! `expect_fail.txt` (stderr substring + nonzero exit).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::*;

fn copy_tree(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn collect_files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            collect_files(&entry.path(), base, out);
        } else {
            out.push(entry.path().strip_prefix(base).unwrap().to_path_buf());
        }
    }
}

#[test]
fn fixtures_apply_through_binary() {
    let root = workspace_root();
    let fixtures = root.join("test-fixtures/applyrs");
    let mut names: Vec<String> = fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no fixtures found");

    for name in names {
        let dir = fixtures.join(&name);
        if !dir.is_dir() {
            continue;
        }
        let tmp = tempfile::tempdir().unwrap();
        copy_tree(&dir.join("repo"), tmp.path());
        let st = Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());

        let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_applyrs")))
            .arg("--file")
            .arg(dir.join("patch.rsmpl"))
            .arg("--repo")
            .arg(tmp.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();

        let fail_marker = dir.join("expect_fail.txt");
        if fail_marker.exists() {
            let want = fs::read_to_string(&fail_marker).unwrap();
            let want = want.trim();
            assert!(
                !out.status.success(),
                "[{name}] expected failure but succeeded"
            );
            assert!(
                stderr.contains(want),
                "[{name}] stderr missing {want:?}, got:\n{stderr}"
            );
            continue;
        }

        assert!(out.status.success(), "[{name}] failed:\n{stderr}");
        let expected = dir.join("expected");
        let mut files = Vec::new();
        collect_files(&expected, &expected, &mut files);
        assert!(!files.is_empty(), "[{name}] expected/ is empty");
        for rel in files {
            let want = fs::read(expected.join(&rel)).unwrap();
            let got = fs::read(tmp.path().join(&rel))
                .unwrap_or_else(|_| panic!("[{name}] missing output file {}", rel.display()));
            assert!(
                want == got,
                "[{name}] {} differs:\n--- want ---\n{}\n--- got ---\n{}",
                rel.display(),
                String::from_utf8_lossy(&want),
                String::from_utf8_lossy(&got),
            );
        }
    }
}

#[test]
fn no_rust_file_exceeds_2000_lines() {
    let root = workspace_root();
    let mut over = Vec::new();
    let mut stack = vec![root.join("bin"), root.join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let p = entry.path();
            if entry.file_type().unwrap().is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "target" {
                    continue;
                }
                stack.push(p);
            } else if p.extension().map(|e| e == "rs").unwrap_or(false) {
                let lines = fs::read_to_string(&p).unwrap().lines().count();
                if lines > 2000 {
                    over.push(format!("{} ({lines} lines)", p.display()));
                }
            }
        }
    }
    assert!(over.is_empty(), "files over 2000 LOC: {over:#?}");
}

#[test]
fn example_benches_stay_check_clean() {
    // T248: every example-bench patch.rsmpl must still lower + `git apply
    // --check` cleanly against its own repo/, so benches never rot.
    let root = workspace_root();
    let benches = root.join("example-benches/applyrs");
    let mut names: Vec<String> = fs::read_dir(&benches)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no benches found");
    for name in names {
        let dir = benches.join(&name);
        if !dir.is_dir() {
            continue;
        }
        let tmp = tempfile::tempdir().unwrap();
        copy_tree(&dir.join("repo"), tmp.path());
        let st = Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());
        let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_applyrs")))
            .arg("--file")
            .arg(dir.join("patch.rsmpl"))
            .arg("--repo")
            .arg(tmp.path())
            .arg("--check")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "[{name}] bench patch fails --check:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn example_bench_readme_table_matches_recomputed() {
    // T328: pin committed native.patch freshness + README savings-table
    // numbers against rot. Lowers each bench hermetically (to a tmpdir,
    // never rewriting the bench dir) with the same byte/line/pct formula
    // as example-benches/run.sh, then asserts the stored lowering is
    // byte-identical and the README carries the exact recomputed row.
    let root = workspace_root();
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    let benches = root.join("example-benches/applyrs");
    let mut names: Vec<String> = fs::read_dir(&benches)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no benches found");
    let mut seen = 0;
    for name in names {
        let dir = benches.join(&name);
        if !dir.is_dir() {
            continue;
        }
        seen += 1;
        let tmp = tempfile::tempdir().unwrap();
        copy_tree(&dir.join("repo"), tmp.path());
        let st = Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(st.success());
        let fresh = tmp.path().join("fresh.patch");
        let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_applyrs")))
            .arg("--file")
            .arg(dir.join("patch.rsmpl"))
            .arg("--repo")
            .arg(tmp.path())
            .arg("--output")
            .arg(&fresh)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "[{name}] bench patch fails to lower:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let fresh_bytes = fs::read(&fresh).unwrap();
        let stored = fs::read(dir.join("native.patch")).unwrap();
        assert!(
            fresh_bytes == stored,
            "[{name}] committed native.patch is stale (rebuild with example-benches/run.sh)"
        );
        let patch_bytes = fs::read(dir.join("patch.rsmpl")).unwrap();
        let rb = patch_bytes.len();
        let rl = patch_bytes.iter().filter(|&&b| b == b'\n').count();
        let nb = fresh_bytes.len();
        let nl = fresh_bytes.iter().filter(|&&b| b == b'\n').count();
        let pct = format!("{:.0}%", (1.0 - rb as f64 / nb as f64) * 100.0);
        // Bench-name cells carry human suffixes
        // (`01-rename-calls-many (12 sites, 1 hunk)`), so match the name
        // and the recomputed numeric cells on the same README line.
        let want = format!("{rb} B / {rl} ln | {nb} B / {nl} ln | {pct}");
        let ok = readme
            .lines()
            .any(|line| line.contains(&name) && line.contains(&want));
        assert!(
            ok,
            "[{name}] README table row missing or stale, want line containing:\n{name} ... {want}"
        );
    }
    assert!(seen > 0, "no bench dirs found");
}
