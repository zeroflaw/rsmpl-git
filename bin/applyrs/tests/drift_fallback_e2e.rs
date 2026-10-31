//! WI-4: pattern-anchored drift for stale ranged hunks (SPEC 10.8).
//!
//! Two tiers share one `drift` evidence shape: the anchored tier (hunk body
//! context ` ` lines must agree with the single file-wide candidate) and the
//! pattern-anchored tier (a terse hunk with no context drifts on uniqueness
//! and disjointness alone). Zero or several candidates keep the original
//! `RSMPL_MATCH_CONFLICT` and apply nothing.

mod common;
use common::*;

/// 20 pad items push `fn foo` to lines 21-23; the patch cites the
/// pre-drift range 1-3 with trailing `...`/`}` context anchors.
fn stale_repo() -> tempfile::TempDir {
    let dir = init_repo();
    let mut src = String::new();
    for i in 0..20 {
        src.push_str(&format!("fn pad{i:02}() {{}}\n"));
    }
    src.push_str("fn foo() {\n    1\n}\nfn after() {}\n");
    write(dir.path(), "src/lib.rs", &src);
    dir
}

const STALE_PATCH: &str = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_foo\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";

#[test]
fn stale_range_applies_with_drift_evidence() {
    let dir = stale_repo();
    write(dir.path(), "change.rsmpl", STALE_PATCH);
    let patch = dir.path().join("change.rsmpl");
    let res = run(
        dir.path(),
        &[
            "--file",
            patch.to_str().unwrap(),
            "--format",
            "json",
            "--diff-max-lines",
            "0",
            "--check",
        ],
    );
    assert!(
        res.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    // Cited range is stale; the single anchored candidate reports both.
    assert_eq!(
        v["drift"],
        serde_json::json!([{
            "rule": "rename_foo", "file": "src/lib.rs",
            "cited": [1, 3], "actual": [21, 21],
        }]),
        "drift evidence: {}",
        String::from_utf8_lossy(&res.stdout)
    );
    assert_eq!(v["rules"][0]["matches"], 1);
    assert_eq!(v["rules"][0]["spans"], serde_json::json!([[21, 21]]));
    // Post-tree is byte-identical to a fresh-range application.
    let res = run(dir.path(), &["--file", patch.to_str().unwrap()]);
    assert!(
        res.status.success(),
        "apply stderr: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let err = String::from_utf8_lossy(&res.stderr).to_string();
    assert!(
        err.contains("drift: rename_foo src/lib.rs cited [1,3] -> actual [21,21]"),
        "text drift line: {err}"
    );
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("fn bar() {"), "got:\n{out}");
    assert!(!out.contains("fn foo()"), "got:\n{out}");
    assert!(out.contains("fn pad00() {}"), "pads moved: {out}");
    assert!(out.contains("fn after() {}"), "got:\n{out}");
}

#[test]
fn ambiguous_drift_keeps_original_conflict() {
    // Two `fn foo` definitions: the file-wide retry finds two anchored
    // candidates, so the original zero-match conflict stands and nothing
    // applies.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo() {\n    1\n}\nfn foo() {\n    2\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -5,2 @@ rule=rename_foo\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(!res.status.success(), "drift must not apply ambiguously");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_MATCH_CONFLICT");
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("no match found"),
        "original conflict stands: {v}"
    );
    assert!(v.get("drift").is_none(), "no drift evidence: {v}");
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn foo() {\n    1\n}\nfn foo() {\n    2\n}\n"
    );
}

#[test]
fn terse_stale_range_applies_with_drift_evidence() {
    // WI-4 pattern-anchored tier: terse rename, no context lines, cited
    // 1-2 while `fn foo` lives at 21-21 — uniqueness + disjointness apply
    // it with identical `drift` evidence.
    let dir = stale_repo();
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=rename_foo\n- fn foo\n+ fn bar\n";
    write(dir.path(), "change.rsmpl", patch);
    let patch_path = dir.path().join("change.rsmpl");
    let res = run(
        dir.path(),
        &[
            "--file",
            patch_path.to_str().unwrap(),
            "--format",
            "json",
            "--diff-max-lines",
            "0",
            "--check",
        ],
    );
    assert!(
        res.status.success(),
        "terse drift check failed: {} {}",
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(
        v["drift"],
        serde_json::json!([{
            "rule": "rename_foo", "file": "src/lib.rs",
            "cited": [1, 2], "actual": [21, 21],
        }]),
        "terse drift evidence: {}",
        String::from_utf8_lossy(&res.stdout)
    );
    assert_eq!(v["rules"][0]["matches"], 1);
    assert_eq!(v["rules"][0]["spans"], serde_json::json!([[21, 21]]));
    // `--check` vs apply parity: same spans, same drift, byte-identical tree.
    let res = run(dir.path(), &["--file", patch_path.to_str().unwrap()]);
    assert!(
        res.status.success(),
        "terse drift apply failed: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let err = String::from_utf8_lossy(&res.stderr).to_string();
    assert!(
        err.contains("drift: rename_foo src/lib.rs cited [1,2] -> actual [21,21]"),
        "text drift line: {err}"
    );
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("fn bar() {"), "got:\n{out}");
    assert!(!out.contains("fn foo()"), "got:\n{out}");
}

#[test]
fn terse_zero_match_keeps_original_conflict() {
    // No `foo` anywhere: the file-wide retry finds zero candidates.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn other() {}\n");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_foo\n- fn foo\n+ fn bar\n";
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(!res.status.success(), "zero-match drift must fail");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_MATCH_CONFLICT");
    assert!(v.get("drift").is_none(), "no drift evidence: {v}");
    assert_eq!(read(dir.path(), "src/lib.rs"), "fn other() {}\n");
}

#[test]
fn terse_twin_keeps_original_conflict() {
    // Two `fn foo` definitions, no anchors: two file-wide candidates, so
    // the original conflict stands and nothing applies.
    let dir = init_repo();
    let mut src = String::new();
    for i in 0..10 {
        src.push_str(&format!("fn pad{i:02}() {{}}\n"));
    }
    src.push_str("fn foo() {}\nfn foo() {}\n");
    write(dir.path(), "src/lib.rs", &src);
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_foo\n- fn foo\n+ fn bar\n";
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(!res.status.success(), "twin terse drift must fail");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_MATCH_CONFLICT");
    assert!(v.get("drift").is_none(), "no drift evidence: {v}");
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("fn pad00() {}"), "pads untouched: {out}");
    assert_eq!(
        out.matches("fn foo() {}").count(),
        2,
        "both twins stay: {out}"
    );
}

