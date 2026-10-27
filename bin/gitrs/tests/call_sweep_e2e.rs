//! T333: call-sweep (bench 13 symmetry, SPEC 17).
//!
//! One hunk rewrites 20 `foo(N)` sites (`foo(E)` -> `bar(E)`, two per
//! line over 10 fns) — pins the multi-match fan-out positively plus the
//! paired per-line spans. `# scope=file` + `# allow=many` declaration
//! coverage exists (`scope_modes_e2e.rs`); the `25-symbol-sweep` /
//! `26-macro-sweep` fixtures pin the sibling sweeps; this is the binary
//! e2e pin plus the `27-call-sweep` fixture.

mod common;
use common::*;

const BEFORE: &str = "fn a() -> i32 {\n    foo(1) + foo(2)\n}\n\nfn b() -> i32 {\n    foo(3) + foo(4)\n}\n\nfn c() -> i32 {\n    foo(5) + foo(6)\n}\n\nfn d() -> i32 {\n    foo(7) + foo(8)\n}\n\nfn e() -> i32 {\n    foo(9) + foo(10)\n}\n\nfn f() -> i32 {\n    foo(11) + foo(12)\n}\n\nfn g() -> i32 {\n    foo(13) + foo(14)\n}\n\nfn h() -> i32 {\n    foo(15) + foo(16)\n}\n\nfn i() -> i32 {\n    foo(17) + foo(18)\n}\n\nfn j() -> i32 {\n    foo(19) + foo(20)\n}\n";

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_calls\n# scope=file\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";

#[test]
fn call_sweep_rewrites_all_twenty_sites() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE);
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    let want = BEFORE.replace("foo(", "bar(");
    assert_eq!(got, want, "all twenty call sites rewritten");
}

#[test]
fn call_sweep_json_reports_twenty_matches_with_paired_spans() {
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
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(20), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([
            [2, 2],
            [2, 2],
            [6, 6],
            [6, 6],
            [10, 10],
            [10, 10],
            [14, 14],
            [14, 14],
            [18, 18],
            [18, 18],
            [22, 22],
            [22, 22],
            [26, 26],
            [26, 26],
            [30, 30],
            [30, 30],
            [34, 34],
            [34, 34],
            [38, 38],
            [38, 38]
        ]),
        "got: {v}"
    );
}

#[test]
fn call_sweep_check_leaves_worktree_untouched() {
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
