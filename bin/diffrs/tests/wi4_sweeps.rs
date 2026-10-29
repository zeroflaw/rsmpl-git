//! WI-4a shape pins for `test-fixtures/diffrs/6{9,70}-*`: let
//! type-addition sweeps (`let P = E` → `let P: T = E`, hetero P/E
//! across sites, uniform concrete T). Split out of
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
fn wi4a_let_type_add_fires() {
    // Fixture 69: three `let PAT = INIT;` sites with hetero patterns
    // and inits gain the uniform `: u32` ascription in one
    // `# allow=many` hunk with inferred `P0`/`E0` holes.
    let (native_input, semantic) = convert_fixture("69-let-type-add-fires");
    assert!(
        semantic.contains("rule=r1"),
        "let type-add must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# scope=file"),
        "ranged hunk, got: {semantic}"
    );
    assert!(
        semantic.contains("- let P0 = E0;\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ let P0: u32 = E0;\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    let alpha_value"),
        "typed minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "let sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi4a_let_type_add_extra_site_stays_ordinary() {
    // Fixture 70: a fourth untouched untyped `let` in the pre-image
    // breaks totality (4 != 3) — the sweep would silently rewrite it,
    // so the section stays ordinary.
    let (_native_input, semantic) = convert_fixture("70-let-type-add-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "extra untyped let must veto, got: {semantic}"
    );
    assert!(
        semantic.contains("-    let alpha_value = compute_first(input);"),
        "native minus must survive, got: {semantic}"
    );
}
