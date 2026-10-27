//! T272: engine-verbatim item-rename fail-fast reasons (SPEC 14.8).
//!
//! SPEC 14.8 described these rejections in prose; the engine emits the
//! exact strings pinned here (rule + file lines in text mode, explicit
//! nulls in the JSON envelope). All six are `RSMPL_PATTERN_CONFLICT`
//! with a null suggestion; the worktree stays untouched.

mod common;
use common::*;

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
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    assert!(
        v["error"]["suggestion"].is_null(),
        "null suggestion expected, got: {v}"
    );
    assert!(v["error"]["range"].is_null(), "got: {v}");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    v
}

fn check_text(patch: &str, file: &str, content: &str, reason: &str) {
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
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(
        !stderr.contains("suggestion:"),
        "null suggestion must be omitted in text, got: {stderr}"
    );
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

#[test]
fn auto_trait_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- auto trait Old\n+ auto trait New\n";
    let reason = "`auto trait` headers are not supported in v1 (use an ordinary hunk)";
    check_text(patch, "src/lib.rs", "fn f() {}\n", reason);
    let v = check_json(patch, "src/lib.rs", "fn f() {}\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn prefix_differ_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- pub struct Old\n+ struct New\n";
    let reason = "item rename prefixes differ (`pub` vs ``); item renames are rename-only — write the same prefix on both sides or neither (visibility/generics/bodies are preserved)";
    check_text(patch, "src/lib.rs", "pub struct Old {}\n", reason);
    let v = check_json(patch, "src/lib.rs", "pub struct Old {}\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn kind_mismatch_rejected_verbatim() {
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- struct Old\n+ enum New\n";
    let reason =
        "item rename kind mismatch: minus is `struct` but plus is `enum` (kinds must match)";
    check_text(patch, "src/lib.rs", "struct Old {}\n", reason);
    let v = check_json(patch, "src/lib.rs", "struct Old {}\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn identical_names_rejected_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- struct Old\n+ struct Old\n";
    let reason =
        "item rename minus and plus names are identical (`Old`); write the new name on the `+` side";
    check_text(patch, "src/lib.rs", "struct Old {}\n", reason);
    let v = check_json(patch, "src/lib.rs", "struct Old {}\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn unsafe_prefix_illegal_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- unsafe struct Old\n+ unsafe struct New\n";
    let reason =
        "`unsafe` is only legal on `trait`/`impl` headers, not `struct` (write the bare `<struct> <Name>` form)";
    check_text(patch, "src/lib.rs", "struct Old {}\n", reason);
    let v = check_json(patch, "src/lib.rs", "struct Old {}\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn impl_visibility_illegal_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- pub impl Old\n+ pub impl New\n";
    let reason = "`impl` blocks take no visibility (write the bare `impl <Name>` form)";
    check_text(patch, "src/lib.rs", "struct Old {}\n", reason);
    let v = check_json(patch, "src/lib.rs", "struct Old {}\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}

#[test]
fn unsafe_prefix_illegal_enum_verbatim() {
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- unsafe enum Old\n+ unsafe enum New\n";
    let reason =
        "`unsafe` is only legal on `trait`/`impl` headers, not `enum` (write the bare `<enum> <Name>` form)";
    check_text(patch, "src/lib.rs", "enum Old { A }\n", reason);
    let v = check_json(patch, "src/lib.rs", "enum Old { A }\n");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
}
