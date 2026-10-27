//! Fixture-driven e2e: each `test-fixtures/NN-name/` holds `repo/`,
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
    let fixtures = root.join("test-fixtures");
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

        let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git")))
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
    let benches = root.join("example-benches");
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
        let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git")))
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
