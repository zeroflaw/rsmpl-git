//! T306: engine-verbatim general-shape compile-time reasons (SPEC 18.6 C-table).
//!
//! SPEC 18.6 C-table already carries the exact strings, but nothing pins
//! the text/JSON envelopes end to end. C13/C14/C15 are pinned elsewhere
//! (symbol_shape, inert_directive). C2 closed by WI-5a (`let-else`
//! minus patterns now match structurally — pinned as a fire below).
//! Each remaining test asserts both envelopes plus an untouched
//! worktree. All failures are `RSMPL_PATTERN_CONFLICT` with `rule:` +
//! `file:` lines; text omits the null `range:`/`suggestion:` lines
//! (C11/C12 carry a set `suggestion:`), JSON carries explicit nulls.

mod common;
use common::*;

const BASE: &str = "fn f() {\n let x = 1;\n}\n";
const ATTR_SUGGESTION: &str = "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`";

fn check(patch: &str, reason: &str, want_suggestion: bool) {
    let dir = init_repo();
    write(dir.path(), "a.rs", BASE);
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
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert_eq!(
        stderr.contains("suggestion:"),
        want_suggestion,
        "got: {stderr}"
    );
    if want_suggestion {
        assert!(stderr.contains(ATTR_SUGGESTION), "got: {stderr}");
    }
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, BASE, "worktree must be untouched");

    let dir = init_repo();
    write(dir.path(), "a.rs", BASE);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    assert_eq!(v["error"]["reason"], serde_json::json!(reason), "got: {v}");
    assert_eq!(v["error"]["rule"], serde_json::json!("r"), "got: {v}");
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"), "got: {v}");
    assert!(v["error"]["range"].is_null(), "got: {v}");
    if want_suggestion {
        assert_eq!(
            v["error"]["suggestion"],
            serde_json::json!(ATTR_SUGGESTION),
            "got: {v}"
        );
    } else {
        assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    }
    let got = read(dir.path(), "a.rs");
    assert_eq!(got, BASE, "worktree must be untouched");
}

#[test]
fn c1_ellipsis_in_general_shape() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n- E + ...\n+ E + 1\n",
        "v1 general patterns support `...` only as trailing block statements (`{ prefix; ... }` replays the absorbed tail, omitting it drops the tail); any other `...` position needs a call pattern or a concrete shape",
        false,
    );
}

#[test]
fn c2_let_else_in_minus_now_fires() {
    // WI-5a closed C2: a `let-else` minus matches a `let-else` target
    // structurally (diverge blocks compare as `Block`s).
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn f() {\n let x = foo() else { return 0 };\n let _ = x;\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        "diff --git a.rs\n@@ -2,1 @@ rule=r\n# pat P\n- let P = foo() else { return 0 };\n+ let P = foo();\n",
        &[],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "a.rs");
    assert!(got.contains("let x = foo();"), "got: {got}");
}

#[test]
fn c3_root_stmt_mismatch() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# root=stmt\n# expr E\n- foo(E)\n+ bar(E)\n",
        "`# root=stmt` requires a `let` statement pattern (v1 supports `let` with or without `let-else`)",
        false,
    );
}

#[test]
fn c4_root_expr_mismatch() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# root=expr\n- let x = 1;\n+ let y = 1;\n",
        "`# root=expr` requires a Rust expression pattern",
        false,
    );
}

#[test]
fn c5_root_type_mismatch() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# root=type\n# expr E\n- foo(E)\n+ bar(E)\n",
        "`# root=type` requires a Rust type pattern",
        false,
    );
}

#[test]
fn c6_empty_plus_expression() {
    check(
        "diff --git a.rs\n@@ -2,1 @@ rule=r\n# expr E\n- E + 1\n",
        "v1 expression deletion (empty `+` side) is only supported for `...;` statement shapes; match a `let` statement or write an explicit replacement",
        false,
    );
}

#[test]
fn c7_empty_plus_type() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# type T\n- Vec<T>\n",
        "v1 type patterns cannot be deleted (empty `+` side would produce invalid syntax); write an explicit replacement type",
        false,
    );
}

#[test]
fn c8_embedded_let_metavariable() {
    // WI-3 admits tuple/slice/struct holes, so the C8 pin is the
    // ambiguous shape: `Or` with a hole (every pattern position shares
    // this gate — arm/closure twins live in structural_pat_e2e.rs).
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# pat P\n# expr E\n- let (Some(P) | None) = foo();\n+ let (Some(P) | None) = bar();\n",
        "v1 patterns support a whole-pattern `# pat P` placeholder, a concrete pattern, WI-3 structural composites (`Some(P)`, `(P, Q)`, `S { f: P }`, `&P`, `[P]`, `(P)`) with holes in bindable leaf positions, or `P: T` with placeholder parts, in `let`, match-arm, `for`, closure, and `let`-expression positions; embedded metavariables elsewhere in a pattern are not supported",
        false,
    );
}

#[test]
fn c9_returns_none_on_non_function() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# returns=none\n# expr E\n- foo(E)\n+ bar(E)\n",
        "`# returns=none` requires a function-item pattern",
        false,
    );
}

#[test]
fn c10_leading_attrs_on_non_let() {
    check(
        "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n- #[inline]\n- E + 1\n+ E + 2\n",
        "leading attributes are only supported on `let` statement patterns in v1",
        false,
    );
}

#[test]
fn c11_plus_writes_unmatched_attrs() {
    check(
        "diff --git a.rs\n@@ -1,3 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ #[inline]\n+ fn bar(...) {\n      ...\n  }\n",
        "replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them",
        true,
    );
}

#[test]
fn c12_attrs_differ() {
    check(
        "diff --git a.rs\n@@ -1,3 @@ rule=r\n# lang=rust\n- #[inline]\n- fn foo(...) {\n+ #[must_use]\n+ fn bar(...) {\n      ...\n  }\n",
        "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
        true,
    );
}

#[test]
fn c18_multi_statement_minus() {
    // WI-9c(c) + WI-5c narrowing: a minus side that parses as two or
    // more Rust statements outside the 2–4 plain-`let` sequence shape
    // fails fast at compile time instead of dying at match with a
    // misleading "no match found". (Two plain lets compile as a
    // `Sequence` — pinned by the sequence suites.)
    check(
        "diff --git a.rs\n@@ -2,2 @@ rule=r\n# allow=many\n- let a = foo(E);\n- bar(E);\n+ let a = foo2(E);\n+ bar2(E);\n",
        "v1 statement sequences need 2–4 plain `let` statements on both sides with equal counts (the minus side parses as 2 Rust statements, not a plain-`let` sequence); write 2–4 plain `let` statements, one hunk per statement or item, or use an ordinary hunk",
        false,
    );
}
