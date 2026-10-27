//! T165: explicitly written fn attributes under default preserve.
//!
//! Split into its own file to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git"))
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let st = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(st.success());
    dir
}

fn write(dir: &Path, rel: &str, text: &str) {
    fs::write(dir.join(rel), text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

fn run_stdin(dir: &Path, patch: &str, extra: &[&str]) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(binary())
        .args(extra)
        .arg("--repo")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn literal_mirror_applies_byte_exact() {
    // The documented SPEC 14.6 spelling: written `#[inline]` on both sides
    // constrains and preserves, landing byte-exact.
    let dir = init_repo();
    write(dir.path(), "a.rs", "#[inline]\nfn foo() {}\n");
    let patch = "diff --git a.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- #[inline]\n- fn foo(...) {\n+ #[inline]\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "#[inline]\nfn bar() {}\n");
}

#[test]
fn wrong_attribute_fails_safe_text_and_json() {
    // T165 dogfood probe: minus `#[other]` vs target `#[inline]` must not
    // silently rename the wrong item. The site is skipped, count
    // validation fails with `RSMPL_MATCH_CONFLICT`, worktree untouched.
    let dir = init_repo();
    write(dir.path(), "a.rs", "#[inline]\nfn foo() {}\n");
    let patch = "diff --git a.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- #[other]\n- fn foo(...) {\n+ #[other]\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert_eq!(read(dir.path(), "a.rs"), "#[inline]\nfn foo() {}\n");
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(!out.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(payload["error"]["code"], "RSMPL_MATCH_CONFLICT");
}

#[test]
fn attributes_abstraction_rejected_with_needle() {
    // `# attributes A` has no engine binding (T165); the compile gate
    // fails fast with the repair instead of misrouting.
    let dir = init_repo();
    write(dir.path(), "a.rs", "#[inline]\nfn foo() {}\n");
    let patch = "diff --git a.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n# attributes A\n- A\n- fn foo(...) {\n+ A\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    assert!(
        err.contains("attribute lists cannot be abstracted"),
        "got: {err}"
    );
    assert_eq!(read(dir.path(), "a.rs"), "#[inline]\nfn foo() {}\n");
}

#[test]
fn item_preserve_written_mismatch_fails_safe_text_and_json() {
    // T235 dogfood probe: item renames reuse the fn attribute contract
    // (SPEC 14.6/14.8) — a written `///` the target lacks skips the site
    // instead of silently renaming it.
    let dir = init_repo();
    write(dir.path(), "a.rs", "struct Local;\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- /// Local thing.\n- struct Local\n+ /// Local thing.\n+ struct Remote\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert_eq!(read(dir.path(), "a.rs"), "struct Local;\n");
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(!out.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(payload["error"]["code"], "RSMPL_MATCH_CONFLICT");
}

#[test]
fn item_plus_attrs_differ_rejected_with_needle() {
    // T235: the item rewrite only replaces the ident span, so plus-side
    // attrs differing from the minus side are rejected (fn/general
    // parity) instead of silently dropped.
    let dir = init_repo();
    write(dir.path(), "a.rs", "/// Other thing.\nstruct Other;\n");
    let patch = "diff --git a.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- /// Other thing.\n- struct Other\n+ /// Changed thing.\n+ struct Renamed\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    assert!(err.contains("differ from matched attributes"), "got: {err}");
    assert_eq!(
        read(dir.path(), "a.rs"),
        "/// Other thing.\nstruct Other;\n"
    );
}
