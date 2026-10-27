//! Stdin-patch e2e: `rsmpl-git` reading the patch from stdin, piped or
//! via the T129 no-input-flags default.
//!
//! Split from `cli_e2e.rs` to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

mod common;
use common::*;

#[test]
fn stdin_patch_applies() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path(), "src/lib.rs").contains("fn bar(x: i32)"));
}

#[test]
fn stdin_default_applies_without_input_flags() {
    // T129: no `--file` means stdin (Unix filter behavior).
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path(), "src/lib.rs").contains("fn bar(x: i32)"));
}

#[test]
fn stdin_default_empty_is_parse_conflict_text_and_json() {
    // T129: empty stdin is the T59 empty-patch `RSMPL_PARSE_CONFLICT`,
    // not silent success — text keeps the stderr diagnostic, JSON carries
    // the envelope on stdout (exit 2 both).
    for json in [false, true] {
        let dir = init_repo();
        write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
        let extra: &[&str] = if json { &["--format", "json"] } else { &[] };
        let out = run_stdin(dir.path(), "", extra);
        assert_eq!(out.status.code(), Some(2));
        if json {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let v: serde_json::Value =
                serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
            assert_eq!(
                v["error"]["code"],
                serde_json::json!("RSMPL_PARSE_CONFLICT"),
                "got: {v}"
            );
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
        }
    }
}
