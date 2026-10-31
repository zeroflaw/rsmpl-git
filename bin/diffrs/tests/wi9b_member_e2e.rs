//! WI-9b e2e: field/variant member rename compresses the definition
//! plus use-site lines to one hunk with `# follow=uses`; real-apply is
//! byte-identical. Shadowed definitions and bare-`Old` uses veto to
//! ordinary (the probe declines instead of emitting a half-follow).
//!
//! Split out to hold the 2000-line budget (helpers duplicated, not
//! shared — T246 precedent).

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

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn commit_all(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(
        dir,
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
    );
}

fn write(dir: &Path, rel: &str, text: &str) {
    if let Some(parent) = Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(dir.join(parent)).unwrap();
        }
    }
    std::fs::write(dir.join(rel), text).unwrap()
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

fn real_apply(dir: &Path, patch: &str) -> String {
    git(dir, &["stash", "-q"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(patch)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    std::fs::read_to_string(dir.join("src/lib.rs")).unwrap()
}

const FIELD_BEFORE: &str = "struct ConnectionConfig {\n    request_timeout_milliseconds: u32,\n    max_retry_attempts: u8,\n}\nfn latency(cfg: ConnectionConfig) -> u32 {\n    cfg.request_timeout_milliseconds + cfg.max_retry_attempts as u32\n}\nfn spawn() -> ConnectionConfig {\n    ConnectionConfig { request_timeout_milliseconds: 5000, max_retry_attempts: 3 }\n}\nfn describe(cfg: ConnectionConfig) -> u32 {\n    let ConnectionConfig { request_timeout_milliseconds: current, max_retry_attempts: _left } = cfg;\n    current\n}\nfn duplicate(cfg: ConnectionConfig) -> ConnectionConfig {\n    ConnectionConfig { request_timeout_milliseconds: cfg.request_timeout_milliseconds, max_retry_attempts: cfg.max_retry_attempts }\n}\nfn quick() -> ConnectionConfig {\n    let request_timeout_milliseconds = 42;\n    ConnectionConfig { request_timeout_milliseconds, max_retry_attempts: 0 }\n}\n";

const FIELD_AFTER: &str = "struct ConnectionConfig {\n    response_deadline_milliseconds: u32,\n    max_retry_attempts: u8,\n}\nfn latency(cfg: ConnectionConfig) -> u32 {\n    cfg.response_deadline_milliseconds + cfg.max_retry_attempts as u32\n}\nfn spawn() -> ConnectionConfig {\n    ConnectionConfig { response_deadline_milliseconds: 5000, max_retry_attempts: 3 }\n}\nfn describe(cfg: ConnectionConfig) -> u32 {\n    let ConnectionConfig { response_deadline_milliseconds: current, max_retry_attempts: _left } = cfg;\n    current\n}\nfn duplicate(cfg: ConnectionConfig) -> ConnectionConfig {\n    ConnectionConfig { response_deadline_milliseconds: cfg.response_deadline_milliseconds, max_retry_attempts: cfg.max_retry_attempts }\n}\nfn quick() -> ConnectionConfig {\n    let request_timeout_milliseconds = 42;\n    ConnectionConfig { response_deadline_milliseconds: request_timeout_milliseconds, max_retry_attempts: 0 }\n}\n";

#[test]
fn field_member_rename_compresses_to_follow_hunk() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FIELD_BEFORE);
    commit_all(dir.path());
    write(dir.path(), "src/lib.rs", FIELD_AFTER);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        patch.contains("  struct ConnectionConfig {"),
        "got: {patch}"
    );
    assert!(
        patch.contains("- request_timeout_milliseconds: u32"),
        "got: {patch}"
    );
    assert!(
        patch.contains("+ response_deadline_milliseconds: u32"),
        "got: {patch}"
    );
    assert!(patch.contains("# follow=uses"), "got: {patch}");
    assert!(
        !patch.contains("cfg.request_timeout_milliseconds"),
        "use lines must be consumed, got: {patch}"
    );

    let got = real_apply(dir.path(), &patch);
    assert_eq!(got, FIELD_AFTER, "real-apply must reproduce the post-tree");
}

const VARIANT_BEFORE: &str = "enum ConnectionState {\n    EstablishedConnection(u32),\n    ClosedConnection,\n}\nuse crate::ConnectionState::EstablishedConnection;\nfn handle(state: ConnectionState) -> u32 {\n    let first = ConnectionState::EstablishedConnection(1);\n    let second = ConnectionState::EstablishedConnection(2);\n    match state {\n        ConnectionState::EstablishedConnection(value) => value + first + second,\n        ConnectionState::ClosedConnection => 0,\n    }\n}\n";

const VARIANT_AFTER: &str = "enum ConnectionState {\n    ActiveSession(u32),\n    ClosedConnection,\n}\nuse crate::ConnectionState::ActiveSession;\nfn handle(state: ConnectionState) -> u32 {\n    let first = ConnectionState::ActiveSession(1);\n    let second = ConnectionState::ActiveSession(2);\n    match state {\n        ConnectionState::ActiveSession(value) => value + first + second,\n        ConnectionState::ClosedConnection => 0,\n    }\n}\n";

#[test]
fn variant_member_rename_compresses_to_follow_hunk() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", VARIANT_BEFORE);
    commit_all(dir.path());
    write(dir.path(), "src/lib.rs", VARIANT_AFTER);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("  enum ConnectionState {"), "got: {patch}");
    assert!(patch.contains("- EstablishedConnection\n"), "got: {patch}");
    assert!(patch.contains("+ ActiveSession\n"), "got: {patch}");
    assert!(patch.contains("# follow=uses"), "got: {patch}");

    let got = real_apply(dir.path(), &patch);
    assert_eq!(
        got, VARIANT_AFTER,
        "real-apply must reproduce the post-tree"
    );
}

#[test]
fn shadowed_field_definition_stays_ordinary() {
    // A second same-named field anywhere vetoes the authored follow, so
    // the probe declines and the class stays ordinary.
    let dir = init_repo();
    let before = "struct Alpha {\n    timeout_value_milliseconds: u32,\n}\nstruct Beta {\n    timeout_value_milliseconds: u32,\n}\nfn f(a: Alpha) -> u32 {\n    a.timeout_value_milliseconds\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = before.replace("timeout_value_milliseconds", "deadline_value_milliseconds");
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.contains("follow=uses"),
        "shadowed class must not raise with follow, got: {patch}"
    );
}

#[test]
fn bare_variant_use_stays_ordinary() {
    // `use E::Old;` + bare `Old` is opaque: the follow would veto, so
    // the probe declines and the class stays ordinary.
    let dir = init_repo();
    let before = "enum SessionState {\n    Active,\n    Idle,\n}\nuse SessionState::Active;\nfn current() -> SessionState {\n    Active\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = before.replace("Active", "Ready");
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.contains("follow=uses"),
        "bare-use class must not raise with follow, got: {patch}"
    );
}
