//! T300: engine-verbatim sigiled/bare mismatch diagnostics
//! (SPEC 15.5 / 25.2 / 25.4).
//!
//! v0.1 implements both `# metavars` modes (SPEC 15.5 no longer hedges).
//! Neither mismatch failure was verbatim-pinned before:
//!
//! - bare `E` under `# metavars=sigiled` means the literal Rust
//!   identifier `E`, so against any other target the hunk fails with
//!   the standard `RSMPL_MATCH_CONFLICT` no-match diagnostic
//!   (rule + file + old_range, set suggestion; nearest-source preview
//!   parameterized per target like Section 25.1);
//! - `$E` under bare (or default) mode is not a metavariable reference:
//!   the literal `$` survives into the output and the rewritten-file
//!   reparse fails with `RSMPL_PARSE_CONFLICT`
//!   `rewritten file does not parse as Rust: expected an expression`
//!   (file-only envelope; the `: {parser error}` suffix varies;
//!   canonical fence in Section 25.2).
//!
//! Two success controls prove the positive rules: sigiled `$E` rewrites
//! structurally, and sigiled bare `E` matches a literal `E` in the
//! target. The worktree stays untouched in every failure case.

mod common;
use common::*;

const BASE: &str = "fn f() { let x = 1 + 2; }\n";
const LITERAL: &str = "fn f() { let x = E + 2; }\n";

fn check_json(patch: &str, file: &str, content: &str) -> serde_json::Value {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    v
}

#[test]
fn sigiled_bare_means_literal_no_match() {
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# metavars=sigiled\n# expr E\n- E + 2\n+ E + 3\n";
    let reason = "no match found \u{2014} nearest source at line 1: 'fn f() { let x = 1 + 2; }'";
    let suggestion = "read `a.rs` to list valid ranges; check the pattern or add `# optional`";
    let dir = init_repo();
    write(dir.path(), "a.rs", BASE);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(stderr.contains("old_range: 1..2"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(stderr.contains(suggestion), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, BASE, "worktree must be untouched");
    let v = check_json(patch, "a.rs", BASE);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
    assert_eq!(v["error"]["range"], serde_json::json!("1..2"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
}

#[test]
fn bare_dollar_fails_rewritten_reparse() {
    let reason = "rewritten file does not parse as Rust: expected an expression";
    for patch in [
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n- $E + 2\n+ $E + 3\n",
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# metavars=bare\n# expr E\n- $E + 2\n+ $E + 3\n",
    ] {
        let dir = init_repo();
        write(dir.path(), "a.rs", BASE);
        let out = run_stdin(dir.path(), patch, &["--check"]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "patch {patch:?} stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
        assert!(stderr.contains("file: a.rs"), "got: {stderr}");
        assert!(stderr.contains(reason), "got: {stderr}");
        assert!(!stderr.contains("rule:"), "got: {stderr}");
        assert!(!stderr.contains("old_range:"), "got: {stderr}");
        assert!(!stderr.contains("suggestion:"), "got: {stderr}");
        let got = read(dir.path(), "a.rs");
        assert_eq!(got, BASE, "worktree must be untouched");
        let v = check_json(patch, "a.rs", BASE);
        assert_eq!(
            v["error"]["code"],
            serde_json::json!("RSMPL_PARSE_CONFLICT")
        );
        assert!(v["error"]["rule"].is_null(), "got: {v}");
        assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
        assert!(v["error"]["range"].is_null(), "got: {v}");
        assert_eq!(v["error"]["reason"], serde_json::json!(reason));
        assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    }
}

#[test]
fn sigiled_dollar_rewrites() {
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# metavars=sigiled\n# expr E\n- $E + 2\n+ $E + 3\n";
    let dir = init_repo();
    write(dir.path(), "a.rs", BASE);
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, "fn f() { let x = 1 + 3; }\n", "got: {got}");
}

#[test]
fn sigiled_bare_matches_literal() {
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# metavars=sigiled\n# expr E\n- E + 2\n+ E + 3\n";
    let dir = init_repo();
    write(dir.path(), "a.rs", LITERAL);
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, "fn f() { let x = E + 3; }\n", "got: {got}");
}
