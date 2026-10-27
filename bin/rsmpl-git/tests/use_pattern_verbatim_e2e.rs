//! T285: engine-verbatim `use`-pattern / missing-brace / attrs-merge rejections
//! (SPEC 14.9 / 14.5 / 14.6 / 25.3).
//!
//! Use-pattern family (7 reasons) + missing-brace pair (2 reasons):
//! pattern-parse stage, `rule:`/`file:` set in text mode, explicit
//! `range: null` / `suggestion: null` in JSON. Attrs-merge pair
//! (2 reasons): same envelope but with a SET suggestion (text carries
//! the `suggestion:` line, JSON carries the string). All exit 2 with
//! the worktree untouched.

mod common;
use common::*;

const USE_BASE: &str = "use crate::old::Thing;\n";
const FN_BASE: &str = "fn foo() {}\n";
const ATTR_BASE: &str = "#[inline]\nfn foo() {}\n";

fn check_text_ruled_envelope(base: &str, file: &str, patch: &str, reason: &str) {
    let dir = init_repo();
    write(dir.path(), file, base);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: x"), "got: {stderr}");
    assert!(stderr.contains(&format!("file: {file}")), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    assert_eq!(read(dir.path(), file), base);
    let dir = init_repo();
    write(dir.path(), file, base);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("x"));
    assert_eq!(v["error"]["file"], serde_json::json!(file));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    assert_eq!(read(dir.path(), file), base);
}

fn check_text_ruled_suggestion(
    base: &str,
    file: &str,
    patch: &str,
    reason: &str,
    suggestion: &str,
) {
    let dir = init_repo();
    write(dir.path(), file, base);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: x"), "got: {stderr}");
    assert!(stderr.contains(&format!("file: {file}")), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(
        stderr.contains(&format!("suggestion: {suggestion}")),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), file), base);
    let dir = init_repo();
    write(dir.path(), file, base);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("x"));
    assert_eq!(v["error"]["file"], serde_json::json!(file));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert_eq!(v["error"]["suggestion"], serde_json::json!(suggestion));
    assert_eq!(read(dir.path(), file), base);
}

#[test]
fn use_grouped_glob_tail_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::old::{A}\n+ use crate::new::{A}\n";
    let reason = "grouped (`::{...}`) and glob (`::*`) tails are not supported in the `use` *pattern* in v1 (write the bare shared prefix; target items may still contain groups/globs)";
    check_text_ruled_envelope(USE_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn use_glob_tail_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::old::*\n+ use crate::new\n";
    let reason = "grouped (`::{...}`) and glob (`::*`) tails are not supported in the `use` *pattern* in v1 (write the bare shared prefix; target items may still contain groups/globs)";
    check_text_ruled_envelope(USE_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn use_as_tail_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::old::Thing as W\n+ use crate::new\n";
    let reason =
        "`as` renames are not supported in the `use` *pattern* in v1 (write the bare prefix; target `as` tails are preserved)";
    check_text_ruled_envelope(USE_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn use_ellipsis_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::old::...\n+ use crate::new\n";
    let reason = "v1 `use`-path patterns do not support `...` (write the concrete prefix path)";
    check_text_ruled_envelope(USE_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn use_sigiled_meta_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::old::$M\n+ use crate::new\n";
    let reason =
        "`$Meta` abstracted names are not supported for `use`-path renames in v1 (write concrete paths)";
    check_text_ruled_envelope(USE_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn use_backtick_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::old::`E`\n+ use crate::new\n";
    let reason =
        "backtick literal quoting is not supported in `use`-path patterns in v1 (write the concrete path)";
    check_text_ruled_envelope(USE_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn use_metavariable_segment_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n# expr M\n- use crate::M\n+ use crate::new\n";
    let reason =
        "metavariable `M` is not supported in `use`-path patterns in v1 (write the concrete path)";
    check_text_ruled_envelope(USE_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn use_bang_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::old!\n+ use crate::new\n";
    let reason =
        "`use!` is a macro invocation, not a `use`-path rename (remove the `!` or use a macro pattern)";
    check_text_ruled_envelope(USE_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn fn_missing_brace_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- fn foo(...)\n+ fn bar(...)\n      ...\n";
    let reason = "unexpected trailing tokens in fn pattern: `...`";
    check_text_ruled_envelope(FN_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn fn_where_missing_brace_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- fn foo<T>(...) where T: Clone\n+ fn bar<T>(...) where T: Clone\n      ...\n";
    let reason = "fn pattern `where` clause is missing its opening `{` (or `;` for declarations): the signature swallowed the body lines — put the brace on the signature line (SPEC 14.5)";
    check_text_ruled_envelope(FN_BASE, "src/lib.rs", patch, reason);
}

#[test]
fn attrs_plus_adds_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n- fn foo(...) {\n+ #[inline]\n+ fn bar(...) {\n      ...\n  }\n";
    let reason = "replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them";
    let suggestion = "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`";
    check_text_ruled_suggestion(ATTR_BASE, "src/lib.rs", patch, reason, suggestion);
}

#[test]
fn attrs_plus_drops_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n- #[inline]\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let reason =
        "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges";
    let suggestion = "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`";
    check_text_ruled_suggestion(ATTR_BASE, "src/lib.rs", patch, reason, suggestion);
}
