//! WI-1a: S→N→S cycle harness over the full corpus.
//!
//! The N→S→N leg (native → semantic → native, tree equality) is pinned
//! by `fixtures_roundtrip.rs`, `history_e2e.rs`, and
//! `bin/applyrs/tests/fixtures.rs`. This file pins the S→N→S leg
//! (SPEC D.5): apply authored/emitted S to the pre-tree → post;
//! `git diff -U0` pre→post → `diffrs` → S′;
//! `canonicalize_with_tree(S′) == canonicalize_with_tree(S)` under the
//! pre-image for semantic cases (tree-aware: authored sweep and inferred
//! ranged spellings of the same sites compare equal; plain tree equality
//! for ordinary cases); one more iteration asserts the fixed point
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

fn diffrs_staged(repo: &Path, what: &str) -> String {
    // Staged (index-vs-HEAD) mode: renames between a deleted path and an
    // added path surface as `rename from/to` sections (pending mode
    // cannot — the added path is untracked there, invisible to git).
    let out = Command::new(binary())
        .arg("--staged")
        .arg("--repo")
        .arg(repo)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "[{what}] diffrs staged failed: {}",
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
/// `canonicalize_with_tree(S′) == canonicalize_with_tree(S)` under the
/// fixture pre-image. WI-1d re-authored the diet family to canonical
/// ranged diet, so each entry below asserts the fixed point with an
/// already-canonical authored S; WI-1a additionally unifies authored
/// sweep vs inferred ranged spellings of the same sites, and WI-1b
/// unifies authored `# symbol` hunks vs inferred concrete paths.
const CYCLE_OK: &[&str] = &[
    "02-rename-calls",
    "03-move-fn",
    "15-use-rename",
    "16-repeat-fold",
    "17-scope-file",
    "26-macro-sweep",
    "25-symbol-sweep",
    "20-returns-none",
    "23-slot-sweep",
    "28-attrs-sweep",
    "29-impl-sweep",
    "31-delete-items",
    "32-new-file-move",
    "33-move-evolved-composition",
    "34-verbatim-sweep",
    "35-return-sweep",
    "36-verbatim-call-sweep",
    "37-verbatim-unary-sweep",
    "38-method-to-method-sweep",
    "39-move-evolve",
    "27-call-sweep",
    "24-pat-sweep",
    "40-sequence-sweep",
];

/// Divergent cases pinned with one-line reasons (the SPEC D.5
/// exclusion list is the normative review surface; each reason names
/// the family and the owning follow-up). The harness asserts the
/// divergence persists, so a fix must move the entry to CYCLE_OK
/// instead of silently passing.
///
/// Families: `t375` single-site demoted to ordinary by the byte-guard;
/// `nodet` multi-site shape with no detector yet (names the owning WI);
/// `hetero` sites differ structurally so no template fires (WI-4/7);
/// `empty` zero-edit re-diff (05).
const KNOWN_DIVERGENT: &[(&str, &str)] = &[
    (
        "01-rename-fn",
        "t375: single-site fn rename re-infers ordinary",
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
        "hetero: type+let+expr shapes stay ordinary (sequence detector is WI-5c)",
    ),
    (
        "13-sig-evolution",
        "t375: two single-site sig evolutions re-infer ordinary each",
    ),
    (
        "14-item-rename",
        "hetero: 8 renames re-infer as 1 semantic + 7 ordinary (grouping is WI-7)",
    ),
    (
        "18-general-expr",
        "t375: single general-expr re-infers ordinary",
    ),
    (
        "19-trait-rename",
        "t375: single trait method re-infers ordinary",
    ),
    ("21-attrs-preserve", "t375: single fn re-infers ordinary"),
    (
        "22-attrs-strip",
        "t375: single attributed fn re-infers ordinary",
    ),
    (
        "30-trait-sweep",
        "hetero: decl/def trait shapes stay ordinary",
    ),
];

/// Load the fixture pre-image: repo-relative path → file text.
/// Non-UTF-8 files are skipped (semantic compression is UTF-8 by
/// construction; passthrough cycles stay tree-lossless).
fn pre_image_of(repo_dir: &Path) -> std::collections::HashMap<String, String> {
    let mut rels = Vec::new();
    collect_files(repo_dir, repo_dir, &mut rels);
    let mut map = std::collections::HashMap::new();
    for rel in rels {
        if let Ok(text) = std::fs::read_to_string(repo_dir.join(&rel)) {
            map.insert(rel.to_string_lossy().replace('\\', "/"), text);
        }
    }
    map
}

/// Tree-aware canonical form under `repo_dir` (SPEC D.5, WI-1a).
fn tree_canonical_of(patch: &str, repo_dir: &Path, name: &str, which: &str) -> String {
    let pre = pre_image_of(repo_dir);
    rsmpl_engine::canonicalize_with_tree(patch, &pre).unwrap_or_else(|e| {
        panic!("[{name}] canonicalize_with_tree({which}) failed: {e}\ninput:\n{patch}")
    })
}

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
        // Canonical leg (WI-1a tree-aware: both sides compared under
        // the fixture pre-image, so authored sweep and inferred ranged
        // spellings of the same sites compare equal).
        let repo_dir = dir.join("repo");
        let cs = tree_canonical_of(&s, &repo_dir, &name, "S");
        let cs_prime = tree_canonical_of(&s_prime, &repo_dir, &name, "S′");
        if CYCLE_OK.contains(&name.as_str()) {
            if cs != cs_prime {
                failures.push(format!(
                    "[{name}] CYCLE_OK but tree-canonicalize(S′) != tree-canonicalize(S)\n--- S ---\n{cs}\n--- S′ ---\n{cs_prime}"
                ));
            }
        } else if let Some((_, reason)) = KNOWN_DIVERGENT.iter().find(|(n, _)| *n == name) {
            if cs == cs_prime {
                failures.push(format!(
                    "[{name}] FIXED (was: {reason}): tree-canonical forms now equal — move to CYCLE_OK"
                ));
            }
        } else {
            failures.push(format!(
                "[{name}] UNCLASSIFIED canonical divergence (S {}B → S′ {}B)\n--- tree-canonical(S) ---\n{cs}\n--- tree-canonical(S′) ---\n{cs_prime}",
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

#[test]
fn grouped_rename_cycles_fixed_point() {
    // WI-7b: cross-file shared intent (struct definition + call sites,
    // one Old→New delta) raises one shared rule definition-first and
    // cycles losslessly; a file rename with compressible content raises
    // a semantic hunk inside the preserved rename section and cycles.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    // (a) Grouped cross-file rename. The use file sorts first by path,
    // so definition-first reordering is pinned as well.
    let work = tempfile::tempdir().unwrap();
    let w = work.path().join("w");
    std::fs::create_dir_all(w.join("src")).unwrap();
    std::fs::write(w.join("src/z_def.rs"), "struct Old(i32);\n").unwrap();
    std::fs::write(
        w.join("src/a_use.rs"),
        "fn m() {\n    Old(1);\n    Old(2);\n}\n",
    )
    .unwrap();
    git(&w, &["init", "-q"], "grouped");
    commit_all(&w, "grouped");
    std::fs::write(w.join("src/z_def.rs"), "struct New(i32);\n").unwrap();
    std::fs::write(
        w.join("src/a_use.rs"),
        "fn m() {\n    New(1);\n    New(2);\n}\n",
    )
    .unwrap();
    let s = diffrs_pending(&w, "grouped");
    assert!(!s.trim().is_empty(), "grouped change must infer");
    assert_eq!(
        s.matches("rule=rename_Old_to_New").count(),
        2,
        "one shared rule in both files:\n{s}"
    );
    let z = s.find("z_def").expect("def file present");
    let a = s.find("a_use").expect("use file present");
    assert!(z < a, "definition-first file order:\n{s}");
    // S→N leg on a fresh pre-tree, then tree equality.
    let work2 = tempfile::tempdir().unwrap();
    let pre2 = work2.path().join("pre2");
    std::fs::create_dir_all(pre2.join("src")).unwrap();
    std::fs::write(pre2.join("src/z_def.rs"), "struct Old(i32);\n").unwrap();
    std::fs::write(
        pre2.join("src/a_use.rs"),
        "fn m() {\n    Old(1);\n    Old(2);\n}\n",
    )
    .unwrap();
    git(&pre2, &["init", "-q"], "grouped");
    commit_all(&pre2, "grouped");
    apply_patch(&pre2, &s, "grouped");
    let diffs = compare_trees(&pre2, &w);
    assert!(
        diffs.is_empty(),
        "grouped S→N post tree differs: {}",
        diffs.join("; ")
    );
    // Fixed point + canonical equality.
    let s2 = diffrs_pending(&pre2, "grouped");
    assert_eq!(
        s2, s,
        "grouped cycle must be a fixed point:\n--- S ---\n{s}\n--- S2 ---\n{s2}"
    );
    assert_eq!(
        canonical_of(&s, "grouped", "S"),
        canonical_of(&s2, "grouped", "S2")
    );
    // (b) Rename-with-content: a file move whose content hunks compress
    // inside the preserved rename section. Staged mode: git only emits
    // `rename from/to` for tracked (staged) pairs — pending mode leaves
    // the added path untracked and git shows delete + new-file halves.
    let work3 = tempfile::tempdir().unwrap();
    let w3 = work3.path().join("w3");
    std::fs::create_dir_all(w3.join("src")).unwrap();
    let old_body = "fn helper() {\n    1\n}\nfn extra() {\n    2\n}\nfn m() {\n    old_fn(1);\n    old_fn(2);\n    old_fn(3);\n}\n";
    let new_body = "fn helper() {\n    1\n}\nfn extra() {\n    2\n}\nfn m() {\n    new_fn(1);\n    new_fn(2);\n    new_fn(3);\n}\n";
    std::fs::write(w3.join("src/old_tool.rs"), old_body).unwrap();
    git(&w3, &["init", "-q"], "rename-content");
    commit_all(&w3, "rename-content");
    std::fs::remove_file(w3.join("src/old_tool.rs")).unwrap();
    std::fs::write(w3.join("src/new_tool.rs"), new_body).unwrap();
    git(&w3, &["add", "-A"], "rename-content");
    let r = diffrs_staged(&w3, "rename-content");
    assert!(!r.trim().is_empty(), "rename must infer");
    assert!(
        r.contains("rename from"),
        "file move must stay a rename section:\n{r}"
    );
    assert!(
        r.contains("rule="),
        "rename content must compress semantically:\n{r}"
    );
    let work4 = tempfile::tempdir().unwrap();
    let pre4 = work4.path().join("pre4");
    std::fs::create_dir_all(pre4.join("src")).unwrap();
    std::fs::write(pre4.join("src/old_tool.rs"), old_body).unwrap();
    git(&pre4, &["init", "-q"], "rename-content");
    commit_all(&pre4, "rename-content");
    apply_patch(&pre4, &r, "rename-content");
    let diffs = compare_trees(&pre4, &w3);
    assert!(
        diffs.is_empty(),
        "rename-content S→N post tree differs: {}",
        diffs.join("; ")
    );
    // The applyrs `.history/` audit write is tool output, not cycle
    // content — drop it before staging the re-diff tree.
    std::fs::remove_dir_all(pre4.join(".history")).ok();
    git(&pre4, &["add", "-A"], "rename-content");
    let r2 = diffrs_staged(&pre4, "rename-content");
    assert_eq!(
        r2, r,
        "rename-content cycle must be a fixed point:\n--- R ---\n{r}\n--- R2 ---\n{r2}"
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
