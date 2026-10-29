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

fn normalize_native(native_input: &str) -> String {
    // T434 slice 10: mirror infer's three ordinary-passthrough rewrites
    // (title strip + index drop + header collapse) so patch bytes compare
    // directly for ordinary shapes.
    let mut sections: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut seen = false;
    for line in native_input.lines() {
        if line.starts_with("diff --git ") {
            if seen {
                sections.push(std::mem::take(&mut current));
            }
            seen = true;
        }
        if seen {
            current.push_str(line);
            current.push('\n');
        }
    }
    if seen {
        sections.push(current);
    }
    let mut out = String::new();
    for sec in &sections {
        let collapsible = sec
            .lines()
            .any(|l| l.starts_with("--- ") || l.starts_with("+++ "));
        for line in sec.lines() {
            if line.starts_with("index ") {
                continue;
            }
            if collapsible && line.starts_with("diff --git ") {
                out.push_str(&rsmpl_patch::collapse_git_header(line));
                out.push('\n');
                continue;
            }
            if let Some(rest) = line.strip_prefix("@@") {
                if let Some(end) = rest.find("@@") {
                    let ranges = &rest[..end];
                    if ranges.contains('-') && ranges.contains('+') {
                        out.push_str("@@");
                        out.push_str(ranges);
                        out.push_str("@@\n");
                        continue;
                    }
                }
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out
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

        // T477: stage the post-state so new-file additions ride the index:
        // unstaged `git diff` never shows untracked files (and plain
        // `git stash` leaves them behind for --check/--output), while
        // `git diff --cached` + `diffrs --staged` cover them.
        git_ok(work.path(), &["add", "-A"], &name);

        // 2. Native input: zero-context staged git diff, `index` lines
        // stripped (diffrs never emits them, T374 — strip for a fair byte
        // count). `--cached` (T477) so new-file additions are included —
        // unstaged `git diff` never shows untracked files.
        let native = git(
            work.path(),
            &[
                "diff",
                "--cached",
                "--unified=0",
                "--no-color",
                "--no-ext-diff",
            ],
        );
        assert!(native.status.success(), "[{name}] git diff failed");
        let native_input: String = String::from_utf8_lossy(&native.stdout)
            .lines()
            .filter(|l| !l.starts_with("index "))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(!native_input.is_empty(), "[{name}] fixture has no change");

        // 3. Semantic compression, through the index (T477): after the
        // `add -A` the worktree==index, so `--staged` sees exactly the
        // post-state including new files (T392: staged scope skips the
        // untracked path, but nothing is untracked anymore).
        let out = Command::new(binary())
            .arg("--repo")
            .arg(work.path())
            .arg("--staged")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "[{name}] diffrs failed");
        let semantic = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(!semantic.trim().is_empty(), "[{name}] diffrs is empty");

        // 4. Back to the pre-change tree: semantic must check clean.
        // `stash` restores HEAD including staged new files (T477), so
        // pastes lower against the true pre-state (missing-file path).
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
            // T434 slice 10: ordinary passthrough is exactly normalized
            // native (the three infer rewrites), so patch bytes compare
            // directly — not just end-states.
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "03-deleted-file-move" {
            assert!(
                semantic.contains("rule=r1"),
                "deleted-file move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
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
                semantic.contains("rule=r1"),
                "struct move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
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
        } else if name == "05-move-trailing-leftover" {
            assert!(
                semantic.contains("rule=r1"),
                "trailing-leftover move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "trailing-leftover move must paste, got: {semantic}"
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
                semantic.contains("+// trailing"),
                "trailing leftover must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "06-move-prefix-leftover" {
            assert!(
                semantic.contains("rule=r1"),
                "prefix-leftover move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "prefix-leftover move must paste, got: {semantic}"
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
                semantic.contains("-fn gone() {"),
                "prefix leftover must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "07-move-fanout" {
            assert!(
                semantic.contains("rule=r1"),
                "fan-out move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "fan-out move must paste, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r3"),
                "fan-out second paste must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("+ M1\n").count(),
                2,
                "fan-out pastes must share the bind, got: {semantic}"
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
        } else if name == "08-move-eof-cap" {
            assert!(
                semantic.contains("rule=r1"),
                "eof-cap move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "eof-cap move must paste, got: {semantic}"
            );
            assert!(
                !semantic.contains("rule=r3"),
                "second pair must cap, got: {semantic}"
            );
            assert!(
                !semantic.contains("rule=r4"),
                "second pair must cap, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn one("),
                "paired lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn one("),
                "paired lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("-fn two() {"),
                "capped run must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("+fn two() {"),
                "capped run must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "09-move-dedup" {
            assert!(
                semantic.contains("rule=r1"),
                "dedup move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "dedup move must paste, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r1\n").count(),
                1,
                "first claim wins, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r2\n").count(),
                1,
                "single paste, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("+ M1\n").count(),
                1,
                "single paste shares the bind once, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r3"),
                "losing twin minus compresses (T472 EOF relaxation), got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn big("),
                "losing twin minus lines must be consumed, got: {semantic}"
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
        } else if name == "10-move-same-hunk-fanout" {
            assert!(
                semantic.contains("rule=r1"),
                "same-hunk fan-out move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "same-hunk fan-out move must paste, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r3"),
                "same-hunk fan-out second paste must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("+ M1\n").count(),
                2,
                "both same-hunk pastes share the bind, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r").count(),
                3,
                "one cut + two pastes, got: {semantic}"
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
        } else if name == "11-move-same-file-reorder" {
            assert!(
                semantic.contains("rule=r1"),
                "same-file reorder must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "same-file reorder must paste, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("+ M1\n").count(),
                1,
                "single paste shares the bind once, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn aaa("),
                "moved lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn aaa("),
                "moved lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("fn bbb("),
                "untouched middle item stays out of sem, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "12-move-interior-blanks" {
            assert!(
                semantic.contains("rule=r1"),
                "interior-blank move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r1"),
                "interior-blank second item must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "interior-blank move must paste, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "interior-blank second paste must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn big("),
                "moved lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn big("),
                "moved lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn spaced("),
                "interior-blank lines must ride the cut, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn spaced("),
                "interior-blank lines must ride the paste, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "13-slot-sweep" {
            assert!(
                semantic.contains("rule=r1"),
                "slot sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- V Q fn old_function_name G (...) W {"),
                "slot sweep minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ V Q fn new_function_name G (...) W {"),
                "slot sweep plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn old_function_name("),
                "renamed lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn new_function_name("),
                "renamed lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "14-method-to-function" {
            assert!(
                semantic.contains("rule=r1"),
                "m2f sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- Receiver.old_method(E0)"),
                "m2f sweep minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ new_function(Receiver, E0)"),
                "m2f sweep plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    a.old_method(1);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    new_function(a, 1);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "15-method-to-method" {
            assert!(
                semantic.contains("rule=r1"),
                "m2m sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- Receiver.old_method(E0)"),
                "m2m sweep minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ Receiver.new_method(E0)"),
                "m2m sweep plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    a.old_method(1);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    a.new_method(1);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "16-use-prefix" {
            assert!(
                semantic.contains("rule=r1"),
                "use-prefix sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- use crate::old"),
                "use-prefix minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ use crate::new"),
                "use-prefix plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-use crate::old::Thing;"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+use crate::new::Thing;"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "17-same-literal" {
            assert!(
                semantic.contains("rule=r1"),
                "same-literal sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- Receiver.contains(\"skills/rsmpl-git/SKILL.md\")"),
                "same-literal minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ Receiver.contains(\"skills/gitrs/SKILL.md\")"),
                "same-literal plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    agents.contains(\"skills/rsmpl-git/SKILL.md\");"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    agents.contains(\"skills/gitrs/SKILL.md\");"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "18-ref-strip" {
            assert!(
                semantic.contains("rule=r1"),
                "ref-strip sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- &Receiver.join(\"repo\")"),
                "ref-strip minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ Receiver"),
                "ref-strip plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    copy_tree(&a.join(\"repo\"), x);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    copy_tree(a, x);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "19-verbatim" {
            assert!(
                semantic.contains("rule=r1"),
                "verbatim sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- foo(&state.parsed)"),
                "verbatim minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ foo(parsed)"),
                "verbatim plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    foo(&state.parsed);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    foo(parsed);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "20-let-sweep" {
            assert!(
                semantic.contains("rule=r1"),
                "let-statement verbatim sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- let out = run(dir.path(), &[\"--patch\", patch]);"),
                "let minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ let out = run_stdin(dir.path(), patch, &[]);"),
                "let plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    let out = run(dir.path(), &[\"--patch\", patch]);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    let out = run_stdin(dir.path(), patch, &[]);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "21-call-peel" {
            assert!(
                semantic.contains("rule=r1"),
                "peel sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- run(D, &[\"--patch\", M])"),
                "j=0 peel minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ run_stdin(D, M, &[])"),
                "j=0 peel plus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- run(D, &[\"--patch\", M, T0])"),
                "j=1 peel minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ run_stdin(D, M, &[T0])"),
                "j=1 peel plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    let a = run(dir, &[\"--patch\", patch]);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    let d = run_stdin(dir, patch, &[\"--check\"]);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains(
                    "-    let f = run(dir, &[\"--patch\", patch, \"--format\", \"json\"]);"
                ),
                "singleton j=2 site must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "22-verbatim-stranded" {
            assert!(
                semantic.contains("@@ -2,19 @@ rule=r1"),
                "stranded verbatim sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- validated(&text)"),
                "stranded minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ validated_live(&text, &pre)"),
                "stranded plus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("@@ -20,0 +21,3 @@"),
                "stranded leftover insertion must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    validated(&text);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    validated_live(&text, &pre);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "23-macro-veto" {
            // T445: both probed veto shapes stay ordinary — the
            // format!-interior sites (33a8cbb miniature, engine-blind
            // macro bodies) and the env!-in-init lets (e06313c
            // miniature, verbatim_init_ok macro gate). No rule= lines;
            // passthrough is exactly normalized native.
            assert!(
                !semantic.contains("rule="),
                "macro-gate shapes must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("-        header = rsmpl_patch::collapse_git_header(&format!(\"diff --git a/{path} b/{path}\"))"),
                "macro-interior minus lines must be preserved, got: {semantic}"
            );
            assert!(
                semantic.contains("+        header = super::git_header(path)"),
                "macro-interior plus lines must be preserved, got: {semantic}"
            );
            assert!(
                semantic.contains(
                    "-    let out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));"
                ),
                "macro-init minus lines must be preserved, got: {semantic}"
            );
            assert!(
                semantic.contains(
                    "+    let out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));"
                ),
                "macro-init plus lines must be preserved, got: {semantic}"
            );
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "24-call-arg-add" {
            // T447: homo trailing addition compresses (a.rs), hetero
            // trailing (b.rs, 09d3446 miniature) stays ordinary.
            assert!(
                semantic.contains("rule=r1"),
                "arg-add sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- make_widget(E0, E1)"),
                "arg-add minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ make_widget(E0, E1, false)"),
                "arg-add plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    make_widget(alpha, 1);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    make_widget(alpha, 1, false);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("-    make_widget(theta, 4);"),
                "hetero-trailing minus lines must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("+    make_widget(theta, 4, symlink);"),
                "hetero-trailing plus lines must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "25-call-literal" {
            // T448: homo trailing literal delta compresses (a.rs, the
            // 7025a14 prize miniature); hetero trailing (b.rs) stays
            // ordinary without vetoing via tuple-keyed totality.
            assert!(
                semantic.contains("rule=r1"),
                "literal sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- make_call(E0, E1, false)"),
                "literal minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ make_call(E0, E1, \"100644\")"),
                "literal plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    make_call(\"alpha\", first_count, false);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    make_call(\"alpha\", first_count, \"100644\");"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("-    make_call(\"gamma\", third_count, true);"),
                "hetero-trailing minus lines must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("+    make_call(\"delta\", fourth_count, symlink);"),
                "hetero-trailing plus lines must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "26-call-literal-cross-hunk" {
            // T449: spaced `let` sites split across singleton `-U0`
            // hunks (a pure addition vetoes whole-section balance, the
            // 7025a14 shape) group into one literal sweep via the
            // cross-hunk section driver; the addition stays ordinary
            // in place, and hetero-trailing b.rs stays ordinary.
            assert!(
                semantic.contains("rule=r1"),
                "literal cross-hunk sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- make_call(E0, E1, false)"),
                "literal minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ make_call(E0, E1, \"100644\")"),
                "literal plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    let a = make_call(\"alpha\", first_count, false);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    let a = make_call(\"alpha\", first_count, \"100644\");"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("+    let cached = extra();"),
                "pure-addition leftover must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("-    let c = make_call(\"gamma\", third_count, true);"),
                "hetero-trailing minus lines must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("+    let d = make_call(\"delta\", fourth_count, symlink);"),
                "hetero-trailing plus lines must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "27-macro-invocation-veto" {
            // T450: top-level macro invocations stay ordinary even as a
            // clean same-callee same-arity literal-delta pair (68f092f
            // miniature: 2x assert_eq!(out.rsmpl, TEXT_SECTION) ->
            // TEXT_STRIPPED in singleton hunks + a pure addition forcing
            // the unbalanced section). Expr::Macro never reaches
            // call_parts and verbatim rejects macros > 0 (engine-blind
            // macro gate, SPEC 21/22, T445 family) — the literal
            // cross-hunk driver skips the unparseable pairs, no rule=
            // lines, passthrough is exactly normalized native.
            assert!(
                !semantic.contains("rule="),
                "macro-invocation pairs must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("-    assert_eq!(out.rsmpl, TEXT_SECTION);"),
                "macro-invocation minus lines must be preserved, got: {semantic}"
            );
            assert!(
                semantic.contains("+    assert_eq!(out.rsmpl, TEXT_STRIPPED);"),
                "macro-invocation plus lines must be preserved, got: {semantic}"
            );
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "28-verbatim-return" {
            // T452: return-shape verbatim (b9c00d5/T418 miniature) — two
            // identical `return make_widget("a", 1);` lines sweep via the
            // engine Expr::Return arm (bare-call sweeps decline: `return`
            // is not a bare call).
            assert!(
                semantic.contains("rule=r1"),
                "return verbatim sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- return make_widget(\"a\", 1)"),
                "return minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ return make_widget(\"a\", 2)"),
                "return plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    return make_widget(\"a\", 1);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    return make_widget(\"a\", 2);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "29-verbatim-struct-field" {
            // T452: struct-field verbatim (c48a6e6/T416 miniature) — two
            // identical `text: make_text(sub),` field lines sweep on the
            // stripped inner call (labels match, engine drops the label).
            assert!(
                semantic.contains("rule=r1"),
                "struct-field verbatim sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- make_text(sub)"),
                "struct-field minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ make_text_new(sub)"),
                "struct-field plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-        text: make_text(sub),"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+        text: make_text_new(sub),"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "30-verbatim-unary" {
            // T452: unary verbatim (b208847/T415 miniature) — two bare
            // `!ready(flag);` lines sweep via leading-`!` Raw bypass and
            // the structural Unary match (bare-call sweeps decline: the
            // top-level node is Unary, not Call).
            assert!(
                semantic.contains("rule=r1"),
                "unary verbatim sweep must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- !ready(flag)"),
                "unary minus pattern must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("+ !ready_done(flag)"),
                "unary plus pattern must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    !ready(flag);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+    !ready_done(flag);"),
                "rewritten lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "31-delete-whole-file" {
            // T473: whole-file item deletion (e062dda/T464 miniature) —
            // the deleted file's items sweep into one scope=file
            // delete_items hunk; module-doc lines ride ordinary leftovers.
            assert!(
                semantic.contains("rule=r1"),
                "whole-file delete sweep must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r1").count(),
                1,
                "exactly one delete hunk, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn old_"),
                "deleted item lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("-//! Old helpers"),
                "non-item doc lines ride ordinary leftovers, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "32-partial-delete" {
            // T473: partial-file item deletion (7945da1/T465 prize-shape
            // miniature) — one minus-heavy unbalanced hunk (whole helper
            // items out, `mod common;` + `use common::*;` in) compresses
            // to one real-range delete_items hunk plus an ordinary
            // insertion leftover with the compensating trailing blank.
            assert!(
                semantic.contains("rule=r1"),
                "partial delete sweep must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r1").count(),
                1,
                "exactly one delete hunk, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn binary"),
                "deleted item lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn init_repo"),
                "deleted item lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("+mod common;"),
                "plus lines ride the insertion leftover, got: {semantic}"
            );
            assert!(
                semantic.contains("+use common::*;"),
                "plus lines ride the insertion leftover, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "33-nested-partial-delete" {
            // T474: nested partial-file item deletion (b0da138/T466
            // prize-shape miniature) -- two mod-tests fns out, minus
            // hunk starting with a separator blank the engine never
            // absorbs: one shrunk-range delete_items hunk plus an
            // ordinary minus-blank leftover ahead of it.
            assert!(
                semantic.contains("rule=r1"),
                "nested partial delete sweep must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r1").count(),
                1,
                "exactly one delete hunk, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    fn first"),
                "deleted item lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    fn second"),
                "deleted item lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("\n-\n"),
                "leading blank rides an ordinary leftover, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "34-eof-touch-delete" {
            // T474: EOF-touching item deletion (3baecc1/a2fc9b3/T472
            // prize-shape miniature) -- trailing helper out through EOF,
            // separator blank absorbed inside the minus range: one
            // real-range delete_items hunk, no ordinary leftover.
            assert!(
                semantic.contains("rule=r1"),
                "EOF-touch delete sweep must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r1").count(),
                1,
                "exactly one delete hunk, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("\n@@ ").count(),
                1,
                "no ordinary leftover hunks, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn stale_helper"),
                "deleted item lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "35-impl-method-partial-delete" {
            // T475: impl-method partial-file item deletion (d426215/T466
            // prize-shape miniature) -- doc'd `lowercase_warnings`
            // method out of `impl Catalog`, minus hunk starting with a
            // separator blank the engine never absorbs: one shrunk-range
            // delete_items hunk plus an ordinary minus-blank leftover
            // ahead of it. First dedicated cover for the
            // collect_impl_method_spans path (33 covers mod-nested fns).
            assert!(
                semantic.contains("rule=r1"),
                "impl-method partial delete sweep must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r1").count(),
                1,
                "exactly one delete hunk, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    pub fn lowercase_warnings"),
                "deleted item lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    /// Warn (not error)"),
                "deleted doc lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("\n-\n"),
                "leading blank rides an ordinary leftover, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "36-eof-touch-kept-blank-veto" {
            // T475: EOF-touch kept-preceding-blank veto (T472 veto-side
            // miniature) -- same repo shape as 34 but the separator blank
            // is kept, so the minus hunk covers only the fn lines and the
            // preceding kept blank vetoes the sweep: no rule=delete_items,
            // ordinary hunk only, cycle still byte-identical.
            assert!(
                !semantic.contains("rule=r3"),
                "kept preceding blank must veto the sweep, got: {semantic}"
            );
            assert!(
                semantic.contains("-fn stale_helper"),
                "vetoed lines stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "ordinary must still beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "37-move-evolved-composition" {
            // T476: move + partial-delete composition (c649b34/T471
            // prize-shape miniature) -- one verbatim fn moves cross-file
            // (cut + paste) while two in-transit-evolved fns (`super::`
            // -> `crate::`) ride one real-range delete_items hunk plus
            // an ordinary addition leftover. First full-cycle cover for
            // sweep interaction (09 covers the same-file dedup-loser).
            assert!(
                semantic.contains("rule=r1"),
                "verbatim move cut must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "verbatim move paste must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=r1").count(),
                1,
                "exactly one delete hunk, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn sweep_a"),
                "evolved minus lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("-    super::helper();"),
                "evolved minus body must be consumed, got: {semantic}"
            );
            assert!(
                semantic.contains("+    crate::helper();"),
                "evolved plus lines ride the ordinary leftover, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "composition must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "38-new-file-move" {
            // T477: verbatim cross-file move to a NEW file (the shape the
            // old unstaged harness could not host: untracked `new.rs`
            // never appeared in `git diff`, and plain `stash` left it
            // behind for --check/--output). Staged collection covers it:
            // one `move_parse_pair` cut + one `paste_parse_pair` new-file
            // paste, every minus/plus line consumed, no ordinary leftover.
            assert!(
                semantic.contains("rule=r1"),
                "new-file move cut must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "new-file move paste must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("rule=r3"),
                "verbatim move needs no delete sweep, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("\n@@ ").count(),
                2,
                "no ordinary leftover hunks, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn parse_pair"),
                "moved minus lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn parse_pair"),
                "moved plus lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "new-file move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "39-tiny-move-stays-ordinary" {
            // WI-1e diet (no `# item I`): the 3-line move now wins T375
            // (byte-guard flip pinned, same as the unit tiny-move pin).
            assert!(
                semantic.contains("rule=r1"),
                "diet tiny move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "diet tiny move must paste, got: {semantic}"
            );
        } else if name == "40-tiny-move-fires" {
            // T480: the same 3-line shape with a longer body fires —
            // emitted cut+paste is ~constant (the item rides a
            // metavar, never respelled) while replaced ordinary grows
            // with content, so the byte guard clears: no fixed line
            // threshold. First sub-4-line firing-move cover.
            assert!(
                semantic.contains("rule=r1"),
                "tiny move cut must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r2"),
                "tiny move paste must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn m() {"),
                "moved minus lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn m() {"),
                "moved plus lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "tiny firing move must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "41-single-rename-stays-ordinary" {
            // WI-1e diet terse (`- fn foo` + `# sweep`, no lang): the
            // short single now wins T375 (byte-guard flip pinned).
            assert!(
                semantic.contains("rule=r1"),
                "diet short single must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("- fn foo\n"),
                "diet terse minus must fire, got: {semantic}"
            );
        } else if name == "42-single-rename-fires" {
            // T481: the same single-rename shape with a long signature
            // fires -- the `fn Old(...)` / `fn New(...)` pattern lines
            // are far shorter than the ordinary lines, so the byte
            // guard clears (unit-test `fn_rename_compresses` shape,
            // first full-cycle cover).
            assert!(
                semantic.contains("rule=r1"),
                "long single rename must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn foo("),
                "renamed minus lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn bar("),
                "renamed plus lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "single rename must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "43-single-use-stays-ordinary" {
            // T482: single use-prefix site veto full-cycle miniature --
            // one `use crate::old::Thing;` -> `new::Thing` line costs more
            // as a semantic hunk (rule + scope overhead) than the one
            // ordinary hunk it replaces, so no rule= hunks: pure ordinary
            // passthrough, still shorter via 0-context + header collapse.
            // Veto side of fixture 16 (3-site use_old_to_new fire).
            assert!(
                !semantic.contains("rule="),
                "single use-prefix edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "44-single-slot-fires" {
            // T482: single slot-sweep site with a long signature fires --
            // the `fn Old(...)` / `fn New(...)` slot pattern lines are far
            // shorter than the ordinary lines, so the byte guard clears
            // (13-shape minus one site + keep wrapper; first single-site
            // slot-sweep full-cycle cover, mirrors 42 for the slot family).
            assert!(
                semantic.contains("rule=r1"),
                "single slot rename must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn old_function_name("),
                "renamed minus lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn new_function_name("),
                "renamed plus lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "slot rename must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "45-single-m2f-stays-ordinary" {
            // T483: single method-to-function site veto full-cycle miniature --
            // one `a.old_method(1);` -> `new_function(a, 1);` line costs more
            // as a semantic hunk than the one ordinary hunk it replaces, so
            // no rule= hunks: pure ordinary passthrough. Veto side of
            // fixture 14 (3-site calls_old_method_to_new_function fire).
            assert!(
                !semantic.contains("rule="),
                "single m2f edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "46-use-divergent-tail-stays-ordinary" {
            // T483: divergent-tail use-prefix veto full-cycle miniature --
            // `use crate::old::A/B;` -> `new::A`/`new::C` keeps per-site tails
            // byte-unequal on the second site, so the N>=2 prefix sweep
            // declines (unit gate use_prefix_sweep_divergent_tail_...). No
            // rule= hunks: pure ordinary passthrough. Veto side of fixture
            // 16 (3-site use_old_to_new fire); hetero-across-sites shapes
            // still fire, only per-site divergence vetoes.
            assert!(
                !semantic.contains("rule="),
                "divergent-tail use-prefix edit must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("-use crate::old::B;"),
                "divergent-tail minus lines must be preserved, got: {semantic}"
            );
            assert!(
                semantic.contains("+use crate::new::C;"),
                "divergent-tail plus lines must be preserved, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "47-single-m2m-stays-ordinary" {
            // T484: single method-to-method site veto miniature --
            // one `a.old_method(1);` -> `a.new_method(1);` line
            // costs more as a semantic hunk than the ordinary hunk
            // it replaces: pure ordinary passthrough (code gate
            // `minuses.len() < 2` in the m2m sweep, veto side of
            // fixture 15, the 4-site old_method_to_new_method fire).
            assert!(
                !semantic.contains("rule="),
                "single m2m edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "48-single-call-stays-ordinary" {
            // T484: single bare-call rename veto miniature --
            // one `foo(1);` -> `bar(1);` line costs more as a
            // semantic hunk than the ordinary hunk it replaces:
            // pure ordinary passthrough (unit gate
            // `single_call_site_stays_passthrough`, veto side of
            // fixture 01, the 3-site foo_to_bar fire).
            assert!(
                !semantic.contains("rule="),
                "single call edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "49-single-same-literal-stays-ordinary" {
            // T485: single same-literal site veto miniature --
            // one `agents.contains("skills/rsmpl-git/SKILL.md");` ->
            // `agents.contains("skills/gitrs/SKILL.md");` costs more
            // as a semantic hunk than the ordinary hunk it replaces:
            // pure ordinary passthrough (unit gate
            // `method_same_literal_single_site_stays_passthrough`,
            // veto side of fixture 17, the 3-site calls_contains_literal fire).
            assert!(
                !semantic.contains("rule="),
                "single same-literal edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "50-single-ref-strip-stays-ordinary" {
            // T485: single ref-strip site veto miniature --
            // one `copy_tree(&a.join("repo"), x);` -> `copy_tree(a, x);`
            // costs more as a semantic hunk than the ordinary hunk it
            // replaces: pure ordinary passthrough (unit gate
            // `ref_strip_sweep_single_site_stays_passthrough`, veto side
            // of fixture 18, the 2-site calls_join_strip fire).
            assert!(
                !semantic.contains("rule="),
                "single ref-strip edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "51-single-call-arg-add-stays-ordinary" {
            // T486: single call-arg-add site veto miniature --
            // one `make_widget(alpha, 1);` -> `make_widget(alpha, 1, false);`
            // costs more as a semantic hunk than the ordinary hunk it
            // replaces: pure ordinary passthrough (veto side of
            // fixture 24, the multi-site arg-add fire).
            assert!(
                !semantic.contains("rule="),
                "single call-arg-add edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "52-single-call-literal-stays-ordinary" {
            // T486: single call-literal site veto miniature --
            // one `make_call("alpha", first_count, false);` ->
            // `make_call("alpha", first_count, "100644");` costs more
            // as a semantic hunk than the ordinary hunk it replaces:
            // pure ordinary passthrough (veto side of fixture 25,
            // the multi-site literal fire).
            assert!(
                !semantic.contains("rule="),
                "single call-literal edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "53-single-let-stays-ordinary" {
            // T487: single let-sweep site veto miniature --
            // one `let out = run(dir.path(), &["--patch", patch]);` ->
            // `let out = run_stdin(dir.path(), patch, &[]);` costs more
            // as a semantic hunk than the ordinary hunk it replaces:
            // pure ordinary passthrough (veto side of fixture 20,
            // the 2-site let fire).
            assert!(
                !semantic.contains("rule="),
                "single let edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "54-single-call-peel-stays-ordinary" {
            // T487: single call-peel site veto miniature --
            // one `let a = run(dir, &["--patch", patch]);` ->
            // `let a = run_stdin(dir, patch, &[]);` costs more as a
            // semantic hunk than the ordinary hunk it replaces: pure
            // ordinary passthrough (veto side of fixture 21, the
            // multi-site peel fire).
            assert!(
                !semantic.contains("rule="),
                "single call-peel edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "55-slot-hetero-fires" {
            // T488: hetero slot-sweep fire miniature -- two long-signature
            // fns renamed DIFFERENTLY fire one per-site single rule each
            // (homo multi-site shapes share one rule as in fixture 13;
            // hetero deltas cannot share, so diffrs emits per-site singles,
            // each clearing the T375 byte guard on its own).
            assert!(
                semantic.contains("rule=r1"),
                "hetero slot first site must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=r1"),
                "hetero slot second site must fire, got: {semantic}"
            );
            assert!(
                !semantic.contains("-fn old_function_name("),
                "renamed minus lines must be consumed, got: {semantic}"
            );
            assert!(
                !semantic.contains("+fn brand_new_name("),
                "renamed plus lines must be consumed, got: {semantic}"
            );
            assert!(
                semantic.len() < native_input.len(),
                "hetero slot sweep must beat native bytes ({} vs {})",
                semantic.len(),
                native_input.len()
            );
        } else if name == "56-single-let-call-literal-stays-ordinary" {
            // T488: single let-bound call-literal site veto miniature --
            // one `let a = make_call("alpha", first_count, false);` ->
            // `let a = make_call("alpha", first_count, "100644");` costs
            // more as a semantic hunk than the ordinary hunk it replaces:
            // pure ordinary passthrough (veto side of fixture 26, the
            // let-bound cross-hunk fire; fixture 52 pins the same veto for
            // the bare-statement shape of fixture 25).
            assert!(
                !semantic.contains("rule="),
                "single let-bound call-literal edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "57-single-verbatim-stays-ordinary" {
            // T489: single verbatim site veto miniature --
            // one `foo(&state.parsed);` -> `foo(parsed);` costs more
            // as a semantic hunk than the ordinary hunk it replaces:
            // pure ordinary passthrough (N>=2 gate in the verbatim
            // identical-line sweep, veto side of fixture 19, the
            // multi-site verbatim fire).
            assert!(
                !semantic.contains("rule="),
                "single verbatim edit must stay ordinary, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "58-hetero-m2m-stays-ordinary" {
            // T489: hetero-delta m2m veto miniature --
            // `a.old_method(1);` -> `a.new_method(1);` plus
            // `b.old_method(2);` -> `b.brand_new(2);`: different new
            // idents share no (old, new) class, so the m2m sweep
            // declines with no per-site singles (call sweeps need one
            // shared delta -- the contrast to fixture 55, where hetero
            // slot deltas fire one single rule per site). Veto side of
            // fixture 15 (homo multi-site m2m fire).
            assert!(
                !semantic.contains("rule="),
                "hetero-delta m2m edit must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("-    a.old_method(1);"),
                "hetero minus lines must be preserved, got: {semantic}"
            );
            assert!(
                semantic.contains("+    b.brand_new(2);"),
                "hetero plus lines must be preserved, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "59-hetero-m2f-stays-ordinary" {
            // T490: hetero-delta m2f veto miniature --
            // `a.old_method(1);` -> `new_function(a, 1);` plus
            // `b.old_method(2);` -> `brand_new(b, 2);`: different new
            // callees share no (old, new) class, so the m2f sweep
            // declines with no per-site singles (call sweeps need one
            // shared delta -- the contrast to fixture 55, where hetero
            // slot deltas fire one single rule per site). Veto side of
            // fixture 14 (homo multi-site m2f fire).
            assert!(
                !semantic.contains("rule="),
                "hetero-delta m2f edit must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("-    a.old_method(1);"),
                "hetero minus lines must be preserved, got: {semantic}"
            );
            assert!(
                semantic.contains("+    brand_new(b, 2);"),
                "hetero plus lines must be preserved, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        } else if name == "60-hetero-samelit-stays-ordinary" {
            // T490: hetero-delta same-literal veto miniature --
            // two identical `agents.contains("skills/rsmpl-git/...");`
            // sites rewrite to DIFFERENT new literals (`gitrs` /
            // `other`): no shared (old, new) literal pair, so the
            // same-literal sweep declines with no per-site singles
            // (same shared-delta gate as fixtures 58/59 -- the contrast
            // to fixture 55, where hetero slot deltas fire per-site
            // singles). Veto side of fixture 17 (homo multi-site
            // same-literal fire).
            assert!(
                !semantic.contains("rule="),
                "hetero-delta same-literal edit must stay ordinary, got: {semantic}"
            );
            assert!(
                semantic.contains("-    agents.contains(\"skills/rsmpl-git/SKILL.md\");"),
                "hetero minus lines must be preserved, got: {semantic}"
            );
            assert!(
                semantic.contains("+    agents.contains(\"skills/other/SKILL.md\");"),
                "hetero plus lines must be preserved, got: {semantic}"
            );
            let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
            assert!(context.is_empty(), "zero-context, got: {context:?}");
            let normalized = normalize_native(&native_input);
            assert!(
                semantic == normalized,
                "[{name}] ordinary passthrough must equal normalized native"
            );
        }
    }
}
