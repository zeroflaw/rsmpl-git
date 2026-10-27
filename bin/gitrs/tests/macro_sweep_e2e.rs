//! T332: macro-sweep (`# macro=token_tree` verbatim rename, SPEC 21).
//!
//! One hunk rewrites 5 heterogeneous `old_mac!(...)` sites
//! (`old_mac!(...)` -> `new_mac!(...)`) while the `other_mac!(6)`
//! distractor stays untouched — pins the verbatim-only rename positively
//! AND negatively. `# macro=token_tree` declaration coverage exists
//! (`pattern_compilation.rs`); reject shapes are pinned in
//! `macro_shape_verbatim_e2e.rs`; these are the binary e2e pins plus the
//! `26-macro-sweep` fixture and `17-macro-sweep` bench.

mod common;
use common::*;

const BEFORE: &str = "fn f() {\n    old_mac!(1);\n    old_mac!(x + 1);\n    old_mac!(foo(y));\n    old_mac!(4);\n    old_mac!(z);\n    other_mac!(6);\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_macros\n# scope=file\n# allow=many\n# macro=token_tree\n- old_mac!(...)\n+ new_mac!(...)\n";

#[test]
fn macro_sweep_rewrites_all_five_sites_preserving_distractor() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    let want = "fn f() {\n    new_mac!(1);\n    new_mac!(x + 1);\n    new_mac!(foo(y));\n    new_mac!(4);\n    new_mac!(z);\n    other_mac!(6);\n}\n";
    assert_eq!(got, want, "macros rewritten, other_mac untouched");
}

#[test]
fn macro_sweep_json_reports_five_matches_with_spans() {
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
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(5), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[2, 2], [3, 3], [4, 4], [5, 5], [6, 6]]),
        "got: {v}"
    );
}

#[test]
fn macro_sweep_check_leaves_worktree_untouched() {
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
