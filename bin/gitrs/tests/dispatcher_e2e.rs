//! T339 e2e: the `gitrs` dispatcher forwards to the subcommand binaries.
//!
//! `gitrs apply …` must behave exactly like `applyrs …` (exit code +
//! stdout/stderr), `gitrs diff …` reports the T340 stub until `diffrs`
//! lands, and dispatcher-level failures exit 2 with the `gitrs --help`
//! pointer (stdout-only JSON envelope when `--format json` is present).
//! Requires a workspace build so the sibling `applyrs` binary sits next
//! to the dispatcher (`cargo test --workspace`).

use std::path::PathBuf;
use std::process::Command;

fn dispatcher() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gitrs"))
}

fn sibling_applyrs() -> PathBuf {
    let d = dispatcher();
    d.parent()
        .map(|p| {
            p.join(if cfg!(windows) {
                "applyrs.exe"
            } else {
                "applyrs"
            })
        })
        .unwrap()
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

fn write(dir: &std::path::Path, rel: &str, text: &str) {
    if let Some(parent) = std::path::Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(dir.join(parent)).unwrap();
        }
    }
    std::fs::write(dir.join(rel), text).unwrap()
}

#[test]
fn help_lists_both_subcommands() {
    let out = Command::new(dispatcher()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("apply"), "got: {stdout}");
    assert!(stdout.contains("diff"), "got: {stdout}");
}

#[test]
fn version_names_dispatcher() {
    let out = Command::new(dispatcher())
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("gitrs"), "got: {stdout}");
}

#[test]
fn unknown_subcommand_fails_with_pointer() {
    let out = Command::new(dispatcher())
        .arg("frobnicate")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("unknown subcommand"), "got: {stderr}");
    assert!(stderr.contains("gitrs --help"), "got: {stderr}");
}

#[test]
fn unknown_subcommand_json_is_envelope() {
    let out = Command::new(dispatcher())
        .args(["--format", "json", "frobnicate"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!("run `gitrs --help` for usage")
    );
    assert!(v["error"]["rule"].is_null());
}

#[test]
fn bare_dispatcher_needs_subcommand() {
    let out = Command::new(dispatcher()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("no subcommand"), "got: {stderr}");
}

#[test]
fn diff_reports_t340_stub() {
    let out = Command::new(dispatcher()).arg("diff").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("not yet implemented"), "got: {stderr}");
    assert!(stderr.contains("T340"), "got: {stderr}");
}

#[test]
fn apply_passthrough_matches_applyrs() {
    // Sibling must exist: run via `cargo test --workspace`.
    assert!(
        sibling_applyrs().exists(),
        "sibling applyrs missing next to the dispatcher"
    );
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n   }\n";
    // via dispatcher
    let via = Command::new(dispatcher())
        .args(["apply", "--patch", patch, "--check"])
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    // via applyrs directly
    let direct = Command::new(sibling_applyrs())
        .args(["--patch", patch, "--check"])
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(via.status.code(), direct.status.code());
    assert_eq!(
        String::from_utf8_lossy(&via.stdout).to_string(),
        String::from_utf8_lossy(&direct.stdout).to_string()
    );
    assert!(String::from_utf8_lossy(&via.stdout).contains("bar"));
    assert!(direct.status.success());
    assert!(via.status.success());
}
