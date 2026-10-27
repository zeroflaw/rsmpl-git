//! T176: method-call turbofish through the call router (SPEC 18.6 R3).
//!
//! `- R.collect::<T>()` used to be claimed by the call router as
//! `MethodCall` with an exact-compared method ident, so it never matched
//! a `collect::<Vec<i32>>()` site (T144's structural `match_turbofish`
//! lives in the general matcher, which non-`Raw` shapes never reach).
//! The call arm now compares the method turbofish structurally (mirror
//! of T145), so the probe applies byte-exact.
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
fn method_turbofish_rewrite_applies_text() {
    // T176: the probe that failed `RSMPL_MATCH_CONFLICT` before the fix.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn main() {\n    let v = vec![1, 2, 3];\n    let x = v.into_iter().collect::<Vec<i32>>();\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@ rule=collect_turbo\n# allow=many\n# expr R\n# type T\n- R.collect::<T>()\n+ R.collect::<Vec<T>>()\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "text: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn main() {\n    let v = vec![1, 2, 3];\n    let x = v.into_iter().collect::<Vec<Vec<i32>>>();\n}\n"
    );
}

#[test]
fn method_turbofish_rewrite_reports_json_match() {
    // T176: same probe through `--format json` — one match, clean check.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn main() {\n    let v = vec![1, 2, 3];\n    let x = v.into_iter().collect::<Vec<i32>>();\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@ rule=collect_turbo\n# allow=many\n# expr R\n# type T\n- R.collect::<T>()\n+ R.collect::<Vec<T>>()\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "json: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(payload["rules"][0]["matches"], 1);
}
