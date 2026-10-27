//! T177: bare call/method patterns never match turbofish sites
//! (None-vs-Some fails, general-matcher parity — SPEC 18.6 R3).
//!
//! `- R.collect()` on `collect::<Vec<i32>>()` used to apply with exit 0,
//! silently dropping the turbofish. The call router now fails safe, so
//! the agent spells `::<T>` explicitly (T176) or a concrete turbofish.
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
    if let Some(parent) = Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(dir.join(parent)).unwrap();
        }
    }
    fs::write(dir.join(rel), text).unwrap()
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
fn method_bare_pattern_fails_safe_on_turbofish_site() {
    // T177: the original probe — bare `R.collect()` on a turbofish site
    // now fails `RSMPL_MATCH_CONFLICT` instead of dropping `::<...>`.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn main() {\n    let v = vec![1, 2, 3];\n    let x = v.into_iter().collect::<Vec<i32>>();\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@ rule=collect_bare\n# allow=many\n# expr R\n- R.collect()\n+ R.collected()\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        !out.status.success(),
        "bare pattern must fail safe on turbofish site"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("RSMPL_MATCH_CONFLICT"),
        "fail-safe code, got: {stderr}"
    );
}

#[test]
fn method_bare_pattern_fails_safe_json_envelope() {
    // T177: same probe through `--format json` — stdout-only envelope.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn main() {\n    let v = vec![1, 2, 3];\n    let x = v.into_iter().collect::<Vec<i32>>();\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@ rule=collect_bare\n# allow=many\n# expr R\n- R.collect()\n+ R.collected()\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        !out.status.success(),
        "bare pattern must fail safe on turbofish site"
    );
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(payload["error"]["code"], "RSMPL_MATCH_CONFLICT");
}
