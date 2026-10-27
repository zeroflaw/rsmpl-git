//! T346: history-per-commit regression generator.
//!
//! Converts a curated selection of this repository's own commits
//! with the real `diffrs` binary (the T346 recipe: `checkout c` +
//! `diffrs c~1`, empty-tree rev for the root) and round-trips each
//! captured patch through the sibling `applyrs --check` against the
//! pre-change tree. A failing conversion fails this test — that is
//! the pin mechanism: reproduce with the recipe, then extend the
//! table.
//!
//! The table pins exact semantic-hunk counts, so both inference
//! improvements (more compression) and regressions (conversion
//! failures, lost compression) show up here. Harness notes: one
//! scratch `git worktree` under a `TempDir` (the real checkout is
//! never touched); skips — never hard errors — outside a checkout,
//! when `worktree add` fails, or when a curated commit is absent
//! (shallow clones); `applyrs` always runs via `--file` because the
//! large selections exceed the shell inline-arg limit.

use std::path::{Path, PathBuf};
use std::process::Command;

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
/// `git hash-object -t tree /dev/null`: the empty tree, i.e. the
/// `diffrs [rev]` that reproduces the root commit.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

struct Case {
    /// Full SHA (short hashes could collide one day).
    commit: &'static str,
    /// `diffrs [rev]` argument; `None` selects `{commit}~1`.
    rev: Option<&'static str>,
    /// Root commit: the pre-change tree is an empty dir, not a checkout.
    root: bool,
    /// Pinned `^@@ -1,1 @@ rule=` header count.
    expect_sem: usize,
}

const CASES: &[Case] = &[
    // Compressing selections (T349 per-hunk wins on test-fn renames;
    // T351 zero-context splits hunks finer, so more runs compress).
    Case {
        commit: "7cfa9f0b625d351c83cfc7366664fb60d913f91b",
        rev: None,
        root: false,
        expect_sem: 5,
    },
    Case {
        commit: "b57d0f2c3a77a18326dc2612b0eb8c2e28f229bb",
        rev: None,
        root: false,
        expect_sem: 1,
    },
    Case {
        commit: "12d70c6b4e3d027f22f52316243f05989afd6d55",
        rev: None,
        root: false,
        expect_sem: 4,
    },
    // Ordinary selections (convert + round-trip, zero semantic).
    Case {
        commit: "82f6193843665bfab12b61b64e27070b9a729258",
        rev: None,
        root: false,
        expect_sem: 0,
    },
    Case {
        commit: "beba8b45d149bcbb00eaf78851898763b4c6639b",
        rev: None,
        root: false,
        expect_sem: 0,
    },
    Case {
        commit: "21cb5f0329ef8b2170ad87c01c28d6d592161bdb",
        rev: None,
        root: false,
        expect_sem: 0,
    },
    Case {
        commit: "e06313cd2def54f32c9152edd588c5cda230be59",
        rev: None,
        root: false,
        expect_sem: 0,
    },
    // Root commit via the empty tree (3 new-file sections).
    Case {
        commit: "acd1c1eef9758629edc74dfce2920d760114beb9",
        rev: Some(EMPTY_TREE),
        root: true,
        expect_sem: 0,
    },
];

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

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn toplevel() -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(MANIFEST_DIR)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

fn check_case(wt: &Path, tmp: &Path, case: &Case, failures: &mut Vec<String>) {
    let short = &case.commit[..7];
    if !git_ok(wt, &["checkout", "-qf", case.commit]) {
        failures.push(format!("{short}: checkout failed"));
        return;
    }
    let rev = case
        .rev
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}~1", case.commit));
    let out = Command::new(binary())
        .arg("--repo")
        .arg(wt)
        .arg(&rev)
        .output()
        .unwrap();
    if !out.status.success() {
        failures.push(format!(
            "{short}: diffrs {rev} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
        return;
    }
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    let patch_path = tmp.join(format!("{short}.rsmpl"));
    std::fs::write(&patch_path, &patch).unwrap();
    let sem = patch
        .lines()
        .filter(|l| l.starts_with("@@ -1,1 @@ rule="))
        .count();
    if sem != case.expect_sem {
        failures.push(format!(
            "{short}: semantic hunks {sem} != pinned {}",
            case.expect_sem
        ));
    }
    if case.expect_sem > 0 && !patch.contains("rule=rename_") {
        failures.push(format!("{short}: expected rename compression, got none"));
    }
    // Pre-change tree: the parent commit, or an empty dir for root
    // (creation hunks check against empty).
    let empty = tempfile::tempdir().unwrap();
    let pre: &Path = if case.root {
        empty.path()
    } else {
        if !git_ok(wt, &["checkout", "-qf", &format!("{}~1", case.commit)]) {
            failures.push(format!("{short}: parent checkout failed"));
            return;
        }
        wt
    };
    let checked = Command::new(sibling_applyrs())
        .arg("--file")
        .arg(&patch_path)
        .arg("--check")
        .arg("--repo")
        .arg(pre)
        .output()
        .unwrap();
    if !checked.status.success() {
        failures.push(format!(
            "{short}: applyrs --check exited {}: {}",
            checked.status,
            String::from_utf8_lossy(&checked.stderr)
        ));
    }
}

#[test]
fn history_selection_converts_and_round_trips() {
    let Some(top) = toplevel() else {
        eprintln!("skipping history_e2e: not a git checkout");
        return;
    };
    // Only commits present locally participate (shallow clones skip).
    let present: Vec<&Case> = CASES
        .iter()
        .filter(|c| git_ok(&top, &["cat-file", "-e", c.commit]))
        .collect();
    if present.is_empty() {
        eprintln!("skipping history_e2e: no curated commits present");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let wt = tmp.path().join("wt");
    if !git_ok(
        &top,
        &[
            "worktree",
            "add",
            "--detach",
            &wt.to_string_lossy(),
            present[0].commit,
        ],
    ) {
        eprintln!("skipping history_e2e: worktree add failed");
        return;
    }
    let mut failures: Vec<String> = Vec::new();
    for case in &present {
        check_case(&wt, tmp.path(), case, &mut failures);
    }
    // Cleanup before asserting: no stranded worktree on the green
    // path (a panic leaves stale metadata; `git worktree prune`
    // recovers).
    let _ = Command::new("git")
        .arg("-C")
        .arg(&top)
        .args(["worktree", "remove", "--force"])
        .arg(&wt)
        .output();
    assert!(
        failures.is_empty(),
        "history regressions:\n{}",
        failures.join("\n")
    );
}
