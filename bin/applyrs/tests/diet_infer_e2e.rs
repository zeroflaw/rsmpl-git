//! WI-1c: inferred metavariables end-to-end (SPEC 15.8).
//!
//! No `# expr` / `# type` / `# pat` / `# item` declarations: diet patches
//! infer kinds from position and apply byte-exact. Backticked literals,
//! ambiguity, and explicit-vs-inferred disagreement still fail fast.

mod common;
use common::*;

// --- diet applies -----------------------------------------------------------

#[test]
fn inferred_call_args_apply_without_declarations() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn f() { foo(1); foo(2); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# allow=many\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "fn f() { bar(1); bar(2); }\n");
}

#[test]
fn inferred_type_hole_applies() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn f(x: Vec<u8>) {}\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- Vec<T>\n+ Box<[T]>\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "fn f(x: Box<[u8]>) {}\n");
}

#[test]
fn inferred_let_split_applies() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn f() { let x: u8 = 1; }\n");
    // P/T/E infer as pat/type/expr; the plus reuses the bound P/T and
    // replaces the initializer with a literal (no plus-only holes).
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- let P: T = E;\n+ let P: T = 0;\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "a.rs"), "fn f() { let x: u8 = 0; }\n");
}

#[test]
fn inferred_single_token_captures_item() {
    // `I` infers `# item I`: minus-only single token cuts the one item.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn helper() {}\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# bind=H\n- I\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn inferred_fn_generic_where_slots_apply() {
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn foo<T>(x: T) -> T where T: Clone { x }\n",
    );
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo G (...) W { ... }\n+ fn bar G (...) W { ... }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn bar<T>(x: T) -> T where T: Clone { x }\n"
    );
}

#[test]
fn explicit_declaration_still_agrees() {
    // Explicit diet-compat declarations keep working alongside inference.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn f() { foo(1); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// --- fail-fast needles ------------------------------------------------------

fn fail_both(patch: &str, reason: &str) {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn f() {}\n");
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert!(
        v["error"]["reason"].as_str().unwrap().contains(reason),
        "got: {v}"
    );
}

#[test]
fn ambiguity_names_both_positions() {
    // `E` as call arg (expr) and as `Vec<E>` element (type) in one patch.
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- foo(E)\n- Vec<E>\n+ bar(E)\n+ Box<[E]>\n";
    fail_both(patch, "incompatible positions");
}

#[test]
fn disagreement_names_declared_and_used() {
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr T\n- Vec<T>\n+ Box<[T]>\n";
    fail_both(patch, "declared as expr but used as type");
}

#[test]
fn backticked_literal_stays_literal() {
    // `` `E` `` never infers: the plus literal `E` needs no binding.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn f() { foo(E); }\n");
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(`E`)\n+ foo(`E`)\n";
    // Same literal both sides: matches the literal `E` call site.
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn single_prefn_needs_explicit_disambiguation() {
    // One pre-`fn` token could be `visibility` or `qualifier_list`.
    let patch = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- V fn foo(...) { ... }\n+ V fn bar(...) { ... }\n";
    fail_both(patch, "incompatible positions");
}
