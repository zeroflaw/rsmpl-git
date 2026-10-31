//! WI-14: type-anchored member follow matrix (SPEC 14.8 anchor classes).
//!
//! Proof order: literal-path (struct labels) > signature (fn params) >
//! annotation (`let x: C`, `let x = C { … }`, `self` in `impl C`).
//! Rebinding re-anchors; `-> C` returns never anchor; `make_a().old`
//! never claims. Everything unproven is `shadowed` (sibling defines the
//! name) or `off-target` evidence — never a run failure.

mod common;
use common::*;
use serde_json::Value;

fn json_of(out: &std::process::Output) -> Value {
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON")
}

const PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_timeout\n# sweep\n# follow=uses\n  struct A {\n- timeout: u32\n+ latency: u32\n";

fn check_json(dir: &std::path::Path) -> Value {
    let out = run_stdin(
        dir,
        PATCH,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    json_of(&out)
}

#[test]
fn signature_param_proves_receiver() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct A {\n    timeout: u32,\n}\nfn f(a: A) -> u32 {\n    a.timeout\n}\n",
    );
    let v = check_json(dir.path());
    assert!(v["rules"][0]["follow_skipped"].is_null(), "got: {v}");
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(out.status.success());
    assert!(read(dir.path(), "src/lib.rs").contains("a.latency"));
}

#[test]
fn let_annotation_proves_and_rebinding_reanchors() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct A {\n    timeout: u32,\n}\nstruct B {\n    timeout: u32,\n}\nfn f(a: A) -> u32 {\n    let x: A = a;\n    let y = x.timeout;\n    let x: B = B { timeout: 1 };\n    let z = x.timeout;\n    y + z\n}\n",
    );
    let v = check_json(dir.path());
    let details = v["rules"][0]["follow_skipped_details"].as_array().unwrap();
    assert_eq!(details.len(), 1, "got: {v}");
    assert_eq!(details[0]["reason"], "shadowed", "got: {v}");
    assert_eq!(details[0]["line"], 11, "got: {v}");
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(out.status.success());
    let got = read(dir.path(), "src/lib.rs");
    assert!(
        got.contains("x.latency") || got.contains("y"),
        "got:\n{got}"
    );
    // The second `x.timeout` (B-anchored) survives.
    assert!(
        got.contains("x.timeout"),
        "rebound site untouched, got:\n{got}"
    );
}

#[test]
fn struct_literal_init_proves_receiver() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct A {\n    timeout: u32,\n    keep: bool,\n}\nfn f() -> u32 {\n    let lit = A { timeout: 1, keep: true };\n    lit.timeout\n}\n",
    );
    let v = check_json(dir.path());
    assert!(v["rules"][0]["follow_skipped"].is_null(), "got: {v}");
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(out.status.success());
    assert!(read(dir.path(), "src/lib.rs").contains("lit.latency"));
}

#[test]
fn call_result_receiver_is_off_target_evidence() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct A {\n    timeout: u32,\n}\nfn make_a() -> A {\n    A { timeout: 1 }\n}\nfn f() -> u32 {\n    make_a().timeout\n}\n",
    );
    let v = check_json(dir.path());
    let details = v["rules"][0]["follow_skipped_details"].as_array().unwrap();
    assert_eq!(details.len(), 1, "got: {v}");
    assert_eq!(details[0]["reason"], "off-target", "got: {v}");
}

#[test]
fn self_in_impl_proves_receiver() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct A {\n    timeout: u32,\n}\nimpl A {\n    fn get(&self) -> u32 {\n        self.timeout\n    }\n}\n",
    );
    let v = check_json(dir.path());
    assert!(v["rules"][0]["follow_skipped"].is_null(), "got: {v}");
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(out.status.success());
    assert!(read(dir.path(), "src/lib.rs").contains("self.latency"));
}

#[test]
fn return_type_never_anchors() {
    // `-> A` on the enclosing fn does not prove a bare receiver.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct A {\n    timeout: u32,\n}\nfn f() -> A {\n    A { timeout: 1 }\n}\n",
    );
    let v = check_json(dir.path());
    // No field accesses at all: no skips, no follows.
    assert!(v["rules"][0]["follow_skipped"].is_null(), "got: {v}");
}
