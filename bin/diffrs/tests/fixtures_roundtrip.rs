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
        } else if name == "05-move-trailing-leftover" {
            assert!(
                semantic.contains("rule=move_big"),
                "trailing-leftover move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big"),
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
                semantic.contains("rule=move_big"),
                "prefix-leftover move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big"),
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
                semantic.contains("rule=move_big"),
                "fan-out move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big"),
                "fan-out move must paste, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big_2"),
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
                semantic.contains("rule=move_one"),
                "eof-cap move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_one"),
                "eof-cap move must paste, got: {semantic}"
            );
            assert!(
                !semantic.contains("rule=move_two"),
                "second pair must cap, got: {semantic}"
            );
            assert!(
                !semantic.contains("rule=paste_two"),
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
                semantic.contains("rule=move_big"),
                "dedup move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big"),
                "dedup move must paste, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=move_big\n").count(),
                1,
                "first claim wins, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=paste_big\n").count(),
                1,
                "single paste, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("+ M1\n").count(),
                1,
                "single paste shares the bind once, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=delete_items"),
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
                semantic.contains("rule=move_big"),
                "same-hunk fan-out move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big"),
                "same-hunk fan-out move must paste, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big_2"),
                "same-hunk fan-out second paste must fire, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("+ M1\n").count(),
                2,
                "both same-hunk pastes share the bind, got: {semantic}"
            );
            assert_eq!(
                semantic.matches("rule=paste_big").count(),
                2,
                "exactly two pastes in one hunk, got: {semantic}"
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
                semantic.contains("rule=move_aaa"),
                "same-file reorder must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_aaa"),
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
                semantic.contains("rule=move_big"),
                "interior-blank move must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=move_spaced"),
                "interior-blank second item must fire, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_big"),
                "interior-blank move must paste, got: {semantic}"
            );
            assert!(
                semantic.contains("rule=paste_spaced"),
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
                semantic.contains("rule=rename_old_function_name_to_new_function_name"),
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
                semantic.contains("rule=calls_old_method_to_new_function"),
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
                semantic.contains("rule=calls_old_method_to_new_method"),
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
                semantic.contains("rule=use_old_to_new"),
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
                semantic.contains("rule=calls_contains_literal"),
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
                semantic.contains("rule=calls_join_strip"),
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
                semantic.contains("rule=verbatim"),
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
                semantic.contains("rule=verbatim"),
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
                semantic.contains("rule=calls_run_to_run_stdin_peel"),
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
                semantic.contains("@@ -1,1 @@ rule=verbatim"),
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
                semantic.contains("rule=calls_make_widget_add_1"),
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
                semantic.contains("rule=calls_make_call_literal"),
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
                semantic.contains("rule=calls_make_call_literal"),
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
                semantic.contains("rule=verbatim"),
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
                semantic.contains("rule=verbatim"),
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
                semantic.contains("rule=verbatim"),
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
        }
    }
}

#[test]
fn example_bench_readme_table_matches_recomputed() {
    // T435 (T328-equivalent for diffrs): pin committed native.patch /
    // sem.patch freshness + bench-table numbers against rot. Recompute
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
        let native = git(
            tmp.path(),
            &["diff", "--unified=0", "--no-color", "--no-ext-diff"],
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
