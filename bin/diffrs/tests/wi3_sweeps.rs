//! WI-3 shape pins for `test-fixtures/diffrs/6{1-8}-*`: qualified-call,
//! turbofish, macro-rename, and qualified-macro (WI-3 residue) sweeps.
//! Split out of
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
fn wi3_qualified_call_fires() {
    // Fixture 61: three `crate::old_handler_name(..)` sites rewrite to
    // `crate::new_handler_name(..)` in one `# allow=many` hunk with the
    // concrete path form (no `# symbol` tax).
    let (native_input, semantic) = convert_fixture("61-qualified-call-fires");
    assert!(
        semantic.contains("rule=r1"),
        "qualified rename must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# scope=file"),
        "ranged hunk, got: {semantic}"
    );
    assert!(
        semantic.contains("- crate::old_handler_name(E0)\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ crate::new_handler_name(E0)\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    crate::old_handler_name("),
        "renamed minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "qualified sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi3_qualified_call_divergent_prefix_stays_ordinary() {
    // Fixture 62: `crate::old` -> `crate::new` plus `other::old` ->
    // `other::new` share no prefix class — veto side of fixture 61.
    let (native_input, semantic) = convert_fixture("62-qualified-call-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "divergent-prefix edit must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    crate::old_handler_name(first_argument);"),
        "minus lines must be preserved, got: {semantic}"
    );
    assert!(
        semantic.contains("+    other::new_handler_name(second_argument);"),
        "plus lines must be preserved, got: {semantic}"
    );
    let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
    assert!(context.is_empty(), "zero-context, got: {context:?}");
    assert!(
        semantic == normalize_native(&native_input),
        "ordinary passthrough must equal normalized native, got: {semantic}"
    );
}

#[test]
fn wi3_turbofish_fires() {
    // Fixture 63: three `old_deserializer_name::<u8|i32|u64>(..)` sites
    // rewrite with the turbo position varying — the hunk binds an
    // inferred `T` on both sides and replays per-site types.
    let (native_input, semantic) = convert_fixture("63-turbofish-fires");
    assert!(
        semantic.contains("rule=r1"),
        "turbofish rename must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# scope=file"),
        "ranged hunk, got: {semantic}"
    );
    assert!(
        semantic.contains("- old_deserializer_name::<T>(E0)\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ new_deserializer_name::<T>(E0)\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    old_deserializer_name::"),
        "renamed minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "turbofish sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi3_turbofish_arity_divergent_stays_ordinary() {
    // Fixture 64: `::<u8>` vs `::<u8, u16>` share no turbo class (arity
    // is match-exact) — veto side of fixture 63.
    let (native_input, semantic) = convert_fixture("64-turbofish-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "turbo-arity-divergent edit must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    old_deserializer_name::<u8>(first_argument);"),
        "minus lines must be preserved, got: {semantic}"
    );
    let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
    assert!(context.is_empty(), "zero-context, got: {context:?}");
    assert!(
        semantic == normalize_native(&native_input),
        "ordinary passthrough must equal normalized native, got: {semantic}"
    );
}

#[test]
fn wi3_macro_rename_fires() {
    // Fixture 65: three hetero-body `old_render_widget!(...)` sites
    // (verbatim could never group them, count 1 each) rewrite in one
    // `# macro=token_tree` hunk with the verbatim `...` body.
    let (native_input, semantic) = convert_fixture("65-macro-rename-fires");
    assert!(
        semantic.contains("rule=r1"),
        "macro rename must fire, got: {semantic}"
    );
    assert!(
        semantic.contains("# macro=token_tree\n"),
        "macro directive missing, got: {semantic}"
    );
    assert!(
        semantic.contains("- old_render_widget!(...)\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ new_render_widget!(...)\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    old_render_widget!("),
        "renamed minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "macro sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi3_qualified_macro_fires() {
    // Fixture 67 (WI-3 residue): three hetero-body
    // `crate::old_render_widget!(...)` sites rewrite in one
    // `# macro=token_tree` hunk with the concrete qualified path.
    let (native_input, semantic) = convert_fixture("67-qualified-macro-fires");
    assert!(
        semantic.contains("rule=r1"),
        "qualified macro rename must fire, got: {semantic}"
    );
    assert!(
        semantic.contains("# macro=token_tree\n"),
        "macro directive missing, got: {semantic}"
    );
    assert!(
        semantic.contains("- crate::old_render_widget!(...)\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ crate::new_render_widget!(...)\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    crate::old_render_widget!("),
        "renamed minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "qualified macro sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi3_qualified_macro_prefix_divergent_stays_ordinary() {
    // Fixture 68: `crate::old` -> `crate::new` plus `other::old` ->
    // `other::new` share no path class — veto side of fixture 67.
    let (native_input, semantic) = convert_fixture("68-qualified-macro-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "prefix-divergent macros must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    crate::old_render_widget!(first_widget_argument);"),
        "minus lines must be preserved, got: {semantic}"
    );
    let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
    assert!(context.is_empty(), "zero-context, got: {context:?}");
    assert!(
        semantic == normalize_native(&native_input),
        "ordinary passthrough must equal normalized native, got: {semantic}"
    );
}

#[test]
fn wi3_macro_interior_stays_ordinary() {
    // Fixture 66: same-name `old_render_widget!(..)` bodies rewritten
    // per site are interior edits (WI-9), not a rename — veto side of
    // fixture 65.
    let (native_input, semantic) = convert_fixture("66-macro-rename-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "macro-interior edit must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    old_render_widget!(first_widget_argument);"),
        "minus lines must be preserved, got: {semantic}"
    );
    let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
    assert!(context.is_empty(), "zero-context, got: {context:?}");
    assert!(
        semantic == normalize_native(&native_input),
        "ordinary passthrough must equal normalized native, got: {semantic}"
    );
}

#[test]
fn wi7d_leading_colon_call_fires() {
    // Fixture 93 (WI-7d): three shared-`::`
    // `::framework::old_handler_name(..)` sites rewrite in one
    // `# allow=many` hunk with the leading `::` spelled verbatim.
    let (native_input, semantic) = convert_fixture("93-leading-colon-call-fires");
    assert!(
        semantic.contains("rule=r1"),
        "leading-`::` rename must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# scope=file"),
        "ranged hunk, got: {semantic}"
    );
    assert!(
        semantic.contains("- ::framework::old_handler_name(E0)\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ ::framework::new_handler_name(E0)\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    ::framework::old_handler_name("),
        "renamed minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "leading-`::` sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi7d_qself_call_fires() {
    // Fixture 94 (WI-7d): three shared-qualifier
    // `<TargetType as ConversionTrait>::old_handler_name(..)` sites
    // rewrite in one `# allow=many` hunk with the qualifier spelled
    // verbatim (token-equal, never resolved).
    let (native_input, semantic) = convert_fixture("94-qself-call-fires");
    assert!(
        semantic.contains("rule=r1"),
        "qself rename must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# scope=file"),
        "ranged hunk, got: {semantic}"
    );
    assert!(
        semantic.contains("- <TargetType as ConversionTrait>::old_handler_name(E0)\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ <TargetType as ConversionTrait>::new_handler_name(E0)\n"),
        "plus pattern missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    <TargetType as ConversionTrait>::old_handler_name("),
        "renamed minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "qself sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}
