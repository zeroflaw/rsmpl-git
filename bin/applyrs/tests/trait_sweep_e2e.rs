//! WI-4a: trait-method sweep with verbatim block headers (SPEC 13.4).
//!
//! One hunk per block renames `old_name` -> `new_name` across three
//! heterogeneous `trait` blocks (plain `trait T`, generic
//! `trait U<T: Clone>`, `trait V` with a default body) via
//! `@@ -1,1 @@` + `# scope=file` + `# allow=many`. An `impl Svc`
//! method and a free `fn old_name` stay untouched (trait-context hunk
//! never enters `impl` blocks or free fns). The nonsense `trait ZZZ`
//! header matches nothing by construction. Fixture `30-trait-sweep` +
//! bench `20-trait-sweep` cover the same repo through the
//! binary/rot-gate; these are the e2e pins (matches sum 3, spans,
//! `--check` untouched).

mod common;
use common::*;

const BEFORE: &str = "trait T {\n    fn old_name(&self) -> i32;\n}\n\ntrait U<T: Clone> {\n    fn old_name(&self) -> i32;\n}\n\ntrait V {\n    fn old_name(&self) -> i32 {\n        3\n    }\n}\n\nimpl Svc {\n    fn old_name(&self) -> i32 {\n        0\n    }\n}\n\nfn old_name() -> i32 {\n    1\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_trait\n# scope=file\n# allow=many\n# lang=rust\n  trait T {\n-     fn old_name(...);\n+     fn new_name(...);\n  }\n@@ -1,1 @@ rule=sweep_trait\n# scope=file\n# allow=many\n# lang=rust\n  trait U<T: Clone> {\n-     fn old_name(...);\n+     fn new_name(...);\n  }\n@@ -1,1 @@ rule=sweep_trait\n# scope=file\n# allow=many\n# lang=rust\n  trait V {\n-     fn old_name(...);\n+     fn new_name(...);\n  }\n";

const NONSENSE_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_trait\n# scope=file\n# allow=many\n# lang=rust\n  trait ZZZ {\n-     fn old_name(...);\n+     fn new_name(...);\n  }\n";

#[test]
fn trait_sweep_renames_all_three_methods_leaving_distractors() {
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
        got.matches("fn new_name(&self) -> i32").count(),
        3,
        "got: {got}"
    );
    assert!(
        got.contains("impl Svc {\n    fn old_name(&self) -> i32 {"),
        "impl distractor must be untouched, got: {got}"
    );
    assert!(
        got.contains("fn old_name() -> i32 {\n    1\n}"),
        "free-fn distractor must be untouched, got: {got}"
    );
}

#[test]
fn trait_nonsense_header_rewrites_nothing() {
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
fn trait_sweep_json_reports_three_matches_with_spans() {
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
    assert_eq!(total, 3, "got: {v}");
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
            serde_json::json!([2, 2]),
            serde_json::json!([6, 6]),
            serde_json::json!([10, 10])
        ],
        "got: {v}"
    );
}

#[test]
fn trait_sweep_check_leaves_worktree_untouched() {
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
