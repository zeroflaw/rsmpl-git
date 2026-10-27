//! T330: pat-metavar sweep (`P` / `P: T`, SPEC 18.6 general-`let`).
//!
//! One hunk rewrites 4 ascribed `let` inits (`E` -> `E.wrapping_add(1)`)
//! while preserving every pattern (incl. `mut c`) and type. Unit coverage
//! exists (`matcher_general`); reject shapes are pinned in
//! `general_compile_verbatim_e2e.rs`; these are the binary e2e pins plus
//! the `24-pat-sweep` fixture and `15-pat-sweep` bench.

mod common;
use common::*;

const BEFORE: &str = "fn f() {\n    let a: i32 = 0;\n    let b: u32 = 1;\n    let mut c: i64 = 2;\n    let d: usize = 3;\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_pats\n# scope=file\n# allow=many\n# pat P\n# type T\n# expr E\n- let P: T = E;\n+ let P: T = E.wrapping_add(1);\n";

#[test]
fn pat_sweep_rewrites_all_four_inits_preserving_pats_and_types() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    let want = "fn f() {\n    let a: i32 = 0.wrapping_add(1);\n    let b: u32 = 1.wrapping_add(1);\n    let mut c: i64 = 2.wrapping_add(1);\n    let d: usize = 3.wrapping_add(1);\n}\n";
    assert_eq!(got, want, "patterns/types preserved, inits rewritten");
}

#[test]
fn pat_sweep_json_reports_four_matches_with_spans() {
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
        serde_json::json!([[2, 2], [3, 3], [4, 4], [5, 5]]),
        "got: {v}"
    );
}

#[test]
fn pat_sweep_check_leaves_worktree_untouched() {
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
