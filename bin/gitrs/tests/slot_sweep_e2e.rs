//! T329: slot-metavar sweep (`V`/`Q`/`G`/`W`, SPEC 14.3/14.4/14.5).
//!
//! One hunk renames `foo` -> `bar` across four heterogeneous signatures
//! (plain / `async` / `pub` / `pub(crate) async` + generics + `where`)
//! while preserving every slot and never restating bodies. Unit coverage
//! exists (`matcher_fn`, `pattern_compilation`); these are the binary e2e
//! pins plus the `23-slot-sweep` fixture and `14-slot-sweep` bench.

mod common;
use common::*;

const BEFORE: &str = "fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    let w = z - 3;\n    w\n}\n\nasync fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    let w = z - 3;\n    w\n}\n\npub fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    let w = z - 3;\n    w\n}\n\npub(crate) async fn foo<T>(x: T) -> T where T: Clone {\n    let y = x.clone();\n    let z = y.clone();\n    let w = z.clone();\n    w\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_slots\n# scope=file\n# allow=many\n# visibility V\n# qualifier_list Q\n# generic_params G\n# where_clause W\n- V Q fn foo G (...) W {\n+ V Q fn bar G (...) W {\n      ...\n  }\n";

#[test]
fn slot_sweep_renames_all_four_preserving_slots() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    let want = BEFORE.replace("fn foo", "fn bar");
    assert_eq!(got, want, "slots/bodies must be preserved, names renamed");
    assert!(got.contains("async fn bar(x: i32) -> i32 {"), "got: {got}");
    assert!(got.contains("pub fn bar(x: i32) -> i32 {"), "got: {got}");
    assert!(
        got.contains("pub(crate) async fn bar<T>(x: T) -> T where T: Clone {"),
        "got: {got}"
    );
}

#[test]
fn slot_sweep_json_reports_four_matches_with_spans() {
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
        serde_json::json!([[1, 1], [8, 8], [15, 15], [22, 22]]),
        "got: {v}"
    );
}

#[test]
fn slot_sweep_check_leaves_worktree_untouched() {
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
