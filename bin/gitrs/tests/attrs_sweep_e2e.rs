//! T334: attrs-sweep (SPEC 14.6 preserve across fan-out).
//!
//! One hunk renames `foo` -> `baz` across four heterogeneous signatures
//! (plain / `async` / `pub` / `pub(crate) async` + generics + `where`),
//! each carrying a distinct attribute (`#[inline]`, `/// Doc.`,
//! `#[allow(dead_code)]`, `#[must_use]`), while a `bar` distractor stays
//! untouched. Slot fan-out coverage exists (`slot_sweep_e2e.rs`); single-site
//! attrs coverage exists (`attrs_constraint_e2e.rs`); these are the binary
//! e2e pins plus the `28-attrs-sweep` fixture and `18-attrs-sweep` bench.

mod common;
use common::*;

const BEFORE: &str = "#[inline]\nfn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    let w = z - 3;\n    w\n}\n\n/// Doc comment.\nasync fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    let w = z - 3;\n    w\n}\n\n#[allow(dead_code)]\npub fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    let w = z - 3;\n    w\n}\n\n#[must_use]\npub(crate) async fn foo<T>(x: T) -> T where T: Clone {\n    let y = x.clone();\n    let z = y.clone();\n    let w = z.clone();\n    w\n}\n\n#[inline]\npub fn bar() {}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_attrs\n# scope=file\n# allow=many\n# visibility V\n# qualifier_list Q\n# generic_params G\n# where_clause W\n- V Q fn foo G (...) W {\n+ V Q fn baz G (...) W {\n      ...\n  }\n";

#[test]
fn attrs_sweep_renames_all_four_preserving_attrs() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    let want = BEFORE.replace("fn foo", "fn baz");
    assert_eq!(got, want, "names renamed, attrs/slots/bodies preserved");
    assert!(
        got.contains("#[inline]\nfn baz(x: i32) -> i32 {"),
        "got: {got}"
    );
    assert!(
        got.contains("/// Doc comment.\nasync fn baz(x: i32) -> i32 {"),
        "got: {got}"
    );
    assert!(
        got.contains("#[allow(dead_code)]\npub fn baz(x: i32) -> i32 {"),
        "got: {got}"
    );
    assert!(
        got.contains("#[must_use]\npub(crate) async fn baz<T>(x: T) -> T where T: Clone {"),
        "got: {got}"
    );
    assert!(got.contains("#[inline]\npub fn bar() {}"), "got: {got}");
}

#[test]
fn attrs_sweep_json_reports_four_matches_with_spans() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(
        dir.path(),
        PATCH,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(4), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[2, 2], [10, 10], [18, 18], [26, 26]]),
        "got: {v}"
    );
}

#[test]
fn attrs_sweep_check_leaves_worktree_untouched() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), PATCH, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), BEFORE, "untouched");
}
