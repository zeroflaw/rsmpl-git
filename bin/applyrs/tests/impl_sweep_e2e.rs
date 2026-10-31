//! WI-4a: impl-method sweep with verbatim block headers (SPEC 13.4).
//!
//! The header is a filter in every scope: one hunk renames
//! `old_name` -> `new_name` in exactly the named block. Four hunks
//! (plain `impl A`, generic+where `impl<T: Clone> B<T>`, plain
//! `impl C`, trait-impl `impl crate::T for D`) cover four
//! heterogeneous blocks via `@@ -1,1 @@` + `# scope=file` +
//! `# allow=many`. A `trait T` `;` decl and a free `fn old_name` stay
//! untouched (body-pair hunk matches bodies only, and free fns sit in
//! no `impl` block). A nonsense `impl Svc` header matches nothing.
//! Fixture `29-impl-sweep` + bench `19-impl-sweep` cover the same repo
//! through the binary/rot-gate; these are the e2e pins (`matches` sum
//! 4, per-hunk spans, `--check` untouched).

mod common;
use common::*;

const BEFORE: &str = "struct A;\nstruct B<T>(T);\nstruct C;\nstruct D;\n\ntrait T {\n    fn old_name(&self) -> i32;\n}\n\nimpl A {\n    fn old_name(&self) -> i32 {\n        1\n    }\n}\n\nimpl<T: Clone> B<T> {\n    fn old_name(&self) -> i32 {\n        2\n    }\n}\n\nimpl C {\n    fn old_name(&self) -> i32 {\n        3\n    }\n}\n\nimpl crate::T for D {\n    fn old_name(&self) -> i32 {\n        4\n    }\n}\n\nfn old_name() -> i32 {\n    0\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_impl\n# scope=file\n# allow=many\n# lang=rust\n  impl A {\n      ...\n-     fn old_name(...) {\n+     fn new_name(...) {\n          ...\n      }\n  }\n@@ -1,1 @@ rule=sweep_impl\n# scope=file\n# allow=many\n# lang=rust\n  impl<T: Clone> B<T> {\n      ...\n-     fn old_name(...) {\n+     fn new_name(...) {\n          ...\n      }\n  }\n@@ -1,1 @@ rule=sweep_impl\n# scope=file\n# allow=many\n# lang=rust\n  impl C {\n      ...\n-     fn old_name(...) {\n+     fn new_name(...) {\n          ...\n      }\n  }\n@@ -1,1 @@ rule=sweep_impl\n# scope=file\n# allow=many\n# lang=rust\n  impl crate::T for D {\n      ...\n-     fn old_name(...) {\n+     fn new_name(...) {\n          ...\n      }\n  }\n";

const NONSENSE_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_impl\n# scope=file\n# allow=many\n# lang=rust\n  impl Svc {\n      ...\n-     fn old_name(...) {\n+     fn new_name(...) {\n          ...\n      }\n  }\n";

#[test]
fn impl_sweep_renames_all_four_methods_leaving_distractors() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert_eq!(
        got.matches("fn new_name(&self) -> i32 {").count(),
        4,
        "got: {got}"
    );
    assert!(
        got.contains("trait T {\n    fn old_name(&self) -> i32;\n}"),
        "got: {got}"
    );
    assert!(
        got.contains("fn old_name() -> i32 {\n    0\n}"),
        "got: {got}"
    );
    assert!(
        got.contains("impl crate::T for D {\n    fn new_name(&self) -> i32 {"),
        "got: {got}"
    );
    assert!(
        got.contains("impl<T: Clone> B<T> {\n    fn new_name(&self) -> i32 {"),
        "got: {got}"
    );
}

#[test]
fn nonsense_header_rewrites_nothing() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), NONSENSE_PATCH, &["--format", "json", "--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), BEFORE, "untouched");
}

#[test]
fn impl_sweep_json_reports_four_matches_with_spans() {
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
    let total: i64 = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["matches"].as_i64().unwrap())
        .sum();
    assert_eq!(total, 4, "got: {v}");
    let mut spans: Vec<serde_json::Value> = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["spans"].as_array().unwrap().clone())
        .collect();
    spans.sort_by_key(|s| s[0].as_i64().unwrap());
    assert_eq!(
        spans,
        vec![
            serde_json::json!([11, 11]),
            serde_json::json!([17, 17]),
            serde_json::json!([23, 23]),
            serde_json::json!([29, 29])
        ],
        "got: {v}"
    );
}

#[test]
fn impl_sweep_check_leaves_worktree_untouched() {
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
