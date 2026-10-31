//! WI-9a: struct-field and enum-variant renames, authored + follow
//! (SPEC 13.4, 14.8).
//!
//! A `struct S {` (resp. `enum E {`) context header plus a member pair
//! rewrites the member ident span only. `# follow=uses` completes the
//! rename at `S`-anchored labels / `.old` accesses (fields) and
//! `E::Old` paths (variants); definition-only patches leave orphan
//! evidence while `--check` stays green. Shadowing vetoes loud without
//! `# fallback=syntax`, skips + reports with it.

mod common;
use common::*;

fn json_of(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON")
}

const FIELD_BEFORE: &str = "struct S {\n    old: u32,\n    keep: bool,\n}\nfn f(s: S, x: S) -> u32 {\n    let old = 2;\n    let lit = S { old, keep: true };\n    let other = S { old: 3, keep: false };\n    let S { old: v, keep: k } = x;\n    s.old + lit.old + other.old + v + old + k as u32\n}\n";

const FIELD_PATCH: &str = "diff --git src/lib.rs\n@@ -1,4 @@ rule=rename_field\n# follow=uses\n  struct S {\n- old: u32\n+ new: u32\n";

#[test]
fn field_follow_rewrites_definition_labels_accesses_and_patterns() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FIELD_BEFORE);
    let out = run_stdin(dir.path(), FIELD_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("new: u32,"), "def renamed, got:\n{got}");
    // Shorthand keeps the local binding: `S { old }` → `S { new: old }`.
    assert!(
        got.contains("S { new: old, keep: true }"),
        "shorthand expanded, got:\n{got}"
    );
    assert!(got.contains("S { new: 3, keep: false }"), "got:\n{got}");
    assert!(got.contains("S { new: v, keep: k }"), "got:\n{got}");
    assert!(got.contains("s.new + lit.new + other.new"), "got:\n{got}");
    // Locals and trailing bare idents are not field uses.
    assert!(got.contains("let old = 2;"), "got:\n{got}");
    assert!(got.contains("+ v + old + k"), "got:\n{got}");
    assert!(!got.contains("old: u32"), "got:\n{got}");
    assert!(!got.contains("s.old"), "got:\n{got}");
}

const FIELD_PATCH_NOFOLLOW: &str =
    "diff --git src/lib.rs\n@@ -1,4 @@ rule=rename_field\n  struct S {\n- old: u32\n+ new: u32\n";

#[test]
fn field_definition_only_reports_orphans_and_stays_green() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FIELD_BEFORE);
    let out = run_stdin(
        dir.path(),
        FIELD_PATCH_NOFOLLOW,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    let orphans = v["rules"][0]["orphans"].as_array().unwrap();
    // Literal (7, 8), pattern (9), access (10) lines; the definition
    // line and bare-`old` locals are not orphans.
    for line in [7, 8, 9, 10] {
        assert!(orphans.iter().any(|o| o["line"] == line), "got: {v}");
    }
    assert!(
        orphans.iter().all(|o| o["line"] != 2),
        "def excluded, got: {v}"
    );
}

#[test]
fn field_shadowed_definition_vetoes_then_skips_with_fallback() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S {\n    old: u32,\n}\nstruct T {\n    old: bool,\n}\nfn f(s: S) -> u32 {\n    s.old\n}\n",
    );
    let out = run_stdin(dir.path(), FIELD_PATCH, &["--check"]);
    assert!(!out.status.success(), "veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("shadowed"), "got: {stderr}");
    let patch = FIELD_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
    let out = run_stdin(
        dir.path(),
        &patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    let skipped = v["rules"][0]["follow_skipped"].as_str().unwrap();
    assert!(skipped.contains("shadowed"), "got: {v}");
}

const VARIANT_BEFORE: &str = "enum E {\n    Old(u32),\n    Keep,\n}\nuse crate::E::Old;\nfn f(e: E) -> u32 {\n    let a = E::Old(1);\n    match e {\n        E::Old(v) => v,\n        E::Keep => 0,\n    }\n}\n";

const VARIANT_PATCH: &str = "diff --git src/lib.rs\n@@ -1,4 @@ rule=rename_variant\n# follow=uses\n  enum E {\n- Old\n+ New\n";

#[test]
fn variant_follow_rewrites_definition_construction_and_patterns() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", VARIANT_BEFORE);
    let out = run_stdin(dir.path(), VARIANT_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("New(u32),"), "def renamed, got:\n{got}");
    assert!(got.contains("use crate::E::New;"), "got:\n{got}");
    assert!(got.contains("E::New(1)"), "got:\n{got}");
    assert!(got.contains("E::New(v) => v,"), "got:\n{got}");
    assert!(!got.contains("Old"), "got:\n{got}");
}

const VARIANT_PATCH_NOFOLLOW: &str =
    "diff --git src/lib.rs\n@@ -1,4 @@ rule=rename_variant\n  enum E {\n- Old\n+ New\n";

#[test]
fn variant_definition_only_reports_orphans() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", VARIANT_BEFORE);
    let out = run_stdin(
        dir.path(),
        VARIANT_PATCH_NOFOLLOW,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    let orphans = v["rules"][0]["orphans"].as_array().unwrap();
    // Import (line 5), construction (7), pattern (9) lines.
    for line in [5, 7, 9] {
        assert!(orphans.iter().any(|o| o["line"] == line), "got: {v}");
    }
}

#[test]
fn variant_bare_spelling_is_opaque_under_follow() {
    // `use E::Old;` + bare `Old(2)`: the bare spelling may be
    // import-spelled, so it never rewrites — veto without fallback,
    // skip + report with it.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "enum E {\n    Old,\n    Keep,\n}\nuse E::Old;\nfn f() -> E {\n    Old\n}\n",
    );
    let out = run_stdin(dir.path(), VARIANT_PATCH, &["--check"]);
    assert!(!out.status.success(), "veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    let patch = VARIANT_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
    let out = run_stdin(
        dir.path(),
        &patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    let skipped = v["rules"][0]["follow_skipped"].as_str().unwrap();
    assert!(skipped.contains("bare"), "got: {v}");
}

#[test]
fn variant_shadowed_definition_vetoes() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "enum E {\n    Old,\n}\nenum F {\n    Old,\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_variant\n# follow=uses\n  enum E {\n- Old\n+ New\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("shadowed"), "got: {stderr}");
}
