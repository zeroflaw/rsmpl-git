//! WI-9a/WI-9b1/WI-9b2b/WI-9b2/WI-9e shape pins for
//! `test-fixtures/diffrs/{27,82,83,84,85,86,87,88,89,90,91,92}-*`:
//! same-name macro-interior sweep (27 fires hetero, 82 bracket fires
//! delimiter-exact, 83 hetero fire with a stable long arg, 84
//! single-site stays ordinary, 85 concrete twin recovery, 86 nested
//! `env!` extraction, 87 brace fires delimiter-exact, 88 nested
//! multi-token raw-span fire, 89 cross-site hetero grouping with a
//! varying stable arg) plus WI-9e hunk-atomicity vetoes (90 verbatim,
//! 91 peel, 92 literal: a repeated group co-located with a singleton
//! drops instead of overlapping/dropping hunks). Split out of
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
    // ordinary pin; WI-9a flips it, WI-9b1 makes it hetero): two identical
    // `assert_eq!(.., TEXT_SECTION)` → `TEXT_STRIPPED` sites plus a pure
    // addition join to one hetero `# allow=many` hunk (diet `E0` for the
    // stable arg, literal `TEXT_*` delta); the tail stays an ordinary
    // insertion.
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
        !semantic.contains("# expr"),
        "diet holes, no declarations, got: {semantic}"
    );
    assert!(
        semantic.contains("- assert_eq!(E0, TEXT_SECTION)\n"),
        "minus pattern missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ assert_eq!(E0, TEXT_STRIPPED)\n"),
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
        "nested sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9_macro_hetero_xsite_fires() {
    // Fixture 89 (WI-9b2 cross-site hetero): two singleton-class pairs
    // sharing path/arity/delta but varying in the stable arg
    // (`alpha_context` vs `beta_context`) group cross-site to one
    // hetero `# allow=many` hunk — the identical-pair classes veto
    // these as N=1 each, so without the secondary pass the section
    // stays ordinary.
    let (native_input, semantic) = convert_fixture("89-macro-hetero-xsite-fires");
    assert!(
        semantic.contains("rule=r1"),
        "cross-site hetero must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# macro"),
        "plain `!` routing, no directive, got: {semantic}"
    );
    assert!(
        semantic.contains("- check!(E0, FIRST_MODE)\n"),
        "xsite hetero minus missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ check!(E0, SECOND_MODE)\n"),
        "xsite hetero plus missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    check!(alpha_context"),
        "xsite minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "xsite sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9_macro_hetero_fires() {
    // Fixture 83 (WI-9b1): two balanced same-name interior pairs whose
    // stable arg (`request_context`, 15 chars) holes to a diet `E0`
    // while the `FIRST_/SECOND_MODE` delta stays literal — hetero beats
    // concrete on bytes and the real-pair totality probe counts exactly 2.
    let (native_input, semantic) = convert_fixture("83-macro-hetero-fires");
    assert!(
        semantic.contains("rule=r1"),
        "hetero interior must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# expr"),
        "diet holes, no declarations, got: {semantic}"
    );
    assert!(
        semantic.contains("- check!(E0, FIRST_MODE)\n"),
        "hetero minus missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ check!(E0, SECOND_MODE)\n"),
        "hetero plus missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    check!(request_context, FIRST_MODE);"),
        "interior minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "hetero sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9_macro_hetero_single_site_stays_ordinary() {
    // Fixture 84: one hetero-capable pair is N=1 — the N≥2 gate keeps it
    // ordinary (same posture as the WI-9a single-site pin).
    let (native_input, semantic) = convert_fixture("84-macro-hetero-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "single site must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    check!(request_context, FIRST_MODE);"),
        "minus lines must be preserved, got: {semantic}"
    );
    assert!(
        semantic.contains("+    check!(request_context, SECOND_MODE);"),
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
fn wi9_macro_bracket_fires() {
    // Fixture 82 (WI-9b2-delim, was the bracket veto pin): two
    // identical `my_vec![1, 2]` → `my_vec![1, 3]` sites compress to one
    // concrete `# allow=many` hunk — the engine splice emits with the
    // plus pattern's delimiter, so `![...]` round-trips delimiter-exact
    // (the `--check` gate in convert_fixture proves it applies).
    let (native_input, semantic) = convert_fixture("82-macro-bracket-fires");
    assert!(
        semantic.contains("rule=r1"),
        "bracket interior must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# macro"),
        "plain `!` routing, no directive, got: {semantic}"
    );
    assert!(
        semantic.contains("- my_vec![1, 2]\n"),
        "bracket minus missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ my_vec![1, 3]\n"),
        "bracket plus missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("E0"),
        "no holes expected, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    my_vec![1, 2];"),
        "interior minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "bracket sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9_macro_brace_fires() {
    // Fixture 87 (WI-9b2-delim): two identical `my_map!{a: 1}` →
    // `my_map!{a: 2}` sites compress to one concrete `# allow=many`
    // hunk, delimiter-exact like the bracket case above.
    let (native_input, semantic) = convert_fixture("87-macro-brace-fires");
    assert!(
        semantic.contains("rule=r1"),
        "brace interior must fire, got: {semantic}"
    );
    assert!(
        semantic.contains("- my_map!{a: 1}\n"),
        "brace minus missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ my_map!{a: 2}\n"),
        "brace plus missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("E0"),
        "no holes expected, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    my_map!{a: 1};"),
        "interior minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "brace sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9_macro_concrete_twin_fires() {
    // Fixture 85 (WI-9b2a): two `check!(x, FIRST_DELTA)` sites plus an
    // untouched `check!(x, THIRD_DELTA)` twin. Hetero is absent (the only
    // stable arg `x` is shorter than any hole name); the real-pair
    // concrete probe keeps the delta literal so the twin never matches
    // and the sweep fires concrete (the old self probe vetoed → ordinary).
    let (native_input, semantic) = convert_fixture("85-macro-concrete-twin-fires");
    assert!(
        semantic.contains("rule=r1"),
        "concrete interior must fire, got: {semantic}"
    );
    assert!(
        semantic.contains("- check!(x, FIRST_DELTA)\n"),
        "concrete minus missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ check!(x, SECOND_DELTA)\n"),
        "concrete plus missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("E0"),
        "no holes expected, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "concrete sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9_macro_nested_fires() {
    // Fixture 86 (WI-9b2b): three `let` statements differing only inside
    // the nested `env!` macro (bench-17 `tool.rs` shape). The whole-line
    // interior parse declines (the `let` init is a Call) and verbatim
    // vetoes (macro in the init); the nested extraction emits one
    // concrete `# allow=many` hunk spelling just the macro pair.
    let (native_input, semantic) = convert_fixture("86-macro-nested-fires");
    assert!(
        semantic.contains("rule=r1"),
        "nested interior must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# macro"),
        "plain `!` routing, no directive, got: {semantic}"
    );
    assert!(
        semantic.contains("- env!(\"CARGO_BIN_EXE_gitrs\")\n"),
        "nested minus missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ env!(\"CARGO_BIN_EXE_applyrs\")\n"),
        "nested plus missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("E0"),
        "no holes expected, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    let out = Command::new"),
        "nested minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "nested sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9_macro_nested_multi_fires() {
    // Fixture 88 (WI-9b2 nested-raw): two nested multi-token bracket
    // deltas (my_vec 1, 2 to 1, 3 inside a call arg) compress to one
    // concrete hunk spelling just the macro pair. TokenStream display
    // would space the comma and never match byte-exact; raw-span
    // rendering keeps the source bytes.
    let (native_input, semantic) = convert_fixture("88-macro-nested-multi-fires");
    assert!(
        semantic.contains("rule=r1"),
        "nested multi-token interior must fire, got: {semantic}"
    );
    assert!(
        !semantic.contains("# macro"),
        "plain routing, no directive, got: {semantic}"
    );
    assert!(
        semantic.contains("- my_vec![1, 2]\n"),
        "nested minus missing, got: {semantic}"
    );
    assert!(
        semantic.contains("+ my_vec![1, 3]\n"),
        "nested plus missing, got: {semantic}"
    );
    assert!(
        !semantic.contains("E0"),
        "no holes expected, got: {semantic}"
    );
    assert!(
        !semantic.contains("-    let first_value = wrap"),
        "nested minus lines must be consumed, got: {semantic}"
    );
    assert!(
        semantic.len() < native_input.len(),
        "nested sweep must beat native bytes ({} vs {})",
        semantic.len(),
        native_input.len()
    );
}

#[test]
fn wi9e_verbatim_atomicity_stays_ordinary() {
    // Fixture 90 (WI-9e): H1 holds a repeated pair (foo, 3 sites) plus
    // a singleton (bar) — the foo group drops under hunk atomicity
    // (emitting it would overlap the H1 ordinary leftover), so the
    // section stays ordinary passthrough.
    let (native_input, semantic) = convert_fixture("90-verbatim-atomicity-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "atomicity drop must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    foo(x);"),
        "minus lines must be preserved, got: {semantic}"
    );
    assert!(
        semantic.contains("+    foo(y);"),
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
fn wi9e_peel_atomicity_stays_ordinary() {
    // Fixture 91 (WI-9e): H1 holds a peel pair plus a singleton;
    // H2..H5 hold the same hetero peel class. The class drops under
    // hunk atomicity (its first member sits in uncovered H1, so the
    // body could never emit and covered hunks would silently drop) —
    // ordinary passthrough, every line preserved.
    let (native_input, semantic) = convert_fixture("91-peel-atomicity-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "atomicity drop must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    let r = load(ctx, &[\"h\", m2, t1]);"),
        "covered minus lines must be preserved, got: {semantic}"
    );
    assert!(
        semantic.contains("+    let r = fetch(ctx, m2, &[t1]);"),
        "covered plus lines must be preserved, got: {semantic}"
    );
    let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
    assert!(context.is_empty(), "zero-context, got: {context:?}");
    assert!(
        semantic == normalize_native(&native_input),
        "ordinary passthrough must equal normalized native, got: {semantic}"
    );
}

#[test]
fn wi9e_literal_atomicity_stays_ordinary() {
    // Fixture 92 (WI-9e): H1 holds a literal-delta pair plus a
    // singleton; H2..H5 hold the same hetero literal class. Same
    // first-member-in-uncovered-hunk drop as the peel driver —
    // ordinary passthrough, every line preserved.
    let (native_input, semantic) = convert_fixture("92-literal-atomicity-stays-ordinary");
    assert!(
        !semantic.contains("rule="),
        "atomicity drop must stay ordinary, got: {semantic}"
    );
    assert!(
        semantic.contains("-    check(c2, \"a\");"),
        "covered minus lines must be preserved, got: {semantic}"
    );
    assert!(
        semantic.contains("+    check(c2, \"b\");"),
        "covered plus lines must be preserved, got: {semantic}"
    );
    let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
    assert!(context.is_empty(), "zero-context, got: {context:?}");
    assert!(
        semantic == normalize_native(&native_input),
        "ordinary passthrough must equal normalized native, got: {semantic}"
    );
}
