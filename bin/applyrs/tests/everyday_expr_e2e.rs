//! WI-2a: everyday expressions — verbatim macro, repeat, `&raw`, `const`, `_`.
//!
//! Each arm is structural, never guessing: macro is token-equal like
//! `Type::Macro`/`Stmt::Macro`; repeat/raw-addr/const recurse; `_`
//! matches anything and binds nothing. Holes inside macros reject at
//! match time in this slice (C19 compile lint is WI-2b).

mod common;
use common::*;

// --- verbatim macro ---

const MACRO_BEFORE: &str = "fn f() {\n    let b = format!(\"x{}\", 1);\n}\n";

const MACRO_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=macro_sweep\n# scope=file\n# allow=many\n- let b = format!(\"x{}\", 1);\n+ let b = format!(\"y{}\", 1);\n";

#[test]
fn verbatim_macro_fire_rewrites_identical_site() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", MACRO_BEFORE);
    let out = run_stdin(dir.path(), MACRO_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let want = "fn f() {\n    let b = format!(\"y{}\", 1);\n}\n";
    assert_eq!(read(dir.path(), "src/lib.rs"), want);
}

#[test]
fn verbatim_macro_spacing_variant_still_matches() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f() {\n    let b = format!(\"x{}\",1);\n}\n",
    );
    let out = run_stdin(dir.path(), MACRO_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let want = "fn f() {\n    let b = format!(\"y{}\", 1);\n}\n";
    assert_eq!(read(dir.path(), "src/lib.rs"), want);
}

// --- repeat ---

const REPEAT_BEFORE: &str = "fn f() {\n    let buf = [0u8; 8];\n}\n";

const REPEAT_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=repeat_grow\n# scope=file\n- let buf = [0u8; 8];\n+ let buf = [0u8; 16];\n";

#[test]
fn repeat_fire_rewrites_len() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", REPEAT_BEFORE);
    let out = run_stdin(dir.path(), REPEAT_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn f() {\n    let buf = [0u8; 16];\n}\n"
    );
}

const REPEAT_HOLE_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=repeat_hole\n# scope=file\n# expr E\n- let buf = [E; 8];\n+ let buf = [E; 16];\n";

#[test]
fn repeat_length_hole_binds_elem() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f() {\n    let buf = [7u8; 8];\n}\n",
    );
    let out = run_stdin(dir.path(), REPEAT_HOLE_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn f() {\n    let buf = [7u8; 16];\n}\n"
    );
}

// --- &raw addr ---

const RAW_BEFORE: &str = "fn f() {\n    let p = &raw const x;\n}\n";

const RAW_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=raw_mut\n# scope=file\n- let p = &raw const x;\n+ let p = &raw mut x;\n";

#[test]
fn raw_addr_fire_rewrites_mutability() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", RAW_BEFORE);
    let out = run_stdin(dir.path(), RAW_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn f() {\n    let p = &raw mut x;\n}\n"
    );
}

#[test]
fn raw_addr_mutability_veto_is_loud() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f() {\n    let p = &raw mut x;\n}\n",
    );
    let out = run_stdin(dir.path(), RAW_PATCH, &["--check"]);
    assert!(!out.status.success(), "mutability mismatch must not match");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no match found") || stderr.is_empty(),
        "got: {stderr}"
    );
    // JSON carries the machine reason.
    let out = run_stdin(
        dir.path(),
        RAW_PATCH,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(!out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
}

// --- const block ---

const CONST_BEFORE: &str = "fn f() {\n    let v = const { 1 + 2 };\n}\n";

const CONST_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=const_bump\n# scope=file\n- let v = const { 1 + 2 };\n+ let v = const { 1 + 3 };\n";

#[test]
fn const_block_fire_rewrites_inside() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", CONST_BEFORE);
    let out = run_stdin(dir.path(), CONST_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn f() {\n    let v = const { 1 + 3 };\n}\n"
    );
}

// --- infer `_` ---

const INFER_BEFORE: &str = "fn f() {\n    let v = 1 + 2;\n}\n";

const INFER_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,1 @@ rule=infer_wild\n# scope=file\n- let v = _;\n+ let v = 0;\n";

#[test]
fn infer_pattern_matches_any_expr_and_writes_literal() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", INFER_BEFORE);
    let out = run_stdin(dir.path(), INFER_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn f() {\n    let v = 0;\n}\n"
    );
}
