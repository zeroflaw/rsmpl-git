//! CLI edge cases split out of `cli_e2e.rs` (gate headroom).
//!
//! `cli_e2e.rs` sat at 1960 lines, 40 under the
//! `no_rust_file_exceeds_2000_lines` gate; this file holds its 12-test
//! tail verbatim (repeat/collection/trailing-comma/truncation/deletion/
//! over-broad/postcondition/verify) so the next CLI test addition does
//! not trip the gate. No test logic altered; `two_call_patch` is a local
//! copy of the helper that stays in `cli_e2e.rs`.

mod common;
use common::*;

#[test]
fn repeat_folds_nested_match_json_report() {
    // T20: one hunk folds `Box::new(Box::new(x))` to `x`; the report
    // counts the single outermost edit.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {\n    let v = Box::new(Box::new(x));\n}\n",
    );
    let patch =
        "diff --git src/lib.rs\n@@ -1,3 @@ rule=unbox\n# repeat\n# expr E\n- Box::new(E)\n+ E\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(1));
    assert!(v["diff"].as_str().unwrap().contains("+    let v = x;"));
}

#[test]
fn ordinary_hunk_count_mismatch_rejected() {
    // T367 (reverses T134): ordinary hunk headers must agree with the
    // body. Counts are validated at parse, so a miscounted header fails
    // fast with RSMPL_PARSE_CONFLICT instead of surfacing downstream as
    // a misleading GIT_HUNK_CONFLICT. Body is 2 context + 1 removal
    // (+ 1 addition); header claims 1,4/1,99.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n",
    );
    let mismatched =
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,99 @@\n fn a() {}\n-fn b() {}\n+fn b2() {}\n fn c() {}\n";
    let out = run_stdin(dir.path(), mismatched, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains(
            "ordinary hunk body has 3 old-side and 3 new-side lines but header claims -1,4 +1,99"
        ),
        "got: {stderr}"
    );
    assert!(
        stderr.contains("old_range: line 4"),
        "range is the hunk-header line number, got: {stderr}"
    );
    assert!(
        stderr.contains("suggestion: fix the header counts"),
        "got: {stderr}"
    );
    let out = run_stdin(dir.path(), mismatched, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert_eq!(v["error"]["range"], serde_json::json!("line 4"));
    assert!(
        v["error"]["suggestion"].is_string(),
        "suggestion must be present, got: {v}"
    );
    // Same file, correct counts, but one wrong content line: still
    // GIT_HUNK_CONFLICT with the engine-verbatim reason.
    let wrong_content =
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n fn a() {}\n-WRONG\n+fn b2() {}\n fn c() {}\n";
    let out = run_stdin(dir.path(), wrong_content, &["--check"]);
    assert!(!out.status.success(), "wrong content must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("GIT_HUNK_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("old_range: 1..4"),
        "range is start..start+len (@@ -1,3 @@ -> 1..4), got: {stderr}"
    );
    assert!(
        stderr.contains("removal mismatch at line 2: expected `WRONG`"),
        "reason must be engine-verbatim, got: {stderr}"
    );
    let out = run_stdin(dir.path(), wrong_content, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(v["error"]["code"], serde_json::json!("GIT_HUNK_CONFLICT"));
    assert_eq!(v["error"]["range"], serde_json::json!("1..4"));
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("removal mismatch"),
        "got: {v}"
    );
}

#[test]
fn repeat_with_bind_rejected_json_envelope() {
    // T20: `# repeat` on a capture hunk fails fast with PATTERN conflict.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\n");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=cut\n# repeat\n# item I\n# bind=H\n- I\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
}

#[test]
fn bind_with_allow_many_cuts_collection() {
    // WI-6: `# bind=` + `# allow=many` on an item cut binds the ordered
    // collection (SPEC 20.3) — one hunk cuts every in-scope item.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\nfn b() {}\n");
    let patch =
        "diff --git src/lib.rs\n@@ -1,2 @@ rule=cut\n# item I\n# bind=H\n# allow=many\n- I\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(2));
    assert_eq!(v["rules"][0]["spans"], serde_json::json!([[1, 1], [2, 2]]));
    assert_eq!(read(dir.path(), "src/lib.rs"), "");
}

#[test]
fn trailing_comma_where_rename_applies() {
    // T224: multi-line `where` target matches the single-line pattern
    // spelling; the clause is preserved on a pure rename.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f<T>(x: T) -> T\nwhere\n    T: Clone,\n{\n    x\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,6 @@ rule=t\n# lang=rust\n- fn f<T>(...) -> T where T: Clone {\n+ fn g<T>(...) -> T where T: Clone {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn g<T>(x: T) -> T\nwhere\n    T: Clone,\n{\n    x\n}\n"
    );
}

