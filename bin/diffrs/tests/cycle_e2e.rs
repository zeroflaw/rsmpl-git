//! WI-1c: S→N→S cycle harness over the full corpus.
//!
//! The N→S→N leg (native → semantic → native, tree equality) is pinned
//! by `fixtures_roundtrip.rs`, `history_e2e.rs`, and
//! `bin/applyrs/tests/fixtures.rs`. This file pins the S→N→S leg
//! (SPEC D.5): apply authored/emitted S to the pre-tree → post;
//! `git diff -U0` pre→post → `diffrs` → S′; `canonicalize(S′) ==
//! canonicalize(S)` for semantic cases (plain tree equality for
//! ordinary cases); one more iteration asserts the fixed point
//! `S″ == S′` byte-identical.
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

fn git(dir: &Path, args: &[&str], what: &str) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
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
        let ft = entry.file_type().unwrap();
        if ft.is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else if ft.is_symlink() {
            #[cfg(unix)]
            {
                let link = std::fs::read_link(entry.path()).unwrap();
                let _ = std::fs::remove_file(&target);
                std::os::unix::fs::symlink(link, &target).unwrap();
            }
            #[cfg(not(unix))]
            {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
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

/// Byte-compare two materialized trees; tool-internal `.history/` and
/// `.git` are ignored. Returns one line per mismatch (empty = identical).
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

fn apply_patch(repo: &Path, patch: &str, what: &str) {
    let out = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(patch)
        .arg("--repo")
        .arg(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "[{what}] applyrs failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn diffrs_pending(repo: &Path, what: &str) -> String {
    // Pending (unstaged) mode: the applyrs `.history/` audit file is
    // untracked, so the source layer skips it with a warning and the
    // patch stays comparable.
    let out = Command::new(binary())
        .arg("--repo")
        .arg(repo)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "[{what}] diffrs pending failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn commit_all(dir: &Path, what: &str) {
    git(dir, &["add", "-A"], what);
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
        what,
    );
}

/// Semantic cases whose re-inference is canonically lossless today:
/// `canonicalize(S′) == canonicalize(S)`. Empty for the applyrs
/// corpus: every authored fixture predates canonical diet (sweep
/// dummies, long decls, illustrative ranges) — WI-1d re-authors them
/// here as they are fixed.
const CYCLE_OK: &[&str] = &[];

/// Divergent cases pinned with one-line reasons (WI-1d exclusion seed:
/// re-author to canonical ranged diet, fix the detector, or move to
/// the D.5 exclusion list). The harness asserts the divergence
/// persists, so a fix must move the entry instead of silently passing.
///
/// Families: `t375` single-site demoted to ordinary by the byte-guard;
/// `diet` same sites/template, authored long/sweep form vs canonical
/// ranged diet (mechanical WI-1d re-author); `nodet` multi-site shape
/// with no detector yet (names the owning WI); `hetero` sites differ
/// structurally so no template fires; `empty` zero-edit re-diff.
const KNOWN_DIVERGENT: &[(&str, &str)] = &[
    (
        "01-rename-fn",
        "t375: single-site fn rename re-infers ordinary",
    ),
    (
        "02-rename-calls",
        "diet: authored sweep+# lang+# expr E vs ranged diet E0",
    ),
    (
        "03-move-fn",
        "diet: authored # lang+Helper+# item I+context hunk vs M1/inferred/minimal",
    ),
    ("04-mixed", "t375: two single-site edits re-infer ordinary"),
    (
        "05-optional-noop",
        "empty: zero-match optional leaves an empty re-diff",
    ),
    (
        "06-insert-use",
        "t375: single insertion anchor re-infers ordinary",
    ),
    (
        "08-symbol-fallback",
        "t375: single qualified call re-infers ordinary (qualified detector is WI-3)",
    ),
    (
        "09-token-tree",
        "t375: single macro invocation re-infers ordinary (macro detector is WI-3)",
    ),
    (
        "10-impl-rename",
        "t375: single impl method re-infers ordinary",
    ),
    (
        "11-method-to-function",
        "t375: single m2f re-infers ordinary",
    ),
    (
        "12-stmt-rewrite",
        "hetero: type+let+expr shapes stay ordinary (sequence detector is WI-4/9)",
    ),
    (
        "13-sig-evolution",
        "t375: two single-site sig evolutions re-infer ordinary each",
    ),
    (
        "14-item-rename",
        "hetero: 8 renames re-infer as 1 semantic + 6 ordinary (grouping is WI-7)",
    ),
    (
        "15-use-rename",
        "diet: authored range 1,7 vs site-exact 1,5",
    ),
    (
        "16-repeat-fold",
        "nodet: nested Box::new stays ordinary (no general repeat-fold detector)",
    ),
    (
        "17-scope-file",
        "diet: authored sweep+# expr E vs ranged diet E0",
    ),
    (
        "18-general-expr",
        "t375: single general-expr re-infers ordinary",
    ),
    (
        "19-trait-rename",
        "t375: single trait method re-infers ordinary",
    ),
    (
        "20-returns-none",
        "diet: authored plain rename+# returns=none vs inferred allow=many slot sweep",
    ),
    ("21-attrs-preserve", "t375: single fn re-infers ordinary"),
    (
        "22-attrs-strip",
        "t375: single attributed fn re-infers ordinary",
    ),
    (
        "23-slot-sweep",
        "diet: authored sweep vs ranged diet slot sweep",
    ),
    (
        "24-pat-sweep",
        "hetero: divergent let shapes stay ordinary (anti-unification is WI-4)",
    ),
    (
        "25-symbol-sweep",
        "nodet: qualified-path call sweep vetoed, stays ordinary (WI-3)",
    ),
    (
        "26-macro-sweep",
        "nodet: macro rename stays ordinary (macro-rename detector is WI-3)",
    ),
    (
        "27-call-sweep",
        "hetero: two-calls-per-line sites stay ordinary",
    ),
    (
        "28-attrs-sweep",
        "diet: authored sweep vs ranged diet slot sweep",
    ),
    (
        "29-impl-sweep",
        "diet: authored concrete impl-method vs inferred VQGW slot sweep",
    ),
    (
        "30-trait-sweep",
        "hetero: decl/def trait shapes stay ordinary",
    ),
    (
        "31-delete-items",
        "diet: authored declared # item I vs inferred I",
    ),
    (
        "32-new-file-move",
        "diet: authored declared # item I vs inferred I",
    ),
    (
        "33-move-evolved-composition",
        "diet: authored declared # item I (x2) vs inferred I",
    ),
    (
        "34-verbatim-sweep",
        "diet: authored sweep vs ranged diet verbatim sweep",
    ),
    (
        "35-return-sweep",
        "diet: authored sweep vs ranged diet return sweep",
    ),
    (
        "36-verbatim-call-sweep",
        "diet: authored sweep vs ranged diet verbatim sweep",
    ),
    (
        "37-verbatim-unary-sweep",
        "diet: authored sweep vs ranged diet unary sweep",
    ),
    (
        "38-method-to-method-sweep",
        "diet: authored sweep+declared metavars vs ranged diet inferred",
    ),
];

fn canonical_of(patch: &str, name: &str, which: &str) -> String {
    rsmpl_patch::canonicalize(patch)
        .unwrap_or_else(|e| panic!("[{name}] canonicalize({which}) failed: {e}\ninput:\n{patch}"))
}

#[test]
fn applyrs_corpus_sns_cycle() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let fixtures = workspace_root().join("test-fixtures").join("applyrs");
    let mut names: Vec<String> = std::fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no applyrs fixtures found");
    let mut failures: Vec<String> = Vec::new();
    let mut covered = 0usize;

    for name in names {
        let dir = fixtures.join(&name);
        if !dir.is_dir() || dir.join("expect_fail.txt").exists() {
            continue;
        }
        covered += 1;
        // (a) Apply authored S to the pre-tree → post; post must equal
        // `expected/` (mirrors bin/applyrs/tests/fixtures.rs).
        let work = tempfile::tempdir().unwrap();
        let pre = work.path().join("pre");
        std::fs::create_dir_all(&pre).unwrap();
        copy_tree(&dir.join("repo"), &pre);
        git(&pre, &["init", "-q"], &name);
        commit_all(&pre, &name);
        let s = std::fs::read_to_string(dir.join("patch.rsmpl")).unwrap();
        apply_patch(&pre, &s, &name);
        let diffs = compare_trees(&pre, &dir.join("expected"));
        if !diffs.is_empty() {
            failures.push(format!(
                "[{name}] S→N post tree differs from expected/: {}",
                diffs.join("; ")
            ));
            continue;
        }
        // (b) Re-diff pre→post through pending worktree state → S′.
        let s_prime = diffrs_pending(&pre, &name);
        if s_prime.trim().is_empty() {
            // Zero-edit shapes (e.g. 05-optional-noop): nothing to
            // re-diff. The empty leg is the pin — not a failure.
            if KNOWN_DIVERGENT.iter().any(|(n, _)| *n == name) {
                continue;
            }
            failures.push(format!("[{name}] UNCLASSIFIED: empty S′ (zero-edit?)"));
            continue;
        }
        // Tree leg of the inferred patch: S′ must rebuild `expected/`.
        let work2 = tempfile::tempdir().unwrap();
        let pre2 = work2.path().join("pre2");
        std::fs::create_dir_all(&pre2).unwrap();
        copy_tree(&dir.join("repo"), &pre2);
        git(&pre2, &["init", "-q"], &name);
        commit_all(&pre2, &name);
        apply_patch(&pre2, &s_prime, &name);
        let diffs2 = compare_trees(&pre2, &dir.join("expected"));
        if !diffs2.is_empty() {
            failures.push(format!(
                "[{name}] S′→N post tree differs from expected/: {}",
                diffs2.join("; ")
            ));
            continue;
        }
        // Canonical leg.
        let cs = canonical_of(&s, &name, "S");
        let cs_prime = canonical_of(&s_prime, &name, "S′");
        if CYCLE_OK.contains(&name.as_str()) {
            if cs != cs_prime {
                failures.push(format!(
                    "[{name}] CYCLE_OK but canonicalize(S′) != canonicalize(S)\n--- S ---\n{cs}\n--- S′ ---\n{cs_prime}"
                ));
            }
        } else if let Some((_, reason)) = KNOWN_DIVERGENT.iter().find(|(n, _)| *n == name) {
            if cs == cs_prime {
                failures.push(format!(
                    "[{name}] FIXED (was: {reason}): canonical forms now equal — move to CYCLE_OK"
                ));
            }
        } else {
            failures.push(format!(
                "[{name}] UNCLASSIFIED canonical divergence (S {}B → S′ {}B)\n--- canonical(S) ---\n{cs}\n--- canonical(S′) ---\n{cs_prime}",
                s.len(),
                s_prime.len()
            ));
        }
        // (c) Fixed point: S′ applied to a fresh pre-tree re-diffs to
        // byte-identical S″.
        let work3 = tempfile::tempdir().unwrap();
        let pre3 = work3.path().join("pre3");
        std::fs::create_dir_all(&pre3).unwrap();
        copy_tree(&dir.join("repo"), &pre3);
        git(&pre3, &["init", "-q"], &name);
        commit_all(&pre3, &name);
        apply_patch(&pre3, &s_prime, &name);
        let s_double_prime = diffrs_pending(&pre3, &name);
        if s_double_prime != s_prime {
            failures.push(format!(
                "[{name}] fixed point broken: S″ != S′ ({}B vs {}B)\n--- S′ ---\n{s_prime}\n--- S″ ---\n{s_double_prime}",
                s_prime.len(),
                s_double_prime.len()
            ));
        }
    }
    assert!(covered > 0, "no appliable applyrs fixtures found");
    assert!(
        failures.is_empty(),
        "S→N→S regressions ({} cases, {} failures):\n{}",
        covered,
        failures.len(),
        failures.join("\n")
    );
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

#[test]
fn diffrs_corpus_sns_fixed_point() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let fixtures = workspace_root().join("test-fixtures").join("diffrs");
    let mut names: Vec<String> = std::fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no diffrs fixtures found");
    let mut failures: Vec<String> = Vec::new();

    // S: pending recipe (worktree post-state vs index pre-state, no
    // staging). Same-mode cycles only: pending appends untracked
    // new-file sections after tracked ones while staged/rev flows are
    // git-sorted, so a staged S vs pending S₂ would differ in section
    // order for create+modify pairs (probed 38-new-file-move). D.5
    // byte-identity is defined on canonical patches; raw fixed points
    // must compare within one mode.
    for name in names {
        let dir = fixtures.join(&name);
        if !dir.is_dir() {
            continue;
        }
        let work = tempfile::tempdir().unwrap();
        let w = work.path().join("w");
        std::fs::create_dir_all(&w).unwrap();
        copy_tree(&dir.join("repo"), &w);
        git(&w, &["init", "-q"], &name);
        commit_all(&w, &name);
        copy_tree(&dir.join("expected"), &w);
        sync_expected_deletions(&w, &dir.join("repo"), &dir.join("expected"));
        let s = diffrs_pending(&w, &name);
        if s.trim().is_empty() {
            failures.push(format!("[{name}] diffrs emitted empty S"));
            continue;
        }
        // S→N leg: fresh pre + applyrs S must rebuild `expected/`.
        // The pre-state is committed: re-diffing needs a HEAD.
        let work2 = tempfile::tempdir().unwrap();
        let pre2 = work2.path().join("pre2");
        std::fs::create_dir_all(&pre2).unwrap();
        copy_tree(&dir.join("repo"), &pre2);
        git(&pre2, &["init", "-q"], &name);
        commit_all(&pre2, &name);
        apply_patch(&pre2, &s, &name);
        let diffs = compare_trees(&pre2, &dir.join("expected"));
        if !diffs.is_empty() {
            failures.push(format!(
                "[{name}] S→N post tree differs from expected/: {}",
                diffs.join("; ")
            ));
            continue;
        }
        // Fixed point: pending re-diff of the applied tree must
        // reproduce S byte-identical (deterministic emission), hence
        // also canonically.
        let s2 = diffrs_pending(&pre2, &name);
        if s2 != s {
            failures.push(format!(
                "[{name}] fixed point broken: S₂ != S ({}B vs {}B)\n--- S ---\n{s}\n--- S₂ ---\n{s2}",
                s.len(),
                s2.len()
            ));
            continue;
        }
        let cs = canonical_of(&s, &name, "S");
        let cs2 = canonical_of(&s2, &name, "S₂");
        if cs != cs2 {
            failures.push(format!("[{name}] canonical fixed point broken"));
        }
    }
    assert!(
        failures.is_empty(),
        "diffrs-corpus fixed-point regressions:\n{}",
        failures.join("\n")
    );
}

#[test]
fn bench_corpus_sns_fixed_point() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let root = workspace_root();
    let mut failures: Vec<String> = Vec::new();

    // applyrs benches: repo/ + patch.rsmpl, no expected/ — fixed point
    // plus canonical self-equality only.
    let apply_benches = root.join("example-benches").join("applyrs");
    let mut anames: Vec<String> = std::fs::read_dir(&apply_benches)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    anames.sort();
    assert!(!anames.is_empty(), "no applyrs benches found");
    for name in anames {
        let dir = apply_benches.join(&name);
        if !dir.is_dir() {
            continue;
        }
        let label = format!("applyrs/{name}");
        let work = tempfile::tempdir().unwrap();
        let w = work.path().join("w");
        std::fs::create_dir_all(&w).unwrap();
        copy_tree(&dir.join("repo"), &w);
        git(&w, &["init", "-q"], &label);
        commit_all(&w, &label);
        let s = std::fs::read_to_string(dir.join("patch.rsmpl")).unwrap();
        apply_patch(&w, &s, &label);
        let s_prime = diffrs_pending(&w, &label);
        if s_prime.trim().is_empty() {
            failures.push(format!("[{label}] empty S′ after applying bench patch"));
            continue;
        }
        let work2 = tempfile::tempdir().unwrap();
        let w2 = work2.path().join("w2");
        std::fs::create_dir_all(&w2).unwrap();
        copy_tree(&dir.join("repo"), &w2);
        git(&w2, &["init", "-q"], &label);
        commit_all(&w2, &label);
        apply_patch(&w2, &s_prime, &label);
        let s_double_prime = diffrs_pending(&w2, &label);
        if s_double_prime != s_prime {
            failures.push(format!(
                "[{label}] fixed point broken: S″ != S′ ({}B vs {}B)",
                s_prime.len(),
                s_double_prime.len()
            ));
        }
        let cs = canonical_of(&s_prime, &label, "S′");
        let cs2 = canonical_of(&s_double_prime, &label, "S″");
        if cs != cs2 {
            failures.push(format!("[{label}] canonical fixed point broken"));
        }
    }

    // diffrs benches: committed sem.patch applied to repo/ must rebuild
    // after/, and re-diff must reproduce sem.patch byte-identical.
    let diff_benches = root.join("example-benches").join("diffrs");
    let mut dnames: Vec<String> = std::fs::read_dir(&diff_benches)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    dnames.sort();
    assert!(!dnames.is_empty(), "no diffrs benches found");
    for name in dnames {
        let dir = diff_benches.join(&name);
        if !dir.is_dir() {
            continue;
        }
        let label = format!("diffrs/{name}");
        let sem = std::fs::read_to_string(dir.join("sem.patch")).unwrap();
        let work = tempfile::tempdir().unwrap();
        let w = work.path().join("w");
        std::fs::create_dir_all(&w).unwrap();
        copy_tree(&dir.join("repo"), &w);
        git(&w, &["init", "-q"], &label);
        commit_all(&w, &label);
        apply_patch(&w, &sem, &label);
        let diffs = compare_trees(&w, &dir.join("after"));
        if !diffs.is_empty() {
            failures.push(format!(
                "[{label}] sem.patch→N post tree differs from after/: {}",
                diffs.join("; ")
            ));
            continue;
        }
        let s2 = diffrs_pending(&w, &label);
        if s2 != sem {
            failures.push(format!(
                "[{label}] fixed point broken: re-diff != sem.patch ({}B vs {}B)\n--- sem.patch ---\n{sem}\n--- re-diff ---\n{s2}",
                sem.len(),
                s2.len()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bench-corpus fixed-point regressions:\n{}",
        failures.join("\n")
    );
}
