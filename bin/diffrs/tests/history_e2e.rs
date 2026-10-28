//! T346: history-per-commit regression generator.
//!
//! Converts a curated selection of this repository's own commits
//! with the real `diffrs` binary (the T346 recipe: `checkout c` +
//! `diffrs c~1`, empty-tree rev for the root) and round-trips each
//! captured patch through the sibling `applyrs --check` against an
//! isolated pre-change tree. A failing conversion fails this test —
//! that is the pin mechanism: reproduce with the recipe, then extend
//! the table.
//! `--check` is followed by a real apply byte-compared against the
//! child tree (T361: sweeps can over-match invisibly to `--check`).
//!
//! The table pins exact semantic-hunk counts, so both inference
//! improvements (more compression) and regressions (conversion
//! failures, lost compression) show up here. Harness notes: one
//! scratch `git worktree` under a `TempDir` for the `diffrs` side
//! (the real checkout is never touched); the `applyrs --check` side
//! materializes the parent via `git archive c~1 | tar -x -C <tmp>`
//! (T356 isolation — no second checkout, no `.history/` pollution in
//! the diffrs worktree); root checks against an empty dir (the
//! empty-tree object is ensured via `git mktree` first — fresh
//! clones may lack it); skips — never hard errors — outside a
//! checkout, when `worktree add` fails, or when a curated commit
//! is absent (shallow clones; in a full checkout a missing pin
//! fails as stale recipe, T362); `applyrs` always runs via
//! `--file` because the large selections exceed the shell
//! inline-arg limit.

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
    // Compressing selections (T359 `&`-receiver strip; T375 guard
    // demoted the T341/T349 single-rename rows below to zero semantic
    // — their hunks cost more than the ordinary lines — but the rows
    // stay as convert + round-trip regression coverage).
    Case {
        commit: "7cfa9f0b625d351c83cfc7366664fb60d913f91b",
        rev: None,
        root: false,
        expect_sem: 0,
    },
    Case {
        commit: "b57d0f2c3a77a18326dc2612b0eb8c2e28f229bb",
        rev: None,
        root: false,
        // T375: the lone long-name test-fn rename now stays ordinary
        // (its semantic hunk cost +160 B over the ordinary hunk), so
        // this row pins zero semantic — still converting + round-trip.
        expect_sem: 0,
    },
    Case {
        commit: "12d70c6b4e3d027f22f52316243f05989afd6d55",
        rev: None,
        root: false,
        // T375: four long-name test-fn renames, each semantic hunk
        // costlier than its ordinary hunk — zero semantic by design.
        expect_sem: 0,
    },
    // T359 `&`-receiver strip: 5 files of `&R.join("")` -> `R` sites
    // (bare arg-position lines + `write(...)`-nested) compress to 5 hunks.
    Case {
        commit: "57140fe11a54a337b0d2bb9d9a5a7c1a5c06412a",
        rev: None,
        root: false,
        expect_sem: 5,
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
    // Symlink create (T121) and symlink delete (T298: the T353
    // `wrong type` dogfood gap) — ordinary passthrough, zero semantic.
    Case {
        commit: "e80988aa125e685f87e9c433320769f96295d2d7",
        rev: None,
        root: false,
        expect_sem: 0,
    },
    Case {
        commit: "53372c43752e2ad1523fe2688525429f47fca7fe",
        rev: None,
        root: false,
        expect_sem: 0,
    },
    // T403 verbatim sweep pins: 50d4a49 (`&state.parsed,` x11 -> `parsed,`)
    // and 3e0b9bc (`cur.push(c)` x2 -> `push_str_byte(&mut cur, c)`) compress
    // to one literal hunk each; 712ad9d (extra `&cand.body_text` guard
    // occurrence), 33a8cbb (macro interior), a2fc9b3 (`?` over closure) and
    // 05de4d1 (`let` statements) stay ordinary by design — totality and
    // shape gates with live corpus evidence.
    Case {
        commit: "50d4a498d3e88c3d4f70c0e809106af993bd8cdf",
        rev: None,
        root: false,
        expect_sem: 1,
    },
    Case {
        commit: "3e0b9bcccc2eabdb66f2fc4bf4d9fc8930dd4084",
        rev: None,
        root: false,
        expect_sem: 1,
    },
    Case {
        commit: "c48a6e60e286137a092b1ffb10084396440da6ee",
        rev: None,
        root: false,
        // T416 field-label sweep: two identical `text: f(&sub)` struct
        // fields sweep on the inner call (label preserved by the engine).
        expect_sem: 1,
    },
    Case {
        commit: "b9c00d537adf49568b957837307208821ab4ccd0",
        rev: None,
        root: false,
        // T418 return-shape sweep: two identical `return f(...)` lines
        // thread a param (engine `Expr::Return` arm; T412 gate lifted).
        expect_sem: 1,
    },
    Case {
        commit: "b20884733ba61b1e6630adfb8920cede2cd751b7",
        rev: None,
        root: false,
        // T415 unary routing: five identical `!opaque_hit_lines(...)`
        // pairs compress to one verbatim hunk (leading-`!` bypasses to
        // `Raw`; the engine matches `Unary` structurally).
        expect_sem: 1,
    },
    Case {
        commit: "712ad9d27c0fbc0a2ca92f483594c635e2fa49a9",
        rev: None,
        root: false,
        // T403 totality veto: an unchanged `&cand.body_text` guard
        // subterm would be rewritten by a sweep — stays ordinary.
        expect_sem: 0,
    },
    Case {
        commit: "33a8cbbe7d3936411f96895437b5eac5e1478918",
        rev: None,
        root: false,
        // T403 macro gate: `&format!(...)` interior patterns are
        // engine-illegal (SPEC 21) — stays ordinary.
        expect_sem: 0,
    },
    Case {
        commit: "a2fc9b3966312b60439e56e40030f8a3dc88d92a",
        rev: None,
        root: false,
        // T403 Try-over-closure gate: the combination matches
        // nothing (probe-bisected) — stays ordinary.
        expect_sem: 0,
    },
    Case {
        commit: "05de4d15bdbe2e44ea2628c8d990ad2a2512e841",
        rev: None,
        root: false,
        // T403 let gate: hetero-arg `let` sweeps need engine
        // arg-restructuring (T404) — stays ordinary.
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

fn is_shallow(top: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(top)
        .args(["rev-parse", "--is-shallow-repository"])
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
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

/// Materialize `parent` (`c~1`) into `dest` via
/// `git -C <top> archive <parent>` piped to `tar -x -C <dest>`
/// (no shell). Returns true on success; preserves symlinks and mode
/// bits exactly as tracked. Caller creates `dest`.
fn materialize_parent(top: &Path, parent: &str, dest: &Path) -> bool {
    let archive = Command::new("git")
        .arg("-C")
        .arg(top)
        .args(["archive", parent])
        .output();
    let archive = match archive {
        Ok(o) if o.status.success() => o.stdout,
        _ => return false,
    };
    let mut tar = match Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(dest)
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    use std::io::Write;
    let wrote = tar
        .stdin
        .as_mut()
        .map(|s| s.write_all(&archive).is_ok())
        .unwrap_or(false);
    if !wrote {
        let _ = tar.kill();
        return false;
    }
    matches!(tar.wait(), Ok(st) if st.success())
}

/// Byte-compare two materialized trees (T361): `--check` cannot see
/// sweep over-match (extra sites still match; a `;;` side is valid
/// Rust), so the harness real-applies each captured patch and
/// compares the result against the child tree. Tool-internal paths
/// (top-level `.history/`, defensive `.git`) are ignored; modes are
/// not compared (the engine promises no exec-bit preservation).
/// Returns one line per mismatch (empty = identical).
fn compare_trees(applied: &Path, expect: &Path) -> Vec<String> {
    fn walk(root: &Path, out: &mut Vec<PathBuf>) {
        let mut stack = vec![PathBuf::new()];
        while let Some(rel) = stack.pop() {
            let entries = match std::fs::read_dir(root.join(&rel)) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let rel_path = rel.join(entry.file_name());
                if rel_path
                    .components()
                    .next()
                    .is_some_and(|c| c.as_os_str() == ".history" || c.as_os_str() == ".git")
                {
                    continue;
                }
                let ft = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if ft.is_dir() {
                    stack.push(rel_path.clone());
                }
                out.push(rel_path);
            }
        }
    }
    fn kind_of(p: &Path) -> Option<&'static str> {
        let md = std::fs::symlink_metadata(p).ok()?;
        let ft = md.file_type();
        Some(if ft.is_symlink() {
            "symlink"
        } else if ft.is_dir() {
            "dir"
        } else {
            "file"
        })
    }
    let mut a_paths = Vec::new();
    let mut e_paths = Vec::new();
    walk(applied, &mut a_paths);
    walk(expect, &mut e_paths);
    a_paths.sort();
    e_paths.sort();
    let mut both: Vec<PathBuf> = a_paths;
    both.extend(e_paths);
    both.sort();
    both.dedup();
    let mut diffs = Vec::new();
    for rel in &both {
        let a = applied.join(rel);
        let e = expect.join(rel);
        match (kind_of(&a), kind_of(&e)) {
            (None, None) => {}
            (None, _) => diffs.push(format!("missing in applied: {}", rel.display())),
            (_, None) => diffs.push(format!("extra in applied: {}", rel.display())),
            (Some(x), Some(y)) if x != y => {
                diffs.push(format!("kind differs ({x} vs {y}): {}", rel.display()))
            }
            (Some("dir"), _) => {}
            (Some("symlink"), _) => {
                if std::fs::read_link(&a).ok() != std::fs::read_link(&e).ok() {
                    diffs.push(format!("symlink target differs: {}", rel.display()));
                }
            }
            _ => {
                if std::fs::read(&a).ok() != std::fs::read(&e).ok() {
                    diffs.push(format!("content differs: {}", rel.display()));
                }
            }
        }
    }
    diffs
}

fn check_case(wt: &Path, top: &Path, tmp: &Path, case: &Case, failures: &mut Vec<String>) {
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
    // Anchored count (T358): semantic headers are exactly
    // `@@ -1,1 @@ rule=` (scope=file dummies); ordinary hunks are
    // two-sided `@@ -a,b +c,d @@`, and SPEC/test content lines may
    // contain `rule=` substrings — substring counts inflate (~170/251
    // false "compressing"), so never loosen this prefix.
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
    if case.expect_sem > 0
        && !(patch.contains("rule=rename_")
            || patch.contains("rule=calls_")
            || patch.contains("rule=verbatim"))
    {
        failures.push(format!(
            "{short}: expected rename/sweep compression, got none"
        ));
    }
    // Pre-change tree in isolation (T356): the parent commit
    // materialized via `git archive parent | tar -x -C <tmp>`, or an
    // empty dir for root (creation hunks check against empty). The
    // shared diffrs worktree is never re-checked-out here, so the
    // `applyrs --check` `.history/` audit write lands in the throwaway
    // pre-image instead of polluting the next `diffrs` conversion.
    let empty = tempfile::tempdir().unwrap();
    let staged = tempfile::tempdir().unwrap();
    let pre: &Path = if case.root {
        empty.path()
    } else {
        let parent = format!("{}~1", case.commit);
        if !materialize_parent(top, &parent, staged.path()) {
            failures.push(format!("{short}: git archive {parent} failed"));
            return;
        }
        staged.path()
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
    // Apply-and-compare (T361): `--check` is blind to sweep
    // over-match (extra sites still match; a `;;` rewrite is valid
    // Rust), so real-apply onto the throwaway pre-image and
    // byte-compare against the child tree. `.history/` (apply audit)
    // is ignored by the comparator.
    let applied = Command::new(sibling_applyrs())
        .arg("--file")
        .arg(&patch_path)
        .arg("--repo")
        .arg(pre)
        .output()
        .unwrap();
    if !applied.status.success() {
        failures.push(format!(
            "{short}: applyrs apply exited {}: {}",
            applied.status,
            String::from_utf8_lossy(&applied.stderr)
        ));
        return;
    }
    let expect_tmp = tempfile::tempdir().unwrap();
    if !materialize_parent(top, case.commit, expect_tmp.path()) {
        failures.push(format!("{short}: git archive {} failed", case.commit));
        return;
    }
    let diffs = compare_trees(pre, expect_tmp.path());
    if !diffs.is_empty() {
        let mut msg = format!("{short}: applied tree differs from {}:", case.commit);
        for d in diffs.iter().take(5) {
            msg.push_str(&format!("\n  {d}"));
        }
        if diffs.len() > 5 {
            msg.push_str(&format!("... and {} more", diffs.len() - 5));
        }
        failures.push(msg);
    }
}

#[test]
fn history_selection_converts_and_round_trips() {
    let Some(top) = toplevel() else {
        eprintln!("skipping history_e2e: not a git checkout");
        return;
    };
    // Only commits present locally participate (shallow clones skip);
    // in a full checkout a missing curated pin is stale-recipe rot,
    // not a skip — fail loudly instead of shrinking coverage (T362).
    let mut failures: Vec<String> = Vec::new();
    let (present, missing): (Vec<&Case>, Vec<&Case>) = CASES
        .iter()
        .partition(|c| git_ok(&top, &["cat-file", "-e", c.commit]));
    if !missing.is_empty() && !is_shallow(&top) {
        failures.push(format!(
            "stale history pins (absent in full checkout): {}",
            missing
                .iter()
                .map(|c| c.commit[..7].to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    if present.is_empty() {
        eprintln!("skipping history_e2e: no curated commits present");
        assert!(
            failures.is_empty(),
            "history regressions:\n{}",
            failures.join("\n")
        );
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
    // The root row converts against the well-known empty tree, which
    // git creates lazily (a fresh clone may lack the object and the
    // conversion would fail with `unknown revision`). `mktree` is
    // idempotent and deterministic — ensure it once, best-effort (a
    // real gap still fails loudly on the root row itself).
    let _ = Command::new("git")
        .arg("-C")
        .arg(&top)
        .args(["mktree"])
        .stdin(std::process::Stdio::null())
        .output();
    for case in &present {
        check_case(&wt, &top, tmp.path(), case, &mut failures);
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
