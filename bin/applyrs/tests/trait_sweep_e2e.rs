//! T336: trait-method sweep (SPEC 13.4 header-advisory fan-out, impl parity).
//!
//! One hunk renames `old_name` -> `new_name` across three heterogeneous
//! `trait` blocks (plain `trait T`, generic `trait U<T: Clone>`, `trait V`
//! with a default body) via `@@ -1,1 @@` + `# scope=file` +
//! `# allow=many`. The nonsense `trait ZZZ` header proves the advisory
//! rule (header names no block, yet all three blocks rewrite — the T336
//! engine fix; before it only the first block rewrote). An `impl Svc`
//! method and a free `fn old_name` stay untouched (trait-context hunk
//! never enters `impl` blocks or free fns). Fixture `30-trait-sweep` +
//! bench `20-trait-sweep` cover the same repo through the binary/rot-gate;
//! these are the e2e pins (`matches == 3`, spans, `--check` untouched).

mod common;
use common::*;

const BEFORE: &str = "trait T {\n    fn old_name(&self) -> i32;\n}\n\ntrait U<T: Clone> {\n    fn old_name(&self) -> i32;\n}\n\ntrait V {\n    fn old_name(&self) -> i32 {\n        3\n    }\n}\n\nimpl Svc {\n    fn old_name(&self) -> i32 {\n        0\n    }\n}\n\nfn old_name() -> i32 {\n    1\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_trait\n# scope=file\n# allow=many\n# lang=rust\n  trait ZZZ {\n-     fn old_name(...);\n+     fn new_name(...);\n  }\n";

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
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(3), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[2, 2], [6, 6], [10, 10]]),
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
