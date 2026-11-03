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

fn sibling(name: &str) -> PathBuf {
    let d = dispatcher();
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    d.parent().map(|p| p.join(&file)).unwrap()
}

fn sibling_applyrs() -> PathBuf {
    sibling("applyrs")
}

fn sibling_diffrs() -> PathBuf {
    sibling("diffrs")
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
    assert_eq!(v["version"], serde_json::json!(rsmpl_patch::REPORT_VERSION));
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!("run `gitrs --help` for usage")
    );
    assert!(v["error"]["rule"].is_null());
}

// WI-20: one contract — `applyrs`, `diffrs`, and the `gitrs`
// dispatcher take the report schema number from the single
// `rsmpl_patch::REPORT_VERSION` constant, and the sibling
// version-skew handshake transitively pins schema parity.
#[test]
fn tri_way_envelope_reports_unified_schema_v5() {
    assert!(
        sibling_applyrs().exists() && sibling_diffrs().exists(),
        "siblings missing next to the dispatcher"
    );
    let dir = init_repo();
    // applyrs error envelope (empty patch is an authoring typo).
    let out_applyrs = Command::new(sibling_applyrs())
        .args(["--patch", "", "--format", "json"])
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out_applyrs.status.code(), Some(2));
    // diffrs error envelope (unresolvable revision).
    let out_diffrs = Command::new(sibling_diffrs())
        .args(["nosuchrev12345", "--format", "json"])
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out_diffrs.status.code(), Some(2));
    // gitrs dispatcher error envelope.
    let out_gitrs = Command::new(dispatcher())
        .args(["--format", "json", "frobnicate"])
        .output()
        .unwrap();
    assert_eq!(out_gitrs.status.code(), Some(2));
    let parse = |out: &std::process::Output| -> serde_json::Value {
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap()
    };
    let (a, d, g) = (parse(&out_applyrs), parse(&out_diffrs), parse(&out_gitrs));
    assert_eq!(a["version"], serde_json::json!(rsmpl_patch::REPORT_VERSION));
    assert_eq!(d["version"], serde_json::json!(rsmpl_patch::REPORT_VERSION));
    assert_eq!(g["version"], serde_json::json!(rsmpl_patch::REPORT_VERSION));
    // The handshake compares `--version` last tokens; skewed siblings
    // cannot silently disagree on the schema they report.
    let token = |out: &std::process::Output| {
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .last()
            .unwrap()
            .to_string()
    };
    let v_gitrs = Command::new(dispatcher())
        .arg("--version")
        .output()
        .unwrap();
    let v_applyrs = Command::new(sibling_applyrs())
        .arg("--version")
        .output()
        .unwrap();
    let v_diffrs = Command::new(sibling_diffrs())
        .arg("--version")
        .output()
        .unwrap();
    assert!(v_gitrs.status.success() && v_applyrs.status.success() && v_diffrs.status.success());
    assert_eq!(token(&v_gitrs), token(&v_applyrs));
    assert_eq!(token(&v_gitrs), token(&v_diffrs));
}

#[test]
fn bare_dispatcher_needs_subcommand() {
    let out = Command::new(dispatcher()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("no subcommand"), "got: {stderr}");
}

#[test]
fn diff_passthrough_matches_diffrs() {
    // Sibling must exist: run via `cargo test --workspace`.
    assert!(
        sibling_diffrs().exists(),
        "sibling diffrs missing next to the dispatcher"
    );
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let via: Vec<String> = vec![
        "diff".to_string(),
        "--repo".to_string(),
        dir.path().to_string_lossy().to_string(),
    ];
    // Untracked file: `gitrs diff` must surface it (git-diff-only
    // would report nothing).
    let out_via = Command::new(dispatcher()).args(&via).output().unwrap();
    let out_direct = Command::new(sibling_diffrs())
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out_via.status.code(), Some(0));
    assert_eq!(out_direct.status.code(), Some(0));
    let via_stdout = String::from_utf8_lossy(&out_via.stdout).to_string();
    let direct_stdout = String::from_utf8_lossy(&out_direct.stdout).to_string();
    assert_eq!(via_stdout, direct_stdout);
    assert!(direct_stdout.contains("new file mode"));
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

// T246 session 20: `gitrs diff` forwards every surface cell to the
// sibling `diffrs` byte-identical (pending, --staged, [REV], two-dot
// range, --format json, error paths) — the agent entry point never
// drifts from the binary it invokes.
#[test]
fn diff_forwards_all_cells_identical_to_diffrs() {
    assert!(
        sibling_diffrs().exists(),
        "sibling diffrs missing next to the dispatcher"
    );
    let dir = init_repo();
    let g = |args: &[&str]| {
        let st = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(st.status.success(), "git {args:?} failed");
    };
    g(&["config", "user.email", "t@t"]);
    g(&["config", "user.name", "t"]);
    g(&["config", "commit.gpgsign", "false"]);
    write(dir.path(), "f.rs", "fn v1() {}\n");
    g(&["add", "-A"]);
    g(&["commit", "-qm", "one"]);
    write(dir.path(), "f.rs", "fn v2() {}\n");
    g(&["add", "-A"]);

    let cells: &[&[&str]] = &[
        &[],
        &["--staged"],
        &["HEAD~1"],
        &["HEAD~1..HEAD"],
        &["--format", "json"],
        &["nosuchrev12345"],
    ];
    for cell in cells {
        let mut direct_args: Vec<String> = cell.iter().map(|s| s.to_string()).collect();
        direct_args.push("--repo".to_string());
        direct_args.push(dir.path().to_string_lossy().to_string());
        let mut via_args = vec!["diff".to_string()];
        via_args.extend(direct_args.clone());
        let out_via = Command::new(dispatcher()).args(&via_args).output().unwrap();
        let out_direct = Command::new(sibling_diffrs())
            .args(&direct_args)
            .output()
            .unwrap();
        assert_eq!(
            out_via.status.code(),
            out_direct.status.code(),
            "cell {cell:?}"
        );
        assert_eq!(out_via.stdout, out_direct.stdout, "cell {cell:?}");
        assert_eq!(out_via.stderr, out_direct.stderr, "cell {cell:?}");
    }
}
