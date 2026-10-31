//! WI-4a: impl/trait-context header values are filters, not advisory.
//!
//! SPEC 13.4: the engine parses the header once and compares it
//! structurally against each candidate block — only the named block
//! matches, in every scope including `# scope=file`. A mismatched or
//! nonsense header matches nothing (loud zero-match, tree untouched).
//! Sweeping K blocks takes K hunks, one header per block.

mod common;
use common::*;

const IMPL_SOURCE: &str =
    "struct A;\nstruct B;\nimpl A {\n    fn old(&self) {}\n}\nimpl B {\n    fn old(&self) {}\n}";

#[test]
fn impl_header_mismatch_matches_nothing() {
    // `impl A` header + range over `impl B`: strict filter, no
    // range-overlap fallback — zero matches, tree untouched.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", IMPL_SOURCE);
    let patch = "diff --git src/lib.rs\n@@ -6,3 @@ rule=impl_mismatch\n# lang=rust\n  impl A {\n-     fn old(...) {\n+     fn new(...) {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), IMPL_SOURCE);
}

#[test]
fn trait_header_mismatch_matches_nothing() {
    let dir = init_repo();
    let src = "trait T {\n    fn old(&self);\n}\ntrait U {\n    fn old(&self);\n}\n";
    write(dir.path(), "src/lib.rs", src);
    let patch = "diff --git src/lib.rs\n@@ -4,3 @@ rule=trait_mismatch\n# lang=rust\n  trait T {\n-     fn old(...);\n+     fn new(...);\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), src);
}

#[test]
fn nonsense_header_matches_nothing() {
    // A header naming no block is a loud zero-match, never a sweep.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", IMPL_SOURCE);
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep\n# scope=file\n# allow=many\n  impl ZZZ {\n-     fn old(...) {\n+     fn new(...) {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), IMPL_SOURCE);
}

const DISJOINT_SOURCE: &str = "trait T {\n    fn old(&self);\n}\nfn unrelated() {}\n";
const DISJOINT_PATCH: &str = "diff --git src/lib.rs\n@@ -4,1 @@ rule=disjoint\n# lang=rust\n  trait T {\n-     fn old(...);\n+     fn new(...);\n  }\n";

#[test]
fn disjoint_range_fails_no_match_text() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", DISJOINT_SOURCE);
    let out = run_stdin(dir.path(), DISJOINT_PATCH, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("no match found"), "got: {stderr}");
    assert!(stderr.contains("# scope=file"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), DISJOINT_SOURCE);
}

#[test]
fn disjoint_range_fails_no_match_json() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", DISJOINT_SOURCE);
    let out = run_stdin(dir.path(), DISJOINT_PATCH, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("disjoint"));
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("no match found"),
        "got: {v}"
    );
    assert!(
        v["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("# scope=file"),
        "got: {v}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), DISJOINT_SOURCE);
}

#[test]
fn scope_file_sweeps_only_the_named_impl_block() {
    // `# scope=file` no longer skips the gate: `impl A` rewrites only
    // `impl A`, leaving `impl B` untouched.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", IMPL_SOURCE);
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep\n# scope=file\n# allow=many\n  impl A {\n-     fn old(...) {\n+     fn new(...) {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(1), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[4, 4]]),
        "got: {v}"
    );
}

const TRAIT_SWEEP_SOURCE: &str =
    "trait T {\n    fn old(&self);\n}\ntrait U {\n    fn old(&self);\n}\n";

#[test]
fn scope_file_sweeps_only_the_named_trait_block() {
    // `trait T` rewrites only `trait T`; the nonsense `trait ZZZ`
    // header matches nothing at all.
    for (patch, matches) in [
        (
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep\n# scope=file\n# allow=many\n  trait T {\n-     fn old(...);\n+     fn new(...);\n  }\n",
            1,
        ),
        (
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep\n# scope=file\n# allow=many\n  trait ZZZ {\n-     fn old(...);\n+     fn new(...);\n  }\n",
            0,
        ),
    ] {
        let dir = init_repo();
        write(dir.path(), "src/lib.rs", TRAIT_SWEEP_SOURCE);
        let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
        if matches == 1 {
            assert!(
                out.status.success(),
                "patch {patch:?} stderr: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let v: serde_json::Value =
                serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
                    .expect("stdout is JSON");
            assert_eq!(
                v["rules"][0]["matches"],
                serde_json::json!(1),
                "patch {patch:?} got: {v}"
            );
            assert_eq!(
                v["rules"][0]["spans"],
                serde_json::json!([[2, 2]]),
                "patch {patch:?} got: {v}"
            );
        } else {
            assert_eq!(
                out.status.code(),
                Some(2),
                "patch {patch:?} stdout: {} stderr: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(read(dir.path(), "src/lib.rs"), TRAIT_SWEEP_SOURCE);
        }
    }
}
