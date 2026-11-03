//! WI-6 durable fixture: inherent-method follow, end-to-end.
//!
//! Engine units pin the domain walk and the orphan parity; the raising
//! sibling lives in `diffrs` (`wi6_inherent_method_e2e`). These tests
//! apply real patches and prove the post-tree builds: the F6 probe
//! (def + param-anchored call) completes and reports `verified`; a
//! same-name method on another struct keeps its uses, reports `partial`
//! with `shadowed` evidence, and still builds.

mod common;
use common::*;
use std::process::Command;

fn cargo_crate(dir: &std::path::Path, lib_rs: &str) {
    write(
        dir,
        "Cargo.toml",
        "[package]\nname = \"wi6crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(dir, "src/lib.rs", lib_rs);
}

fn cargo_check(dir: &std::path::Path) {
    let out = Command::new("cargo")
        .args(["check", "--quiet"])
        .current_dir(dir)
        .output()
        .expect("cannot run `cargo check`");
    assert!(
        out.status.success(),
        "cargo check failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON")
}

fn check_json(dir: &std::path::Path, patch: &str) -> serde_json::Value {
    let out = run_stdin(
        dir,
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    json_of(&out)
}

fn apply_patch(dir: &std::path::Path, patch: &str) {
    let out = run_stdin(dir, patch, &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// F6 headline: `impl S` method + param-anchored call in one hunk.
const INHERENT_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# follow=uses\n  impl S {\n      ...\n-     V Q fn old G (...) W {\n+     V Q fn new G (...) W {\n          ...\n      }\n  }\n";

const CLEAN_LIB: &str =
    "pub struct S;\nimpl S {\n    pub fn old(&self) {}\n}\npub fn f(s: &S) {\n    s.old();\n}\n";

#[test]
fn inherent_follow_completion_applies_and_checks_clean() {
    let dir = init_repo();
    cargo_crate(dir.path(), CLEAN_LIB);
    let v = check_json(dir.path(), INHERENT_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "got: {v}"
    );
    apply_patch(dir.path(), INHERENT_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("fn new(&self)"), "got: {post}");
    assert!(post.contains("s.new();"), "got: {post}");
    assert!(!post.contains("old"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn inherent_follow_shadowed_reports_partial_and_builds() {
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub struct S;\nimpl S {\n    pub fn old(&self) {}\n}\npub struct U;\nimpl U {\n    pub fn old(&self) {}\n}\npub fn f(s: &S, u: &U) {\n    s.old();\n    u.old();\n}\n",
    );
    let v = check_json(dir.path(), INHERENT_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "namesake leftovers block `verified` (WI-1), got: {v}"
    );
    let reasons: Vec<String> = v["rules"][0]["follow_skipped_details"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["reason"].as_str().unwrap().to_string())
        .collect();
    assert!(reasons.iter().any(|r| r == "shadowed"), "got: {reasons:?}");
    apply_patch(dir.path(), INHERENT_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("s.new();"), "got: {post}");
    assert!(post.contains("u.old();"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn inherent_definition_only_lists_call_orphan() {
    let dir = init_repo();
    cargo_crate(dir.path(), CLEAN_LIB);
    let def_only = INHERENT_PATCH.replace("# follow=uses\n", "");
    let v = check_json(dir.path(), &def_only);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "leftover anchored call blocks `verified` (WI-1), got: {v}"
    );
    let texts: Vec<String> = v["rules"][0]["orphans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["text"].as_str().unwrap().to_string())
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("s.old()")),
        "got: {texts:?}"
    );
}

// WI-11: constructor-chained receivers prove through the same-file
// `impl S` return type and complete with `verified`.
const CTOR_LIB: &str = "pub struct S;\nimpl S {\n    pub fn make() -> S {\n        S\n    }\n    pub fn old(&self) {}\n}\npub fn f() {\n    S::make().old();\n}\n";

const CTOR_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# follow=uses\n  impl S {\n      ...\n-     V Q fn old G (...) W {\n+     V Q fn new G (...) W {\n          ...\n      }\n  }\n";

#[test]
fn inherent_ctor_chain_completion_applies_and_checks_clean() {
    let dir = init_repo();
    cargo_crate(dir.path(), CTOR_LIB);
    let v = check_json(dir.path(), CTOR_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "ctor-chained call completes (WI-11), got: {v}"
    );
    apply_patch(dir.path(), CTOR_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("fn new(&self)"), "got: {post}");
    assert!(post.contains("S::make().new();"), "got: {post}");
    assert!(!post.contains("old"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn inherent_two_hop_chain_rewrites() {
    // WI-11: two hops prove S through both steps.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub struct S;\nimpl S {\n    pub fn make() -> S {\n        S\n    }\n    pub fn foo(&self) -> S {\n        S\n    }\n    pub fn old(&self) {}\n}\npub fn f() {\n    S::make().foo().old();\n}\n",
    );
    let v = check_json(dir.path(), CTOR_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("verified"),
        "two-hop chain proves (WI-11), got: {v}"
    );
    apply_patch(dir.path(), CTOR_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("S::make().foo().new();"), "got: {post}");
    assert!(!post.contains("old"), "got: {post}");
    cargo_check(dir.path());
}

#[test]
fn inherent_three_hop_chain_skips_chain_depth() {
    // WI-11 bound: a third hop keeps `chain-depth`.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub struct S;\nimpl S {\n    pub fn make() -> S {\n        S\n    }\n    pub fn foo(&self) -> S {\n        S\n    }\n    pub fn bar(&self) -> S {\n        S\n    }\n    pub fn old(&self) {}\n}\npub fn f() {\n    S::make().foo().bar().old();\n}\n",
    );
    let v = check_json(dir.path(), CTOR_PATCH);
    assert_eq!(
        v["checked"]["semantics"],
        serde_json::json!("partial"),
        "three-hop chain exceeds the bounded-two cap (WI-11), got: {v}"
    );
    let reasons: Vec<String> = v["rules"][0]["follow_skipped_details"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["reason"].as_str().unwrap().to_string())
        .collect();
    assert!(
        reasons.iter().any(|r| r == "chain-depth"),
        "got: {reasons:?}"
    );
    apply_patch(dir.path(), CTOR_PATCH);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("S::make().foo().bar().old();"), "got: {post}");
    // No `cargo check`: the definition renamed while the skipped call
    // still names `old` — the evidence (not the build) is the pin.
}

#[test]
fn inherent_attribute_value_vetoes_then_skips_with_fallback() {
    // Attribute values are opaque to inherent-method renames (trait
    // parity): `#[doc = S::old]` vetoes without fallback, rewrites def
    // + anchored call and reports `macro-hidden` with fallback while
    // the value stays byte-verbatim. Pre-fix, the value rewrote to
    // `#[doc = S::new]` with `verified` and no orphans.
    let dir = init_repo();
    cargo_crate(
        dir.path(),
        "pub struct S;\nimpl S {\n    pub fn old(&self) {}\n}\npub fn f(s: &S) {\n    s.old();\n}\n#[doc = S::old]\nfn g() {}\n",
    );
    let out = run_stdin(dir.path(), INHERENT_PATCH, &["--format", "json", "--check"]);
    assert!(!out.status.success(), "opacity veto expected");
    let v = json_of(&out);
    assert_eq!(v["error"]["code"], "RSMPL_SYMBOL_CONFLICT", "got: {v}");
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("an attribute"),
        "attr ctx named, got: {v}"
    );

    let patch_fb = INHERENT_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
    let v = check_json(dir.path(), &patch_fb);
    assert_eq!(v["checked"]["semantics"], "partial", "got: {v}");
    let details = v["rules"][0]["follow_skipped_details"]
        .as_array()
        .expect("skips present");
    assert!(
        details
            .iter()
            .any(|d| d["line"] == 8 && d["reason"] == "macro-hidden"),
        "attr line named, got: {v}"
    );

    // Real apply: def + anchored call rewrite, attribute verbatim.
    // No `cargo check`: `#[doc = S::old]` is intentionally non-code
    // (an opaque value, never compiled as a path) — the byte-compare
    // below is the pin.
    apply_patch(dir.path(), &patch_fb);
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("pub fn new(&self) {}"), "got: {post}");
    assert!(post.contains("s.new();"), "got: {post}");
    assert!(post.contains("#[doc = S::old]"), "value kept, got: {post}");
}
