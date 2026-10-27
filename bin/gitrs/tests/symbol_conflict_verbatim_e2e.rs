//! T316: engine-verbatim `RSMPL_SYMBOL_CONFLICT` envelope (SPEC 18.6 C13 / 25.5).
//!
//! SPEC 25.5 carried the fence but its envelope sentence was loose (named
//! only JSON `range: null`); the C13 table row carried no envelope at all
//! (unlike C14/C15, hardened in T276). Probed `target/debug/gitrs`
//! (exit 2): text carries `rule:`/`file:`/`reason:`/`suggestion:` with the
//! null range omitted (no `old_range:` line); JSON carries `rule`/`file`
//! set, `range: null`, and the suggestion as a set string. `{Name}` echoes
//! the declared `# symbol Old=...` name, backticked.

mod common;
use common::*;

const REASON: &str = "symbol `Old` could not be resolved (no semantic environment in v0.1)";
const SUGGESTION: &str = "add `# fallback=syntax` to allow syntactic matching";

fn patch() -> &'static str {
    "diff --git a.rs\n@@ -1,1 @@ rule=symtest\n# scope=file\n# allow=many\n# symbol Old=crate::foo\n# expr E\n- Old(E)\n+ new_api(E)\n"
}

fn content() -> &'static str {
    "fn main() { old_api(1); }\n"
}

#[test]
fn symbol_without_fallback_text_envelope_verbatim() {
    let dir = init_repo();
    write(dir.path(), "a.rs", content());
    let out = run_stdin(dir.path(), patch(), &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: symtest"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(stderr.contains(REASON), "got: {stderr}");
    assert!(
        stderr.contains("suggestion: add `# fallback=syntax` to allow syntactic matching"),
        "got: {stderr}"
    );
    assert!(
        !stderr.contains("old_range:"),
        "null range must be omitted from text, got: {stderr}"
    );
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content(), "worktree must be untouched");
}

#[test]
fn symbol_without_fallback_json_envelope_verbatim() {
    let dir = init_repo();
    write(dir.path(), "a.rs", content());
    let out = run_stdin(dir.path(), patch(), &["--format", "json", "--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_SYMBOL_CONFLICT"),
        "got: {v}"
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("symtest"), "got: {v}");
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"), "got: {v}");
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("symbol `Old` could not be resolved (no semantic environment in v0.1)"),
        "got: {v}"
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!("add `# fallback=syntax` to allow syntactic matching"),
        "got: {v}"
    );
    assert_eq!(v["error"]["range"], serde_json::Value::Null, "got: {v}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content(), "worktree must be untouched");
}

#[test]
fn symbol_without_fallback_apply_mode_leaves_worktree_untouched() {
    let dir = init_repo();
    write(dir.path(), "a.rs", content());
    let out = run_stdin(dir.path(), patch(), &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(REASON), "got: {stderr}");
    assert!(stderr.contains(SUGGESTION), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, content(), "worktree must be untouched");
}

#[test]
fn spec_25_5_carries_symbol_conflict_envelope() {
    let spec = std::fs::read_to_string(workspace_root().join("SPEC.md")).unwrap();
    assert!(
        spec.contains(
            "reason: symbol `Old` could not be resolved (no semantic environment in v0.1)"
        ),
        "25.5 reason fence drifted"
    );
    assert!(
        spec.contains("suggestion: add `# fallback=syntax` to allow syntactic matching"),
        "25.5 suggestion fence drifted"
    );
    assert!(
        spec.contains("Suggestion is set in both envelopes here"),
        "25.5 envelope note drifted"
    );
    assert!(
        spec.contains("suggestion-set contrast vs the suggestion-null C14/C15 rows"),
        "C13 envelope note drifted"
    );
}
