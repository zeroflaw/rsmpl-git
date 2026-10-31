//! WI-3: structural pattern positions — destructuring holes in
//! `let`/`match`/`for`/closures.
//!
//! Composite patterns (`Some(P)`, `(P, Q)`, `S { f: P }`, `&P`, `[P]`)
//! match structurally with holes bound per position; `Or`-with-hole
//! fails at C8 in every position. Each fire pins the rewritten tree;
//! each veto pins the untouched tree plus the loud conflict.

mod common;
use common::*;

// --- tuple-struct (`Some(P)`) ---

#[test]
fn some_hole_sweep_rewrites_both_sites() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let _ = (v, w);\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=some_unwrap\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let Some(P) = E;\n+ let Some(P) = E.unwrap_or(0);\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("let Some(v) = o.unwrap_or(0);"), "got: {got}");
    assert!(got.contains("let Some(w) = p.unwrap_or(0);"), "got: {got}");
}

#[test]
fn some_hole_head_mismatch_applies_nothing() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a(o: Option<i32>) {\n    let None = o;\n    let _ = 1;\n}\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=some_unwrap\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let Some(P) = E;\n+ let Some(P) = E.unwrap_or(0);\n",
        &["--check"],
    );
    assert!(!out.status.success(), "must veto");
    assert_eq!(read(dir.path(), "src/lib.rs"), before);
}

// --- tuple + struct ---

#[test]
fn tuple_and_struct_holes_fire() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a(t: (i32, i32), s: S) {\n    let (x, y) = t;\n    let S { f: v } = s;\n    let _ = (x, y, v);\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=swap\n# sweep\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- let (P, Q) = E;\n+ let (Q, P) = E;\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("let (y, x) = t;"), "got: {got}");
}

#[test]
fn struct_field_reorder_matches_by_member_name() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a(s: S) {\n    let S { g: 1, f: v } = s;\n    let _ = v;\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=swap_fields\n# sweep\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- let S { f: P, g: Q } = E;\n+ let S { f: Q, g: P } = E;\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("let S { f: 1, g: v } = s;"), "got: {got}");
}

// --- arm / for / closure positions ---

#[test]
fn composite_hole_in_match_arm_fires() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a(o: Option<i32>) {\n    let v = match o { Some(v) => 1, _ => 2 };\n    let _ = v;\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=arm_bump\n# lang=rust\n# pat P\n# expr E\n- match E { Some(P) => 1, _ => 2 }\n+ match E { Some(P) => 9, _ => 2 }\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("Some(v) => 9"), "got: {got}");
}

#[test]
fn composite_hole_in_for_and_closure_fires() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {\n    for (k, v) in m { break; }\n    let f = |(x, y)| x;\n    let _ = f;\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=for_swap\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- for (P, Q) in E { break; }\n+ for (P, Q) in E { continue; }\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("for (k, v) in m { continue; }"), "got: {got}");
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -3,1 @@ rule=clo_bump\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- |(P, Q)| E\n+ |(P, Q)| E + 1\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("|(x, y)| x + 1"), "got: {got}");
}

// --- C8 uniformity: `Or`-with-hole in non-`let` positions ---

#[test]
fn or_with_hole_in_arm_fails_c8() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a(o: Option<i32>) {\n    let v = match o { Some(v) => 1, _ => 2 };\n    let _ = v;\n}\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=or_arm\n# lang=rust\n# pat P\n# expr E\n- match E { (Some(P) | None) => 1, _ => 2 }\n+ match E { (Some(P) | None) => 9, _ => 2 }\n",
        &["--check"],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("embedded metavariables elsewhere"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), before);
}

#[test]
fn or_with_hole_in_closure_param_fails_c8() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {\n    let f = |(x, y)| x;\n    let _ = f;\n}\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=or_clo\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- |(P | Q)| E\n+ |(P | Q)| E + 1\n",
        &["--check"],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("embedded metavariables elsewhere"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), before);
}

// --- type-hole paths: a declared hole naming the whole path binds ---

#[test]
fn type_hole_struct_path_binds() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a(s: S) {\n    let S { f: v } = s;\n    let _ = v;\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=path_hole\n# sweep\n# lang=rust\n# type S\n# pat P\n# expr E\n- let S { f: P } = E;\n+ let S { f: P } = E + 1;\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("let S { f: v } = s + 1;"), "got: {got}");
}

// --- diet: no directives, holes inferred by position ---

#[test]
fn diet_structural_holes_infer_pat() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a(o: Option<i32>) {\n    let Some(v) = o;\n    let _ = v;\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=diet_some\n# sweep\n- let Some(P) = E;\n+ let Some(P) = E.unwrap_or(0);\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("let Some(v) = o.unwrap_or(0);"), "got: {got}");
}
