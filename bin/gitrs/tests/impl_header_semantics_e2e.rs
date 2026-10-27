//! T323: impl/trait-context header values are advisory, not filters.
//!
//! SPEC 13.4: the engine checks the header value first (impl: self-ty
//! or trait-path token mention; trait: trait ident), but on mismatch it
//! falls back to range-overlap gating — any overlapping `impl` (resp.
//! `trait`) block is accepted and the inner fn match decides. Target one
//! block via its range; sweep every block via `# scope=file` + `@@ -1,1 @@`.
//! Probed: `impl A` header + range over `impl B` rewrote B (same for
//! `trait T` over `trait U`); disjoint ranges fail with the standard
//! no-match + `# scope=file` retry suggestion.

mod common;
use common::*;

const IMPL_SOURCE: &str =
    "struct A;\nstruct B;\nimpl A {\n    fn old(&self) {}\n}\nimpl B {\n    fn old(&self) {}\n}\n";
const IMPL_MISMATCH_PATCH: &str = "diff --git src/lib.rs\n@@ -6,3 @@ rule=impl_mismatch\n# lang=rust\n  impl A {\n-     fn old(...) {\n+     fn new(...) {\n          ...\n      }\n  }\n";

const TRAIT_SOURCE: &str = "trait T {\n    fn old(&self);\n}\ntrait U {\n    fn old(&self);\n}\n";
const TRAIT_MISMATCH_PATCH: &str = "diff --git src/lib.rs\n@@ -4,3 @@ rule=trait_mismatch\n# lang=rust\n  trait T {\n-     fn old(...);\n+     fn new(...);\n  }\n";

const DISJOINT_SOURCE: &str = "trait T {\n    fn old(&self);\n}\nfn unrelated() {}\n";
const DISJOINT_PATCH: &str = "diff --git src/lib.rs\n@@ -4,1 @@ rule=disjoint\n# lang=rust\n  trait T {\n-     fn old(...);\n+     fn new(...);\n  }\n";

const SWEEP_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep\n# scope=file\n# allow=many\n  impl A {\n-     fn old(...) {\n+     fn new(...) {\n          ...\n      }\n  }\n";

#[test]
fn impl_header_mismatch_rewrites_overlapping_block() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", IMPL_SOURCE);
    let out = run_stdin(dir.path(), IMPL_MISMATCH_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(
        got.contains("impl A {\n    fn old(&self) {}\n}"),
        "impl A must be untouched, got:\n{got}"
    );
    assert!(
        got.contains("impl B {\n    fn new(&self) {}\n}"),
        "overlapping impl B must be rewritten, got:\n{got}"
    );
}

#[test]
fn trait_header_mismatch_rewrites_overlapping_block() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", TRAIT_SOURCE);
    let out = run_stdin(dir.path(), TRAIT_MISMATCH_PATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(
        got.contains("trait T {\n    fn old(&self);\n}"),
        "trait T must be untouched, got:\n{got}"
    );
    assert!(
        got.contains("trait U {\n    fn new(&self);\n}"),
        "overlapping trait U must be rewritten, got:\n{got}"
    );
}

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
fn scope_file_sweeps_all_impl_blocks() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", IMPL_SOURCE);
    let out = run_stdin(dir.path(), SWEEP_PATCH, &["--format", "json", "--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(2), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"].as_array().unwrap().len(),
        2,
        "got: {v}"
    );
}
