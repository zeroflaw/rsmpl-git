//! T278: engine-verbatim bad-directive-*value* reasons (SPEC 9.6).
//!
//! SPEC 9.6/25.3 documented unknown *directives* but not bad *values* for
//! known keys. All of these fail `RSMPL_PATTERN_CONFLICT` (exit 2) with a
//! bare `reason:` line in text mode (null rule/file/range/suggestion lines
//! omitted) and an all-null envelope in JSON. The engine's own wording is
//! recorded verbatim: `allow`/`attrs`/`optional` say "value", the rest
//! don't; only `allow` carries `(want one|many)`, only `returns` carries
//! `(only `none` supported)`.

mod common;
use common::*;

fn check_both(patch: &str, file: &str, content: &str) -> (String, serde_json::Value) {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    for absent in ["rule:", "file:", "old_range:", "suggestion:"] {
        assert!(
            !stderr.contains(absent),
            "null `{absent}` must be omitted from text, got: {stderr}"
        );
    }
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
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
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    for field in ["rule", "file", "range", "suggestion"] {
        assert_eq!(v["error"][field], serde_json::Value::Null, "got: {v}");
    }
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    (stderr, v)
}

fn fn_patch(directive: &str) -> String {
    format!(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# {directive}\n# lang=rust\n- fn foo(...) {{\n+ fn foo2(...) {{\n      ...\n  }}\n"
    )
}

#[test]
fn lang_invalid_value_verbatim() {
    let patch = fn_patch("lang=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("unsupported language `bogus` (only `rust` supported)"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("unsupported language `bogus` (only `rust` supported): # lang=bogus"),
        "got: {v}"
    );
}

#[test]
fn scope_invalid_value_verbatim() {
    let patch = fn_patch("scope=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid scope `bogus`: # scope=bogus"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid scope `bogus`: # scope=bogus"),
        "got: {v}"
    );
}

#[test]
fn allow_invalid_value_verbatim() {
    let patch = fn_patch("allow=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid allow value `bogus` (want one|many)"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid allow value `bogus` (want one|many): # allow=bogus"),
        "got: {v}"
    );
}

#[test]
fn allow_empty_value_verbatim() {
    let patch = fn_patch("allow=");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid allow value `` (want one|many): # allow="),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid allow value `` (want one|many): # allow="),
        "got: {v}"
    );
}

#[test]
fn optional_invalid_value_verbatim() {
    let patch = fn_patch("optional=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid optional value `bogus`: # optional=bogus"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid optional value `bogus`: # optional=bogus"),
        "got: {v}"
    );
}

#[test]
fn attrs_invalid_value_verbatim() {
    let patch = fn_patch("attrs=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid attrs value `bogus`: # attrs=bogus"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid attrs value `bogus`: # attrs=bogus"),
        "got: {v}"
    );
}

#[test]
fn mode_invalid_value_verbatim() {
    let patch = fn_patch("mode=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid mode `bogus`: # mode=bogus"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid mode `bogus`: # mode=bogus"),
        "got: {v}"
    );
}

#[test]
fn macro_invalid_value_verbatim() {
    let patch = fn_patch("macro=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid macro mode `bogus`: # macro=bogus"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid macro mode `bogus`: # macro=bogus"),
        "got: {v}"
    );
}

#[test]
fn format_invalid_value_verbatim() {
    let patch = fn_patch("format=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid format `bogus`: # format=bogus"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid format `bogus`: # format=bogus"),
        "got: {v}"
    );
}

#[test]
fn fallback_invalid_value_verbatim() {
    let patch = fn_patch("fallback=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid fallback `bogus`: # fallback=bogus"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid fallback `bogus`: # fallback=bogus"),
        "got: {v}"
    );
}

#[test]
fn returns_invalid_value_verbatim() {
    let patch = fn_patch("returns=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid returns `bogus` (only `none` supported)"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid returns `bogus` (only `none` supported): # returns=bogus"),
        "got: {v}"
    );
}

#[test]
fn metavars_invalid_value_verbatim() {
    let patch = fn_patch("metavars=bogus");
    let (stderr, v) = check_both(&patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid metavars `bogus`: # metavars=bogus"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("invalid metavars `bogus`: # metavars=bogus"),
        "got: {v}"
    );
}

#[test]
fn symbol_without_equals_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# symbol JustName\n# fallback=syntax\n# lang=rust\n- fn foo(...) {\n+ fn foo2(...) {\n      ...\n  }\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn foo() {}\n");
    assert!(
        stderr.contains("invalid symbol directive (want `# symbol Name=path`)"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!(
            "invalid symbol directive (want `# symbol Name=path`): # symbol JustName"
        ),
        "got: {v}"
    );
}

#[test]
fn bind_empty_name_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# bind=\n# item I\n- I\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn a() {}\n");
    assert!(
        stderr.contains("empty metavariable/capture name"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("empty metavariable/capture name"),
        "got: {v}"
    );
}

#[test]
fn use_empty_name_verbatim() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# use=\n# item I\n# bind=H\n- I\n";
    let (stderr, v) = check_both(patch, "a.rs", "fn a() {}\n");
    assert!(
        stderr.contains("empty metavariable/capture name"),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("empty metavariable/capture name"),
        "got: {v}"
    );
}
