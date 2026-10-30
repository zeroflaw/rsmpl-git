//! Shared harness for the split round-trip tests (WI-9-pre).
//!
//! `fixtures_roundtrip.rs` (part 1, fixtures `01`–`44`) and
//! `fixtures_roundtrip_part2.rs` (part 2, `45` and up) share this module:
//! the helpers plus the six-step native → semantic → native loop
//! (`run_all`), with per-part shape pins supplied as a callback. The part
//! boundary (`PART_BOUNDARY`) is numeric on the fixture prefix so future
//! `87+` fixtures land in part 2, which owns all future growth; part 1's
//! `01`–`44` range is closed and never gains arms. Split precedent:
//! WI-4-prerequisite chain split, WI-6a directives split.

use std::path::{Path, PathBuf};
use std::process::Command;

/// First fixture number owned by part 2 (`45-single-m2f-...` and up).
/// Numeric (not lexicographic) so a future `100-...` fixture still lands
/// in part 2 instead of sorting before `"45"`.
pub const PART_BOUNDARY: u32 = 45;

/// Numeric fixture prefix (`"27-macro-..."` → 27). Unparsable names map
/// to 0 (part 1); every real fixture parses, so 0 never fires.
pub fn fixture_num(name: &str) -> u32 {
    name.split('-').next().unwrap_or("").parse().unwrap_or(0)
}

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

pub fn normalize_native(native_input: &str) -> String {
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

/// The six-step native → semantic → native loop, moved verbatim from the
/// pre-split test: per fixture, build the pre-state repo, stage the
/// post-state, take the zero-context staged diff, compress with diffrs,
/// run the per-part shape `pins`, then check + lower with applyrs and
/// byte-compare a fresh `git apply` against `expected/`.
pub fn run_all(label: &str, filter: impl Fn(&str) -> bool, pins: impl Fn(&str, &str, &str)) {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let fixtures = workspace_root().join("test-fixtures").join("diffrs");
    let mut names: Vec<String> = std::fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no diffrs fixtures found");

    let mut ran = 0u32;
    for name in names {
        let dir = fixtures.join(&name);
        if !dir.is_dir() {
            continue;
        }
        if !filter(&name) {
            continue;
        }
        ran += 1;

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

        // 7. Per-fixture shape pins (owned by the calling part).
        pins(&name, &native_input, &semantic);
    }
    assert!(
        ran > 0,
        "[{label}] part ran zero fixtures — boundary rot? check PART_BOUNDARY"
    );
}
