//! T286: engine-verbatim fn-slot `...` / body + item-shape + use-path tail
//! rejections (SPEC 14.7 / 14.8 / 14.9 / 25.3).
//!
//! All eleven fail at the pattern-parse stage: `RSMPL_PATTERN_CONFLICT`
//! (exit 2, `rule:`/`file:` set in text mode, explicit `range: null` /
//! `suggestion: null` in JSON, worktree untouched in both modes).
//! `{kind}`/kind/`{body}`/`{seg}` placeholders echo the offending
//! spelling (single spelling pinned each; parameterization was probed
//! with a second kind during implementation).

mod common;
use common::*;

const FN_BASE: &str = "fn foo() {}\n";
const ITEM_BASE: &str = "struct Old;\n";
const USE_BASE: &str = "use crate::old::Thing;\n";

fn check_ruled_null_envelope(base: &str, patch: &str, reason: &str) {
    let file = "src/lib.rs";
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
    assert!(stderr.contains("file: src/lib.rs"), "got: {stderr}");
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
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    assert_eq!(read(dir.path(), file), base);
}

#[test]
fn slot_return_ellipsis_rejected() {
    check_ruled_null_envelope(
        FN_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- fn foo(...) -> ... {\n+ fn bar(...) -> ... {\n      ...\n  }\n",
        "`...` is not supported as a return type in v0.1: omit `->` to match any return type or write a concrete type (SPEC 14.7)",
    );
}

#[test]
fn slot_generics_ellipsis_rejected() {
    check_ruled_null_envelope(
        FN_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- fn foo<T...>(...)\n+ fn bar<T>(...)\n  {\n      ...\n  }\n",
        "`...` is not supported in generic parameter lists in v0.1: write concrete `<T>` or abstract with `# generic_params G` + `fn foo G (...)` (SPEC 14.7)",
    );
}

#[test]
fn slot_where_ellipsis_rejected() {
    check_ruled_null_envelope(
        FN_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- fn foo(...) where ... {\n+ fn bar(...) where ... {\n      ...\n  }\n",
        "`...` is not supported in where clauses in v0.1: write concrete predicates or abstract with `# where_clause W` + `... W ...` (SPEC 14.7)",
    );
}

#[test]
fn non_ellipsis_body_rejected() {
    check_ruled_null_envelope(
        FN_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- fn foo(...) {\n+ fn foo(...) {\n      x\n  }\n",
        "v1 function hunks require `...` bodies on both sides (body evolution is not supported; the original body is always preserved)",
    );
}

#[test]
fn concrete_item_shape_rejected() {
    check_ruled_null_envelope(
        ITEM_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- struct Old<T>\n+ struct New\n",
        "concrete item shape after `<struct> <name>` is not supported in v1 (write the bare `<struct> <Name>` form; visibility, generics and bodies are preserved)",
    );
}

#[test]
fn missing_item_name_rejected() {
    check_ruled_null_envelope(
        ITEM_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- struct\n+ struct New\n",
        "missing item name after `struct` (write a bare `<kind> <Name>` pattern)",
    );
}

#[test]
fn use_mirrored_attributes_rejected() {
    check_ruled_null_envelope(
        USE_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- #[inline]\n- use crate::old\n+ #[inline]\n+ use crate::new\n",
        "mirrored attributes are not supported in `use`-path patterns in v1 (docs are preserved by default; drop the attribute lines or use an ordinary hunk)",
    );
}

#[test]
fn use_invalid_path_rejected() {
    check_ruled_null_envelope(
        USE_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::old::::bad\n+ use crate::new\n",
        "invalid `use` path `crate::old::::bad` (expected `::`-separated identifiers)",
    );
}

#[test]
fn use_invalid_segment_rejected() {
    check_ruled_null_envelope(
        USE_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use crate::Old<M>\n+ use crate::new\n",
        "invalid `use` path segment `Old<M>` (expected an identifier)",
    );
}

#[test]
fn use_missing_path_rejected() {
    check_ruled_null_envelope(
        USE_BASE,
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=many\n- use\n+ use crate::new\n",
        "missing path after `use` (write a bare `use <path>` prefix)",
    );
}

#[test]
fn concrete_item_shape_enum_rejected() {
    check_ruled_null_envelope(
        "enum Old { A }\n",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- enum Old { A }\n+ enum New { A }\n",
        "concrete item shape after `<enum> <name>` is not supported in v1 (write the bare `<enum> <Name>` form; visibility, generics and bodies are preserved)",
    );
}
