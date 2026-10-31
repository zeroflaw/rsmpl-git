//! WI-16: crate-local `# symbol` resolution pins (SPEC 19.3).
//!
//! A bare `# symbol` hunk verifies its minus-side seeds against the
//! crate graph (file/inline modules, `use` imports, one-hop `pub use`
//! re-exports) and rewrites only the sites that resolve to the same
//! definition. Same-text vendored copies stay untouched, `std::` seeds
//! fail with the concrete-path repair, and `# fallback=syntax` keeps
//! exact-text matching with `symbol: "syntax"`.

mod common;
use common::*;

const RESOLVED_LIB: &str = "pub fn old_api(x: i32) -> i32 { x }\nfn f() {\n    let a = crate::old_api(1);\n    let b = crate::old_api(2);\n}\n";

const RESOLVED_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_symbols\n# scope=file\n# allow=many\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n";

#[test]
fn resolved_rewrites_qualified_sites_and_reports_resolved() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", RESOLVED_LIB);
    let out = run_stdin(dir.path(), RESOLVED_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("crate::new_api(1);"), "got: {got}");
    assert!(got.contains("crate::new_api(2);"), "got: {got}");
    assert!(got.contains("pub fn old_api"), "got: {got}");
}

#[test]
fn resolved_json_carries_symbol_key_and_postcondition() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", RESOLVED_LIB);
    let out = run_stdin(
        dir.path(),
        RESOLVED_PATCH,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["rules"][0]["symbol"],
        serde_json::json!("resolved"),
        "got: {v}"
    );
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(2), "got: {v}");
    assert!(
        v["rules"][0]["semantics"] == serde_json::json!("postcondition")
            || v["rules"][0]["semantics"] == serde_json::json!("instantiated"),
        "got: {v}"
    );
}

#[test]
fn vendored_same_text_copy_is_untouched() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "pub mod vendored;\npub fn old_api(x: i32) -> i32 { x }\nfn f() {\n    let a = crate::old_api(1);\n    let b = crate::vendored::old_api(2);\n}\n",
    );
    write(
        dir.path(),
        "src/vendored.rs",
        "pub fn old_api(x: i32) -> i32 { x * 10 }\n",
    );
    let out = run_stdin(dir.path(), RESOLVED_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("crate::new_api(1);"), "got: {got}");
    assert!(got.contains("crate::vendored::old_api(2);"), "got: {got}");
}

#[test]
fn bare_imported_spelling_resolves_and_matches() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "pub mod a;\npub fn old_api(x: i32) -> i32 { x }\n",
    );
    write(
        dir.path(),
        "src/a.rs",
        "use crate::old_api;\nfn f() {\n    let a = old_api(1);\n    let b = old_api(2);\n}\n",
    );
    let patch = "diff --git src/a.rs\n@@ -1,1 @@ rule=s\n# scope=file\n# allow=many\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/a.rs");
    assert!(got.contains("crate::new_api(1);"), "got: {got}");
    assert!(got.contains("crate::new_api(2);"), "got: {got}");
}

#[test]
fn std_seed_fails_with_concrete_repair() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", RESOLVED_LIB);
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# scope=file\n# allow=many\n# symbol Old=std::vec::Vec\n# symbol New=std::vec::Vec2\n# expr E\n- Old(E)\n+ New(E)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_SYMBOL_CONFLICT"),
        "got: {v}"
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({"action":"add_directive","directive":"# fallback=syntax"}),
        "got: {v}"
    );
}

#[test]
fn fallback_reports_syntax_and_rewrites() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", RESOLVED_LIB);
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# scope=file\n# allow=many\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["rules"][0]["symbol"],
        serde_json::json!("syntax"),
        "got: {v}"
    );
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(2), "got: {v}");
}
