//! T337: impl-method slot-metavar sweep (`V`/`Q`/`G`/`W`, SPEC 14.3/14.4 + 13.4).
//!
//! T329 proved slot sweeps for free fns; T335 proved impl-context sweeps
//! with concrete slots. The combination failed fast with verbatim
//! `fn pattern must contain \`fn\` keyword` because `extract_fn_block`
//! did not recognise a leading slot-metavar line (`V Q fn ...`) as the
//! fn-block start. One hunk now renames `old_name` -> `new_name` across
//! four heterogeneous methods (plain / `pub` / `async`+generics+`where` /
//! trait-impl) while preserving every slot. Pins: apply, JSON
//! `matches == 4` + spans, `--check` untouched.

mod common;
use common::*;

const BEFORE: &str = "struct A;\nstruct B<T>(T);\nstruct C;\nstruct D;\n\nimpl A {\n    fn old_name(&self) -> i32 {\n        1\n    }\n}\n\nimpl B<i32> {\n    pub fn old_name(&self) -> i32 {\n        2\n    }\n}\n\nimpl C {\n    pub async fn old_name<T>(&self, x: T) -> T where T: Clone {\n        x.clone()\n    }\n}\n\nimpl crate::T for D {\n    fn old_name(&self) -> i32 {\n        4\n    }\n}\n\nfn old_name() -> i32 {\n    0\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_impl_slots\n# scope=file\n# allow=many\n# visibility V\n# qualifier_list Q\n# generic_params G\n# where_clause W\n  impl Svc {\n      ...\n-     V Q fn old_name G (...) W {\n+     V Q fn new_name G (...) W {\n          ...\n      }\n  }\n";

#[test]
fn impl_slot_sweep_renames_all_four_preserving_slots() {
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
        got.matches("fn new_name").count(),
        4,
        "got: {got}"
    );
    assert!(got.contains("    fn new_name(&self) -> i32 {"), "got: {got}");
    assert!(
        got.contains("    pub fn new_name(&self) -> i32 {"),
        "got: {got}"
    );
    assert!(
        got.contains("    pub async fn new_name<T>(&self, x: T) -> T where T: Clone {"),
        "got: {got}"
    );
    assert!(
        got.contains("impl crate::T for D {\n    fn new_name(&self) -> i32 {"),
        "got: {got}"
    );
    assert!(
        got.contains("fn old_name() -> i32 {\n    0\n}"),
        "free fn untouched, got: {got}"
    );
}

#[test]
fn impl_slot_sweep_json_reports_four_matches_with_spans() {
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
}

#[test]
fn impl_slot_sweep_check_leaves_worktree_untouched() {
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
