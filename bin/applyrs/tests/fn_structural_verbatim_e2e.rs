//! T301: engine-verbatim structural fn-pattern diagnostics
//! (SPEC 14.5 / 14.7 / 25.3).
//!
//! Beyond the five slot/`...` fences (SPEC 14.7) and the two
//! missing-brace needles (SPEC 14.5), eleven more structural
//! rejections an agent hits while authoring `fn` patterns were
//! prose-only. All fail fast with `RSMPL_PATTERN_CONFLICT` (exit 2,
//! `rule:` + `file:` envelope; text omits the null `range:` /
//! `suggestion:` lines, JSON carries explicit nulls; suggestion is
//! null throughout): missing `fn` keyword (with full-pattern `got`
//! echo), missing function name, missing parameter list, four
//! `unbalanced ...` delimiters, bare `->`, mixed bodies, leading
//! block comments, and the unterminated-`->` twin of the fenced
//! `where` needle. The missing-body control proves the contrast: no
//! body at all fires the already-verbatim `v1 function hunks
//! require ...` fence instead of the `exactly ...` mixed-body fence.
//! The worktree stays untouched in every failure case.

mod common;
use common::*;

const BASE: &str = "fn f() {}\n";

fn fail_both(patch: &str, file: &str, content: &str, reason: &str) {
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
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
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
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

#[test]
fn missing_fn_keyword_echoes_pattern() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- pub foo(...) {\n+ pub bar(...) {\n      ...\n  }\n";
    let reason = "fn pattern must contain `fn` keyword, got `pub foo(...) {\n     ...\n }`";
    fail_both(patch, "a.rs", BASE, reason);
}

#[test]
fn missing_function_name() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- pub fn\n+ pub fn bar\n";
    fail_both(patch, "a.rs", BASE, "fn pattern missing function name");
}

#[test]
fn missing_parameter_list() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo\n+ fn bar(...) {\n      ...\n  }\n";
    fail_both(patch, "a.rs", BASE, "fn pattern missing parameter list");
}

#[test]
fn unbalanced_generics() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo<T(...) {\n+ fn bar<T(...) {\n      ...\n  }\n";
    fail_both(patch, "a.rs", BASE, "unbalanced generics in fn pattern");
}

#[test]
fn unbalanced_parameter_list() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn f(... {\n+ fn g(... {\n      ...\n  }\n";
    fail_both(
        patch,
        "a.rs",
        BASE,
        "unbalanced parameter list in fn pattern",
    );
}

#[test]
fn unbalanced_body_block() {
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn f(...) { ...\n+ fn g(...) { ...\n";
    fail_both(patch, "a.rs", BASE, "unbalanced body block in fn pattern");
}

#[test]
fn unbalanced_attribute() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- #[derive(Debug\n- fn f(...) {\n+ #[derive(Debug\n+ fn g(...) {\n      ...\n  }\n";
    fail_both(patch, "a.rs", BASE, "unbalanced attribute in fn pattern");
}

#[test]
fn unbalanced_pub_parens() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- pub(crate fn f(...) {\n+ pub(crate fn g(...) {\n      ...\n  }\n";
    fail_both(patch, "a.rs", BASE, "unbalanced pub(...) in fn pattern");
}

#[test]
fn arrow_without_return_type() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn f(...) -> {\n+ fn g(...) -> {\n      ...\n  }\n";
    fail_both(
        patch,
        "a.rs",
        BASE,
        "`->` without return type in fn pattern",
    );
}

#[test]
fn mixed_body_must_be_exactly() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn f(...) { ..., x; }\n+ fn g(...) { ..., x; }\n";
    fail_both(
        patch,
        "a.rs",
        BASE,
        "function pattern bodies must be exactly `...` in v0.1 (body evolution is not supported; the original body is always preserved)",
    );
}

#[test]
fn missing_body_requires_ellipsis_control() {
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn f(...) -> T\n+ fn g(...) -> T\n";
    fail_both(
        patch,
        "a.rs",
        BASE,
        "v1 function hunks require `...` bodies on both sides (body evolution is not supported; the original body is always preserved)",
    );
}

#[test]
fn leading_block_comment_needle() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# scope=file\n- /** Multi\n- line. */\n- fn foo(...) {\n+ /** Multi\n+ line. */\n+ fn bar(...) {\n      ...\n  }\n";
    fail_both(
        patch,
        "a.rs",
        BASE,
        "block comments (`/* ... */`) are not supported in patterns in v1 (mirror doc comments with `#[doc = \"...\"]` or use an ordinary hunk)",
    );
}

#[test]
fn unterminated_return_type_needle() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn f(...) -> T\n-     ...\n- }\n+ fn g(...) -> T\n+     ...\n+ }\n";
    fail_both(
        patch,
        "a.rs",
        BASE,
        "fn pattern `->` return type is missing its opening `{` (or `;` for declarations): the signature swallowed the body lines — put the brace on the signature line (SPEC 14.5)",
    );
}