#[test]
fn terse_call_pair_drifts() {
    // Bare call pair with no context: unique `foo` call far from the cited
    // range drifts the same way renames do.
    let dir = init_repo();
    let mut src = String::new();
    for i in 0..10 {
        src.push_str(&format!("fn pad{i:02}() {{}}\n"));
    }
    src.push_str("fn main() {\n    foo(1);\n}\n");
    write(dir.path(), "src/lib.rs", &src);
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_call\n# allow=many\n- foo(E)\n+ bar(E)\n";
    // Cited 1-1 holds only a pad fn; the single `foo(1)` call lives at 12.
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    // `allow=many` ranged hunks never drift-report per SPEC 10.8, so this
    // stays a zero-match conflict: use a `allow=one` call hunk instead.
    assert!(!res.status.success(), "allow=many must not drift");
    let dir = init_repo();
    let mut src = String::new();
    for i in 0..10 {
        src.push_str(&format!("fn pad{i:02}() {{}}\n"));
    }
    src.push_str("fn main() {\n    foo(1);\n}\n");
    write(dir.path(), "src/lib.rs", &src);
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_call\n- foo(E)\n+ bar(E)\n";
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(
        res.status.success(),
        "terse call drift failed: {} {}",
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert!(v.get("drift").is_some(), "call drift evidence: {v}");
    let res = run(
        dir.path(),
        &["--file", dir.path().join("change.rsmpl").to_str().unwrap()],
    );
    assert!(
        res.status.success(),
        "call apply: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    assert!(
        read(dir.path(), "src/lib.rs").contains("bar(1)"),
        "got:\n{}",
        read(dir.path(), "src/lib.rs")
    );
}

#[test]
fn terse_let_pair_drifts() {
    // Bare `let` pair with no context drifts under the same gate.
    let dir = init_repo();
    let mut src = String::new();
    for i in 0..10 {
        src.push_str(&format!("fn pad{i:02}() {{}}\n"));
    }
    src.push_str("fn main() {\n    let x = 1;\n}\n");
    write(dir.path(), "src/lib.rs", &src);
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_let\n- let x = E;\n+ let y = E;\n";
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(
        res.status.success(),
        "terse let drift failed: {} {}",
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert!(v.get("drift").is_some(), "let drift evidence: {v}");
    let res = run(
        dir.path(),
        &["--file", dir.path().join("change.rsmpl").to_str().unwrap()],
    );
    assert!(
        res.status.success(),
        "let apply: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    assert!(
        read(dir.path(), "src/lib.rs").contains("let y = 1;"),
        "got:\n{}",
        read(dir.path(), "src/lib.rs")
    );
}

#[test]
fn anchored_disagree_keeps_original_conflict() {
    // Exactly one `foo` call exists, but the hunk's comment anchor names a
    // line far from it: anchor agreement fails, so the original conflict
    // stands (the anchored tier keeps priority over the unique-match proof).
    // Comment anchors compile on call shapes; `fn`-item anchors would not.
    let dir = init_repo();
    let mut src = String::new();
    for i in 0..10 {
        src.push_str(&format!("fn pad{i:02}() {{}}\n"));
    }
    src.push_str("fn main() {\n    foo(1);\n}\n");
    write(dir.path(), "src/lib.rs", &src);
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=rename_call\n  // stale-anchor\n- foo(E)\n+ bar(E)\n";
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(!res.status.success(), "anchor disagreement must fail");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_MATCH_CONFLICT");
    assert!(v.get("drift").is_none(), "no drift evidence: {v}");
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
}

#[test]
fn drift_overlap_fails_as_overlap() {
    // Hunk 1 terse-drifts onto the span hunk 2 rewrites: the plan-stage
    // overlap gate fires, nothing applies.
    let dir = stale_repo();
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=r1\n- fn foo\n+ fn bar\ndiff --git src/lib.rs\n@@ -21,3 @@ rule=r2\n- fn foo(...) {\n+ fn baz(...) {\n      ...\n  }\n";
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(!res.status.success(), "overlap must fail");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "GIT_HUNK_CONFLICT");
    assert!(read(dir.path(), "src/lib.rs").contains("fn foo() {"));
}

#[test]
fn optional_stale_range_never_drifts() {
    // `# optional` hunks owe zero-is-success: a stale range succeeds with
    // no `drift` key and no edits.
    let dir = stale_repo();
    let patch =
        "diff --git src/lib.rs\n@@ -1,2 @@ rule=maybe_foo\n# optional\n- fn missing\n+ fn other\n";
    write(dir.path(), "change.rsmpl", patch);
    let res = run(
        dir.path(),
        &[
            "--file",
            dir.path().join("change.rsmpl").to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(
        res.status.success(),
        "optional should succeed: {} {}",
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert!(v.get("drift").is_none(), "optional never drifts: {v}");
    assert!(read(dir.path(), "src/lib.rs").contains("fn foo() {"));
}
