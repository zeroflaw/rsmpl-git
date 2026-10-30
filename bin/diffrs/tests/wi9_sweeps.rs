//! WI-9a shape pins for `test-fixtures/diffrs/{27,82}-*`: same-name
//! concrete macro-interior sweep (fires) and the bracket-delimiter
//! veto (stays ordinary). Split out of
//! `fixtures_roundtrip.rs` to hold that file's 2000-line budget
//! (helpers duplicated, not shared — T246 precedent). The generic
//! native → semantic → native round-trip for these fixtures already
//! runs in `fixtures_roundtrip.rs`; this file adds the fire/veto shape
//! pins plus a `--check` gate on the emitted semantic patch.

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

fn normalize_native(native_input: &str) -> String {
    // Mirror infer's three ordinary-passthrough rewrites (title strip +
    // index drop + header collapse) so patch bytes compare directly for
    // ordinary shapes (T434 slice 10).
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

/// Run one fixture through native `git diff -U0` → `diffrs --staged` →
/// `applyrs --check`, returning `(native_input, semantic)`.
fn convert_fixture(name: &str) -> (String, String) {
    let dir = workspace_root()
        .join("test-fixtures")
        .join("diffrs")
        .join(name);
    assert!(dir.is_dir(), "missing fixture dir for {name}");
    let work = tempfile::tempdir().unwrap();
    copy_tree(&dir.join("repo"), work.path());
    git_ok(work.path(), &["init", "-q"], name);
    git_ok(work.path(), &["add", "-A"], name);
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
        name,
    );
    copy_tree(&dir.join("expected"), work.path());
    git_ok(work.path(), &["add", "-A"], name);
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
    let out = Command::new(binary())
        .arg("--repo")
        .arg(work.path())
        .arg("--staged")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "[{name}] diffrs failed");
    let semantic = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!semantic.trim().is_empty(), "[{name}] diffrs is empty");
    git_ok(work.path(), &["stash", "-q"], name);
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
    (native_input, semantic)
}

#[test]
fn wi9_macro_interior_fires() {
    // Fixture 27 (was `27-macro-invocation-veto`, the T450 stays-
    // ordinary pin; WI-9a flips it): two identical
    // `assert_eq!(.., TEXT_SECTION)` → `TEXT_STRIPPED` sites plus a pure
    // addition join to one concrete `# allow=many` hunk; the tail stays
    // an ordinary insertion.
    let (native_input, semantic) = convert_fixture("27-macro-interior-fires");
    assert!(
        semantic.contains("rule=r1"),
        "macro interior must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# macro"),
        "plain `!` routing, no directive, got: {semantic}"
    );
    assert!(
        semantic.contains("- assert_eq!(out.rsmpl, TEXT_SECTION)\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ assert_eq!(out.rsmpl, TEXT_STRIPPED)\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    assert_eq!(out.rsmpl, TEXT_SECTION);"),
        "interior minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.contains("+#[test]"),
        "pure-addition tail must stay, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "interior sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9_macro_interior_bracket_stays_ordinary() {
    // Fixture 82: `my_vec![1, 2]` → `my_vec![1, 3]` over two sites —
    // the engine's interior-change splice normalizes to `!(...)`
    // (probed corrupt), so the paren-only gate keeps it ordinary.
    let (native_input, semantic) = convert_fixture("82-macro-interior-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "bracket interior must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    my_vec![1, 2];"),
        "minus lines must be preserved, got: {semantic}"
    );
    assert!(
        semantic.contains("+    my_vec![1, 3];"),
        "plus lines must be preserved, got: {semantic}"
    );
    let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
    assert!(context.is_empty(), "zero-context, got: {context:?}");
    assert!(
        semantic == normalize_native(&native_input),
        "ordinary passthrough must equal normalized native, got: {semantic}"
    );
}
