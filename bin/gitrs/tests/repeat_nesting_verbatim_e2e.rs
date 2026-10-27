//! T304: engine-verbatim `# repeat` nesting-backstop fence (SPEC 17.8).
//!
//! Past the shape gates, pathological nesting fails at match time with
//! `RSMPL_MATCH_CONFLICT` (rule + file, null range, set suggestion).
//! Both the call matcher and the general-expression matcher emit the
//! identical pair. Probed: 65-deep folds clean, 70-deep fails — pins use
//! 70-deep (fail, text + JSON + untouched worktree) plus a shallow fold
//! control, never the exact trip line.

mod common;
use common::*;

const PATCH: &str =
    "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n# repeat\n# expr E\n- Box::new(E)\n+ E\n";
const REASON: &str = "repeat nesting exceeds 64 levels (pathological nesting?)";
const SUGGESTION: &str = "remove `# repeat` or simplify the nesting";

fn deep_source(depth: usize) -> String {
    let mut s = String::from("fn foo() { ");
    for _ in 0..depth {
        s.push_str("Box::new(");
    }
    s.push('x');
    for _ in 0..depth {
        s.push(')');
    }
    s.push_str(" }\n");
    s
}

#[test]
fn repeat_nesting_backstop_verbatim_text() {
    let source = deep_source(70);
    let dir = init_repo();
    write(dir.path(), "a.rs", &source);
    let out = run_stdin(dir.path(), PATCH, &["--check"]);
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
    assert!(stderr.contains(REASON), "got: {stderr}");
    assert!(stderr.contains(SUGGESTION), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, source, "worktree must be untouched");
}

#[test]
fn repeat_nesting_backstop_verbatim_json() {
    let source = deep_source(70);
    let dir = init_repo();
    write(dir.path(), "a.rs", &source);
    let out = run_stdin(dir.path(), PATCH, &["--format", "json"]);
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
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(REASON));
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!(SUGGESTION),
        "got: {v}"
    );
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, source, "worktree must be untouched");
}

#[test]
fn shallow_nesting_still_folds() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn foo() { Box::new(Box::new(x)) }\n");
    let out = run_stdin(dir.path(), PATCH, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("check:"), "got: {stderr}");
}
