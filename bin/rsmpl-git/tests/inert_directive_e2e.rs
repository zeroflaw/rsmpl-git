//! T169: inert directives fail fast with `RSMPL_PATTERN_CONFLICT`
//! instead of applying cleanly while constraining nothing.
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
    fs::create_dir_all(dir.path().join("src")).unwrap();
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
fn lone_fallback_fails_fast_text_and_json() {
    // T169a: `# fallback=syntax` with zero `# symbol` lines used to apply
    // cleanly (the engine symbol loop iterates zero times). Now it fails
    // fast; worktree untouched; JSON envelope carries the needle.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo() {}\nfn main() { foo(); }\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let patch = "diff --git src/lib.rs\n@@ -2,1 @@ rule=t\n# allow=many\n# fallback=syntax\n- foo()\n+ bar()\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("without `# symbol`"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), before, "untouched");
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(!out.status.success(), "json must fail");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("RSMPL_PATTERN_CONFLICT"), "got: {stdout}");
    assert!(stdout.contains("without `# symbol`"), "got: {stdout}");
}

#[test]
fn attrs_on_call_shape_fails_fast() {
    // T169b: `# attrs=strip` on a call rewrite used to apply cleanly
    // while neither matcher_expr nor matcher_macro reads the policy.
    // Now it fails fast with the SPEC 14.6 pointer; worktree untouched.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo() {}\nfn main() { foo(); }\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let patch =
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=t\n# allow=many\n# attrs=strip\n- foo()\n+ bar()\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("ignored on call/macro shapes"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), before, "untouched");
}
