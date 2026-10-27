//! T331: symbol-metavar sweep (`# symbol` + `# fallback=syntax`, SPEC 19).
//!
//! One hunk rewrites 5 qualified `crate::old_api(...)` call sites
//! (`Old(E)` -> `New(E)`) while the bare `old_api(5)` and
//! `other::old_api(6)` distractors stay untouched — pins the v0.1
//! exact-path rule positively AND negatively. `# symbol` declaration
//! coverage exists (`inert_directive_e2e.rs`); reject shapes are pinned in
//! `symbol_conflict_verbatim_e2e.rs` / `symbol_shape_verbatim_e2e.rs`;
//! these are the binary e2e pins plus the `25-symbol-sweep` fixture and
//! `16-symbol-sweep` bench.

mod common;
use common::*;

const BEFORE: &str = "fn f() {\n    let a = crate::old_api(1);\n    let b = crate::old_api(x + 1);\n    let c = crate::old_api(foo(y));\n    let d = crate::old_api(4);\n    let e = crate::old_api(z);\n    let f = old_api(5);\n    let g = other::old_api(6);\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_symbols\n# scope=file\n# allow=many\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n";

#[test]
fn symbol_sweep_rewrites_all_five_qualified_sites_preserving_distractors() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    let want = "fn f() {\n    let a = crate::new_api(1);\n    let b = crate::new_api(x + 1);\n    let c = crate::new_api(foo(y));\n    let d = crate::new_api(4);\n    let e = crate::new_api(z);\n    let f = old_api(5);\n    let g = other::old_api(6);\n}\n";
    assert_eq!(got, want, "qualified rewritten, bare/other untouched");
}

#[test]
fn symbol_sweep_json_reports_five_matches_with_spans() {
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
fn symbol_sweep_check_leaves_worktree_untouched() {
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