#[test]
fn trailing_comma_params_json_spans() {
    // T224 params half via JSON: one match, original-source spans.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f(\n    x: i32,\n    y: i32,\n) -> i32 {\n    x + y\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,6 @@ rule=t\n# lang=rust\n- fn f(x: i32, y: i32) -> i32 {\n+ fn g(x: i32, y: i32) -> i32 {\n      ...\n  }\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(1));
    assert_eq!(v["rules"][0]["spans"], serde_json::json!([[1, 1]]));
}

#[test]
fn too_many_matches_saturates_at_two() {
    // T246: four call sites under `# allow=one` fail with the saturated
    // `expected one match, found 2` wording (matchers stop counting at 2),
    // in text stderr and in the JSON envelope alike (SPEC 25.4).
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn main() {\n    foo(1);\n    foo(2);\n    foo(3);\n    foo(4);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,6 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("expected one match, found 2"),
        "got: {stderr}"
    );
    assert!(
        stderr.contains("old_range: 1..7"),
        "range is start..start+len (@@ -1,6 @@ -> 1..7), got: {stderr}"
    );
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["range"], serde_json::json!("1..7"));
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("expected one match, found 2")
    );
    // File untouched by the failed run.
    assert!(read(dir.path(), "src/lib.rs").contains("foo(4)"));
}

#[test]
fn deleted_file_is_removed_not_truncated() {
    // T350: an input section targeting `/dev/null` removes the file —
    // the lowering must carry `deleted file mode` so `git apply`
    // deletes instead of truncating to empty.
    let dir = init_repo();
    write(dir.path(), "src/old.rs", "fn old() {}\n");
    let patch = "diff --git a/src/old.rs b/src/old.rs\ndeleted file mode 100644\nindex 1111111..0000000\n--- a/src/old.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-fn old() {}\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !dir.path().join("src/old.rs").exists(),
        "deleted file must be gone, not truncated"
    );
}

#[test]
fn ordinary_all_minus_hunk_truncates_but_keeps_file() {
    // T350 twin: without `/dev/null` the same all-minus hunk truncates
    // the file and keeps it (only explicit deletions remove).
    let dir = init_repo();
    write(dir.path(), "src/old.rs", "fn old() {}\n");
    let patch = "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -1,1 +0,0 @@\n-fn old() {}\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.path().join("src/old.rs").exists(), "file must stay");
    assert!(
        read(dir.path(), "src/old.rs").is_empty(),
        "file must be empty"
    );
}

#[test]
fn json_report_shows_extra_span_of_over_broad_patch() {
    // WI-2b: an over-broad authored sweep matches 3 sites, not the 2 the
    // author aimed at — the JSON report exposes every span plus per-site
    // bindings, so the agent rejects before apply.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\nfn b() {\n    foo(3);\n}\n",
    );
    let sweep = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(
        dir.path(),
        sweep,
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
        serde_json::json!([[2, 2], [3, 3], [6, 6]]),
        "got: {v}"
    );
    // WI-2b/2c v2 per-site evidence in span order: the extra site is
    // visible with its own bindings before apply (first-site `bindings`
    // removed with `version: 2`).
    assert_eq!(
        v["rules"][0]["sites"],
        serde_json::json!([
            {"span": [2, 2], "bindings": {"E": "1"}},
            {"span": [3, 3], "bindings": {"E": "2"}},
            {"span": [6, 6], "bindings": {"E": "3"}}
        ]),
        "got: {v}"
    );
    // --check applied nothing: the agent decides from the report alone.
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
}

#[test]
fn identity_sweep_fails_postcondition_loud() {
    // WI-2c: an identity sweep (`foo(E)` -> `foo(E)`) changes no bytes —
    // the sweep change gate (SPEC 24.6) fails loud with
    // `RSMPL_MATCH_CONFLICT` instead of an empty lowering with a clean
    // report, and the worktree stays untouched.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {\n    foo(1);\n}\n");
    let sweep = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# allow=many\n# expr E\n- foo(E)\n+ foo(E)\n";
    let out = run_stdin(dir.path(), sweep, &["--format", "json", "--check"]);
    assert!(!out.status.success(), "identity sweep must fail");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["version"], serde_json::json!(3), "got: {v}");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT"),
        "got: {v}"
    );
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("postcondition"),
        "got: {v}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), "fn a() {\n    foo(1);\n}\n");
}

fn two_call_patch() -> &'static str {
    "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n"
}

#[test]
fn verify_unknown_mode_is_usage_error() {
    // WI-2: only `build` exists; anything else fails fast in both modes.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\n");
    let out = run_stdin(
        dir.path(),
        two_call_patch(),
        &["--format", "json", "--check", "--verify", "nope"],
    );
    assert!(!out.status.success(), "expected failure");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_IO_CONFLICT"),
        "got: {v}"
    );
}
