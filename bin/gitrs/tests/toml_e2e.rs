//! T260: `.toml` files (e.g. `Cargo.toml`) flow through ordinary hunks.
//!
//! The T137 language-agnostic path already applies them (no Rust parse
//! for ordinary-only sections, SPEC 11); RSmPL semantic hunks stay
//! Rust-only and fail with `RSMPL_PARSE_CONFLICT`. These tests pin both
//! sides so agents route TOML edits to ordinary hunks.

mod common;
use common::*;

#[test]
fn cargo_toml_ordinary_hunk_applies() {
    let dir = init_repo();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n",
    );
    let patch =
        "diff --git Cargo.toml\n@@ -1,3 +1,3 @@\n [package]\n name = \"foo\"\n-version = \"0.1.0\"\n+version = \"0.2.0\"\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "Cargo.toml");
    assert!(got.contains("version = \"0.2.0\""), "got: {got}");
    assert!(got.contains("name = \"foo\""), "got: {got}");
}

#[test]
fn rsmpl_hunk_on_toml_is_parse_conflict() {
    let dir = init_repo();
    write(dir.path(), "Cargo.toml", "[package]\nname = \"foo\"\n");
    let patch = "diff --git Cargo.toml\n@@ -1,2 @@ rule=x\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(!out.status.success(), "semantic TOML hunk must fail");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT"),
        "got: {v}"
    );
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("does not parse as Rust:"),
        "reason carries the parser-error suffix, got: {v}"
    );
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
}
