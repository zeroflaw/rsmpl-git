//! WI-5: name holes (`N` / `# ident`) in fn/header patterns (SPEC 15.8).
//!
//! A bare `N` in a fn pattern's name position binds each site's ident and
//! replays it on the plus side while the written slots evolve around it.
//! Explicit `# ident X` declares any other name as a hole there. `N`
//! never binds in call/method callee positions (C20), a plus-side `N`
//! without a minus-side `N` is unbound, and bare `N` elsewhere stays a
//! literal ident.

mod common;
use common::*;

const FOUR_FNS: &str =
    "fn a() {\n    1\n}\nfn b(x: i32) -> i32 {\n    x\n}\nfn c<T>(t: T) -> T {\n    t\n}\nfn d() {}\n";

fn write_patch(dir: &tempfile::TempDir, name: &str, body: &str) -> String {
    write(dir.path(), name, body);
    dir.path().join(name).to_str().unwrap().to_string()
}

fn check_json(dir: &tempfile::TempDir, patch: &str) -> serde_json::Value {
    let res = run(
        dir.path(),
        &["--file", patch, "--format", "json", "--check"],
    );
    assert!(
        res.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap()
}

#[test]
fn vis_sweep_adds_pub_with_name_hole() {
    // `- V Q fn N(...)` / `+ pub fn N(...)` adds `pub` to every
    // plain fn while replaying each name; `--check` and apply agree.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FOUR_FNS);
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=pub_sweep\n# sweep\n- V Q fn N(...) {\n+ pub fn N(...) {\n      ...\n  }\n",
    );
    let v = check_json(&dir, &patch);
    assert_eq!(
        v["rules"][0]["matches"], 3,
        "c<T> keeps its generics gate: {v}"
    );
    assert_eq!(
        v["rules"][0]["semantics"], "postcondition",
        "structural plus re-match, not a substring: {v}"
    );
    assert_eq!(v["checked"]["semantics"], "verified");
    let res = run(dir.path(), &["--file", &patch]);
    assert!(
        res.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("pub fn a() {"), "got:\n{out}");
    assert!(out.contains("pub fn b(x: i32) -> i32 {"), "got:\n{out}");
    assert!(out.contains("pub fn d() {}"), "got:\n{out}");
    assert!(
        out.contains("fn c<T>(t: T) -> T {"),
        "generics gate holds: {out}"
    );
    assert!(!out.contains("\nfn a("), "no stale plain fn: {out}");
}

#[test]
fn already_pub_site_reports_identical_edit() {
    // A site already in the plus state matches with a byte-identical
    // `rename`-labeled edit that contributes no diff lines.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {\n    1\n}\npub fn b() {\n    2\n}\nfn c(x: i32) -> i32 {\n    x\n}\n",
    );
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=pub_sweep2\n# sweep\n- V Q fn N(...) {\n+ pub fn N(...) {\n      ...\n  }\n",
    );
    let v = check_json(&dir, &patch);
    assert_eq!(v["rules"][0]["matches"], 3, "{v}");
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[1, 1], [4, 4], [7, 7]]),
        "every site reported, including the conforming one: {v}"
    );
    assert!(
        v["rules"][0]["span_edits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == "rename"),
        "identical name edit is labeled: {v}"
    );
    // Only two real rewrites render diff lines.
    assert_eq!(v["diff_lines"], 9, "{v}");
    let res = run(dir.path(), &["--file", &patch]);
    assert!(
        res.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("pub fn a() {"), "got:\n{out}");
    assert!(out.contains("pub fn b() {"), "got:\n{out}");
}

#[test]
fn all_conforming_sweep_fails_noop_gate() {
    // Every site already `pub`: the sweep change gate fails loud instead
    // of reporting a vacuous success.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "pub fn a() {\n    1\n}\npub fn b() {\n    2\n}\n",
    );
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=pub_sweep3\n# sweep\n- V Q fn N(...) {\n+ pub fn N(...) {\n      ...\n  }\n",
    );
    let res = run(
        dir.path(),
        &["--file", &patch, "--format", "json", "--check"],
    );
    assert!(!res.status.success(), "all-conforming sweep must fail loud");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_MATCH_CONFLICT");
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("changed no bytes"),
        "sweep change gate: {v}"
    );
}

#[test]
fn bound_adding_sweep_fires() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a<T>(t: T) -> T {\n    t\n}\nfn b<U>(u: U) -> U {\n    u\n}\nfn c(x: i32) -> i32 {\n    x\n}\n",
    );
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=bound\n# sweep\n- fn N<T>(...) {\n+ fn N<T: Clone>(...) {\n      ...\n  }\n",
    );
    let v = check_json(&dir, &patch);
    assert_eq!(
        v["rules"][0]["matches"], 1,
        "concrete `<T>` names its sites: {v}"
    );
    assert_eq!(v["rules"][0]["semantics"], "postcondition");
    let res = run(dir.path(), &["--file", &patch]);
    assert!(
        res.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("fn a<T: Clone>(t: T) -> T {"), "got:\n{out}");
    assert!(out.contains("fn b<U>(u: U) -> U {"), "untouched: {out}");
}

