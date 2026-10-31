//! WI-10b: anchor fallback for stale ranged hunks (SPEC 10.8).
//!
//! A ranged hunk authored against shifted line numbers retries once
//! file-scoped, anchored by its body context (` `) lines. Exactly one
//! anchored candidate applies with `drift` evidence (JSON `drift` +
//! text `drift:` line); zero or several candidates keep the original
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
fn anchorless_hunk_keeps_original_conflict() {
    // Terse rename, no context lines: no anchors, so no retry — the
    // original conflict stands with the `# scope=file` suggestion.
    let dir = stale_repo();
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=rename_foo\n- fn foo\n+ fn bar\n";
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
    assert!(!res.status.success(), "anchorless drift must not apply");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_MATCH_CONFLICT");
    assert!(v.get("drift").is_none(), "no drift evidence: {v}");
    assert!(
        read(dir.path(), "src/lib.rs").contains("fn foo() {"),
        "file untouched"
    );
}
