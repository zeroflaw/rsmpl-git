//! T26 e2e: clap usage errors print the stdout-only JSON envelope in
//! json mode (SPEC D.2) and keep clap's stderr text otherwise.
//!
//! Split from `cli_e2e.rs` to keep every test file under the 2000-LOC
//! repo limit. Also pins the T99 `wants_json` spellings (position +
//! `--format=json` form) and the T105 single-scanner state (only
//! `wants_json` remains; the inspect hint scanner died with T122).

use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git"))
}

fn run_raw(args: &[&str]) -> std::process::Output {
    Command::new(binary()).args(args).output().unwrap()
}

fn assert_envelope(out: &std::process::Output, needle: &str) {
    assert_eq!(
        out.status.code(),
        Some(2),
        "usage error must exit 2, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be the JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_IO_CONFLICT"),
        "got: {v}"
    );
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains(needle),
        "reason must name the usage fault, got: {v}"
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!("run `rsmpl-git --help` for usage"),
        "got: {v}"
    );
    // T89 symmetry: stdout-only envelope, no stderr `error:` line.
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !stderr.contains("error:"),
        "json usage failure must not print a stderr error line, got: {stderr}"
    );
    // Six-key shape holds for usage errors too (absent optionals null).
    for key in ["rule", "file", "range"] {
        assert!(
            v["error"][key].is_null(),
            "usage envelope {key} must be null, got: {v}"
        );
    }
}

#[test]
fn json_unknown_flag_is_envelope() {
    let out = run_raw(&["--format", "json", "--bogus-flag"]);
    assert_envelope(&out, "--bogus-flag");
}

#[test]
fn json_unknown_flag_spelling_variants_envelop() {
    // T99 pin: `wants_json` covers `--format=json` and any position, so
    // the envelope must not depend on spelling or order.
    let out = run_raw(&["--format=json", "--bogus-flag"]);
    assert_envelope(&out, "--bogus-flag");
    let out = run_raw(&["--bogus-flag", "--format", "json"]);
    assert_envelope(&out, "--bogus-flag");
}

#[test]
fn text_unknown_flag_stays_clap() {
    let out = run_raw(&["--bogus-flag"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "text usage error must not print stdout, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("unexpected argument"),
        "clap text must surface, got: {stderr}"
    );
}

#[test]
fn json_both_inputs_is_envelope() {
    // `conflicts_with` fires before `run()`; the envelope must still hold.
    let out = run_raw(&["--patch", "X", "--file", "Y", "--format", "json"]);
    assert_envelope(&out, "--patch");
}

#[test]
fn json_removed_context_flag_is_envelope() {
    // Complement to the T128 text-only `context_flag_is_gone` pin: the
    // removed spelling is a usage error, so json mode envelops it.
    let out = run_raw(&["--format", "json", "--context", "0"]);
    assert_envelope(&out, "--context");
}