#[test]
fn impl_context_method_pair_with_hole() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\nimpl S {\n    fn m1(&self) {\n        1\n    }\n    fn m2(x: i32) -> i32 {\n        x\n    }\n}\n",
    );
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=imeth\n# sweep\n  impl S {\n- fn N(...) {\n+ pub fn N(...) {\n      ...\n  }\n",
    );
    let v = check_json(&dir, &patch);
    assert_eq!(v["rules"][0]["matches"], 2, "{v}");
    assert_eq!(v["rules"][0]["semantics"], "postcondition");
    let res = run(dir.path(), &["--file", &patch]);
    assert!(
        res.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("pub fn m1(&self) {"), "got:\n{out}");
    assert!(out.contains("pub fn m2(x: i32) -> i32 {"), "got:\n{out}");
}

#[test]
fn explicit_ident_sweep() {
    // `# ident Foo` declares a non-`N` name as the hole.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo() {\n    1\n}\nfn bar(x: i32) -> i32 {\n    x\n}\n",
    );
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=foosweep\n# sweep\n# ident Foo\n- V Q fn Foo(...) {\n+ pub fn Foo(...) {\n      ...\n  }\n",
    );
    let v = check_json(&dir, &patch);
    assert_eq!(v["rules"][0]["matches"], 2, "{v}");
    let res = run(dir.path(), &["--file", &patch]);
    assert!(
        res.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("pub fn foo() {"), "got:\n{out}");
    assert!(out.contains("pub fn bar(x: i32) -> i32 {"), "got:\n{out}");
}

#[test]
fn unbound_plus_n_fails() {
    // Plus-side `N` with a concrete minus is unbound.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {\n    1\n}\n");
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=unbound\n# sweep\n- fn a(...) {\n+ fn N(...) {\n      ...\n  }\n",
    );
    let res = run(
        dir.path(),
        &["--file", &patch, "--format", "json", "--check"],
    );
    assert!(!res.status.success(), "unbound plus must fail");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_PATTERN_CONFLICT");
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("without binding: N"),
        "unbound needle: {v}"
    );
    assert_eq!(v["error"]["repair"]["code"], "unbound");
    assert_eq!(read(dir.path(), "src/lib.rs"), "fn a() {\n    1\n}\n");
}

#[test]
fn misplaced_n_in_call_callee_fails_c20() {
    // A name hole never binds a call callee (backticks quote the literal).
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {\n    let y = N(1);\n}\n");
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=miscall\n# ident N\n- N(E)\n+ N(E)\n",
    );
    let res = run(
        dir.path(),
        &["--file", &patch, "--format", "json", "--check"],
    );
    assert!(!res.status.success(), "misplaced N must fail");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_PATTERN_CONFLICT");
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("name hole"),
        "C20 needle: {v}"
    );
    assert_eq!(v["error"]["repair"]["code"], "C20");
}

#[test]
fn misplaced_n_in_method_callee_fails_c20() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {\n    let y = E.old();\n}\n",
    );
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=mismeth\n# ident N\n- E.N()\n+ E.M()\n",
    );
    let res = run(
        dir.path(),
        &["--file", &patch, "--format", "json", "--check"],
    );
    assert!(!res.status.success(), "misplaced method N must fail");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&res.stdout)).unwrap();
    assert_eq!(v["error"]["code"], "RSMPL_PATTERN_CONFLICT");
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("name hole"),
        "C20 needle: {v}"
    );
    assert_eq!(v["error"]["repair"]["code"], "C20");
}

#[test]
fn bare_n_call_stays_literal() {
    // Diet `N` outside name/type positions is the literal ident `N`.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn N(x: i32) -> i32 {\n    x\n}\nfn main() {\n    let y = N(1);\n    let z = N(2);\n}\n",
    );
    let patch = write_patch(
        &dir,
        "change.rsmpl",
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=litcall\n# sweep\n- N(1)\n+ N(2)\n",
    );
    let v = check_json(&dir, &patch);
    assert_eq!(
        v["rules"][0]["matches"], 1,
        "only the literal `N(1)` site: {v}"
    );
    let res = run(dir.path(), &["--file", &patch]);
    assert!(
        res.status.success(),
        "apply: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let out = read(dir.path(), "src/lib.rs");
    assert!(out.contains("let y = N(2);"), "got:\n{out}");
    assert!(out.contains("let z = N(2);"), "N(2) untouched: {out}");
}
