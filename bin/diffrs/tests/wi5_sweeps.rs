//! WI-5a/WI-5b shape pins for `test-fixtures/diffrs/7{5,6}-*` (WI-5a:
//! `if`-expression verbatim sweeps) and `7{7,8,9},80-*` (WI-5b: `match`,
//! closure, `if let` verbatim sweeps whose inits are engine-matchable
//! since WI-5b). Split out of `fixtures_roundtrip.rs` to hold that
//! file's 2000-line budget (helpers are duplicated, not shared — T246
//! precedent). The generic native → semantic → native round-trip for
//! these fixtures already runs in `fixtures_roundtrip.rs`; this file
//! adds the fire/veto shape pins plus a `--check` gate on the emitted
//! semantic patch.

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
fn wi5a_if_cond_fires() {
    // Fixture 75: three uniform `let v = if flag { 1 } else { 2 };`
    // lines rewrite the else branch in one `# allow=many` hunk. The
    // init is an `if` expression — vetoed before WI-5a (SPEC 18.6 R1),
    // engine-matchable since.
    let (native_input, semantic) = convert_fixture("75-if-cond-fires");
    assert!(
        semantic.contains("rule=r1"),
        "if sweep must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# scope=file"),
        "ranged hunk, got: {semantic}"
    );
    assert!(
        semantic.contains("- let v = if flag { 1 } else { 2 };\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ let v = if flag { 1 } else { 3 };\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    let v = if flag"),
        "changed lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "if sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi5a_if_cond_extra_site_stays_ordinary() {
    // Fixture 76: a third untouched `if` line in the pre-image breaks
    // totality (3 != 2) — the sweep would silently rewrite it, so the
    // section stays ordinary.
    let (_native_input, semantic) = convert_fixture("76-if-cond-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "extra if site must veto, got: {semantic}"
    );
    assert!(
        semantic.contains("-    let v = if flag { 1 } else { 2 };"),
        "native minus must survive, got: {semantic}"
    );
}

#[test]
fn wi5b_match_scrutinee_fires() {
    // Fixture 77: three uniform `let v = match flag { ... };` lines
    // rewrite one arm in one `# allow=many` hunk. The init is a `match`
    // expression — vetoed before WI-5b (SPEC 18.6 R1), engine-matchable
    // since.
    let (native_input, semantic) = convert_fixture("77-match-scrutinee-fires");
    assert!(
        semantic.contains("rule=r1"),
        "match sweep must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# scope=file"),
        "ranged hunk, got: {semantic}"
    );
    assert!(
        semantic.contains("- let v = match flag { true => 1, _ => 2 };\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ let v = match flag { true => 1, _ => 3 };\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    let v = match flag"),
        "changed lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "match sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi5b_match_extra_site_stays_ordinary() {
    // Fixture 78: a third untouched `match` line in the pre-image breaks
    // totality (3 != 2) — the section stays ordinary.
    let (_native_input, semantic) = convert_fixture("78-match-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "extra match site must veto, got: {semantic}"
    );
    assert!(
        semantic.contains("-    let v = match flag { true => 1, _ => 2 };"),
        "native minus must survive, got: {semantic}"
    );
}

#[test]
fn wi5b_closure_body_fires() {
    // Fixture 79: three uniform `let f = |x| x;` lines rewrite the
    // closure body in one `# allow=many` hunk. Closure inits were
    // vetoed before WI-5b (SPEC 18.6 R1), engine-matchable since.
    let (native_input, semantic) = convert_fixture("79-closure-param-fires");
    assert!(
        semantic.contains("rule=r1"),
        "closure sweep must fire, got: {semantic}"
    );
    assert!(
        semantic.contains("- let f = |x| x;\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ let f = |x| x + 1;\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "closure sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi5b_if_let_scrutinee_fires() {
    // Fixture 80: three uniform `let v = if let Some(x) = y ...` lines
    // rewrite the else branch in one `# allow=many` hunk — the `If`
    // arm plus the `Let`-expression arm firing together (WI-5a + WI-5b).
    let (native_input, semantic) = convert_fixture("80-if-let-fires");
    assert!(
        semantic.contains("rule=r1"),
        "if-let sweep must fire, got: {semantic}"
    );
    assert!(
        semantic.contains("- let v = if let Some(x) = y { 1 } else { 2 };\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ let v = if let Some(x) = y { 1 } else { 3 };\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "if-let sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}
