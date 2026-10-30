//! T171: pattern-compilation integration tests, moved verbatim out of
//! `src/pattern.rs` (which sat at 1977/2000 LOC under the
//! `no_rust_file_exceeds_2000_lines` repo cap).
//!
//! Each `tests/*.rs` file is its own crate, so the `hunk_of` helper is
//! duplicated here, not shared (split-file precedent: `ellipsis_e2e.rs`).

use rsmpl_patch::{parse_patch, Hunk, RsmplHunk};
use rsmpl_pattern::{compile, split_leading_attrs, ExprPatternKind, GeneralShape, PatternKind};

fn hunk_of(patch: &str) -> RsmplHunk {
    let p = parse_patch(patch).unwrap();
    match p.files[0].hunks[0].clone() {
        Hunk::Rsmpl(rh) => rh,
        _ => panic!("expected rsmpl"),
    }
}

#[test]
fn compiles_fn_rename() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
    );
    let c = compile(&h).unwrap();
    assert!(matches!(c.kind, PatternKind::Fn { .. }));
}

#[test]
fn split_leading_attrs_peels_doc_lines() {
    // T155: `///`/`//!` peel alongside `#[...]` and normalize to the
    // `#[doc = "..."]` canonical form; peeling stops at the first
    // non-attribute line.
    let (attrs, rest) = split_leading_attrs("/// Doc.\n#[inline]\n//! Inner.\nfn foo() {}");
    assert_eq!(
        attrs,
        vec![
            "#[doc = \" Doc.\"]".to_string(),
            "#[inline]".to_string(),
            "#[doc = \" Inner.\"]".to_string(),
        ],
        "got: {attrs:?}"
    );
    assert_eq!(rest, "fn foo() {}");
    let (none, same) = split_leading_attrs("//// Not doc.\nfn foo() {}");
    assert!(none.is_empty(), "got: {none:?}");
    assert_eq!(same, "//// Not doc.\nfn foo() {}");
    // T156: single-line block docs peel to `#[doc]`; multi-line
    // openers and ordinary block comments stop the peel.
    let (attrs, rest) = split_leading_attrs("/** Block doc. */\nlet x = E;");
    assert_eq!(attrs, vec!["#[doc = \" Block doc. \"]".to_string()]);
    assert_eq!(rest, "let x = E;");
    let (none, _) = split_leading_attrs("/** Unclosed.\nlet x = E;");
    assert!(none.is_empty(), "got: {none:?}");
    let (none, _) = split_leading_attrs("/* plain */\nlet x = E;");
    assert!(none.is_empty(), "got: {none:?}");
}

#[test]
fn compiles_visibility_meta_fn() {
    // `V fn foo` routes to a function pattern, not Raw/Expr.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=x\n# lang=rust\n# visibility V\n- V fn foo(...) {\n+ pub fn bar(...) {\n      ...\n  }\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(c.kind, PatternKind::Fn { .. }),
        "got: {:?}",
        c.kind
    );
}

#[test]
fn compiles_call_rewrite() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=rename_calls\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n",
    );
    let c = compile(&h).unwrap();
    assert!(matches!(c.kind, PatternKind::Expr { .. }));
    assert!(c.unbound_plus.is_empty());
}

#[test]
fn compiles_use_rename() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=u\n# lang=rust\n# allow=many\n- use crate::old\n+ use crate::new\n",
    );
    let c = compile(&h).unwrap();
    match &c.kind {
        PatternKind::UseRename {
            minus_segs,
            plus_path,
        } => {
            assert_eq!(minus_segs, &vec!["crate".to_string(), "old".to_string()]);
            assert_eq!(plus_path, "crate::new");
        }
        _ => panic!("expected use rename, got: {:?}", c.kind),
    }
    assert!(c.unbound_plus.is_empty());
}

#[test]
fn use_call_callee_not_stolen() {
    // `use_foo(E)` is a call, not a `use` prefix: word-boundary guard.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- use_foo(E)\n+ use_bar(E)\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(c.kind, PatternKind::Expr { .. }),
        "got: {:?}",
        c.kind
    );
}

#[test]
fn detects_unbound_plus() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=x\n# lang=rust\n# expr E\n- foo(E)\n+ bar(Q)\n",
    );
    let c = compile(&h).unwrap();
    // WI-1c diet: undeclared `Q` in argument position infers `expr`, so a
    // plus-only hole is tracked as unbound (was literal before inference).
    assert_eq!(c.unbound_plus, vec!["Q".to_string()]);
}

#[test]
fn detects_declared_but_unbound_plus() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=x\n# lang=rust\n# expr E\n# expr Q\n- foo(E)\n+ bar(Q)\n",
    );
    let c = compile(&h).unwrap();
    assert_eq!(c.unbound_plus, vec!["Q".to_string()]);
}

#[test]
fn expr_carries_placeholders_and_path_ellipsis() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -80,20 +80,20 @@ rule=x\n# lang=rust\n# allow=many\n# symbol Old=crate::old_api\n- Old(...)\n+ crate::new_api(...)\n",
    );
    let c = compile(&h).unwrap();
    match &c.kind {
        PatternKind::Expr {
            placeholders,
            has_path_ellipsis,
            ..
        } => {
            assert!(!placeholders.is_empty() || c.table.symbols.contains_key("Old"));
            assert!(!has_path_ellipsis);
        }
        _ => panic!("expected expr"),
    }
}

#[test]
fn compiles_item_capture() {
    let h = hunk_of(
        "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -20,12 +20,0 @@ rule=cut\n# lang=rust\n# item I\n# bind=Helper\n- I\n",
    );
    let c = compile(&h).unwrap();
    assert!(matches!(c.kind, PatternKind::ItemCapture { .. }));
}

#[test]
fn repeated_metavar_detected() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,1 +40,1 @@ rule=x\n# lang=rust\n# expr E\n- assert_eq!(E, E)\n+ assert_equal_once(E)\n",
    );
    let c = compile(&h).unwrap();
    assert!(c.repeated.contains(&"E".to_string()));
}

#[test]
fn let_shape_compiles_to_general_stmt() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(
            &c.kind,
            PatternKind::General {
                shape: GeneralShape::Stmt,
                ..
            }
        ),
        "got: {:?}",
        c.kind
    );
    assert!(c.unbound_plus.is_empty());
}

#[test]
fn impl_trait_multisegment_pair_routes_to_general_type() {
    // T149: `- impl std::fmt::Display` trips the bare-name gate in
    // `parse_side`, but both sides are Rust types, so the hunk falls
    // through to the general type matcher instead of failing.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- impl std::fmt::Display\n+ Box<dyn std::fmt::Display>\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(
            &c.kind,
            PatternKind::General {
                shape: GeneralShape::Type,
                ..
            }
        ),
        "got: {:?}",
        c.kind
    );
}

#[test]
fn block_comment_gate_single_line_normalizes_multi_line_needles() {
    // T156: single-line `/** */` peels and the `let` compiles;
    // multi-line block docs fail fast with the targeted needle.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,2 +2,2 @@ rule=x\n# lang=rust\n# expr E\n- /** Block doc. */\n- let x = E;\n+ /** Block doc. */\n+ let y = E;\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(
            &c.kind,
            PatternKind::General {
                shape: GeneralShape::Stmt,
                ..
            }
        ),
        "got: {:?}",
        c.kind
    );
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,3 +2,3 @@ rule=x\n# lang=rust\n# expr E\n- /** Multi\n- line. */\n- let x = E;\n+ /** Multi\n+ line. */\n+ let y = E;\n",
    );
    let err = compile(&h).unwrap_err().to_string();
    assert!(err.contains("block comments"), "unexpected: {err}");
}

#[test]
fn binary_expr_compiles_to_general_expr() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + 1\n+ E.wrapping_add(1)\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(
            &c.kind,
            PatternKind::General {
                shape: GeneralShape::Expr,
                ..
            }
        ),
        "got: {:?}",
        c.kind
    );
}

#[test]
fn type_with_type_metavar_compiles_to_general_type() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# type T\n- Vec<T>\n+ Box<[T]>\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(
            &c.kind,
            PatternKind::General {
                shape: GeneralShape::Type,
                ..
            }
        ),
        "got: {:?}",
        c.kind
    );
}

#[test]
fn call_with_semi_stays_call() {
    // `foo(E);` must keep call-matcher routing (not Stmt/GeneralExpr).
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E);\n+ bar(E);\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(&c.kind, PatternKind::Expr { .. }),
        "got: {:?}",
        c.kind
    );
}

#[test]
fn root_stmt_forces_let() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=stmt\n# stmt S\n- let x = 1;\n+ let y = 1;\n",
    );
    assert!(matches!(
        compile(&h).unwrap().kind,
        PatternKind::General {
            shape: GeneralShape::Stmt,
            ..
        }
    ));
    let bad = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=stmt\n- foo\n+ bar\n",
    );
    assert!(compile(&bad).is_err());
}

#[test]
fn ellipsis_rejected_in_general_shapes() {
    // `...` still works in call routing...
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- foo(..., E)\n+ bar(E)\n",
    );
    assert!(matches!(
        compile(&h).unwrap().kind,
        PatternKind::Expr { .. }
    ));
    // ...but not in general shapes.
    let h2 = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + ...\n+ E\n",
    );
    assert!(compile(&h2).is_err());
}

#[test]
fn type_deletion_rejected() {
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,0 @@ rule=x\n# lang=rust\n# type T\n- Vec<T>\n",
    );
    assert!(compile(&h).is_err());
}

#[test]
fn item_rename_shapes_compile() {
    for (minus, plus) in [
        ("struct Old", "struct New"),
        ("enum Old", "enum New"),
        ("trait Old", "trait New"),
        ("type Old", "type New"),
        ("const Old", "const New"),
        ("static mut Old", "static mut New"),
        ("mod old", "mod new"),
        ("impl Old", "impl New"),
        ("pub struct Old", "pub struct New"),
        ("struct Old { ... }", "struct New { ... }"),
    ] {
        let text = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- {minus}\n+ {plus}\n"
        );
        let c = compile(&hunk_of(&text)).unwrap();
        assert!(
            matches!(&c.kind, PatternKind::ItemRename { .. }),
            "{minus} got: {:?}",
            c.kind
        );
        assert!(c.unbound_plus.is_empty());
    }
}

#[test]
fn item_rename_mismatch_rejected() {
    // Kind mismatch.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct Old\n+ enum New\n",
    );
    assert!(compile(&h).is_err());
    // Identical names.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct Old\n+ struct Old\n",
    );
    assert!(compile(&h).is_err());
    // Concrete body.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct Old<T>\n+ struct New\n",
    );
    assert!(compile(&h).is_err());
    // `$Meta` names unsupported.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct $S\n+ struct New\n",
    );
    assert!(compile(&h).is_err());
}

#[test]
fn item_rename_prefix_difference_rejected() {
    // T23a: spelled vis/qualifier differences fail fast (rename-only,
    // never a silent vis change). Bare-vs-spelled both directions.
    for (minus, plus) in [
        ("pub struct Old", "struct New"),
        ("struct Old", "pub struct New"),
        ("pub struct Old", "pub(crate) struct New"),
        ("static Old", "static mut New"),
        ("unsafe trait Old", "trait New"),
    ] {
        let text = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- {minus}\n+ {plus}\n"
        );
        let err = compile(&hunk_of(&text)).unwrap_err();
        assert!(
            err.to_string().contains("prefixes differ"),
            "{minus} / {plus} got: {err}"
        );
    }
    // Same spelled prefix still compiles (`pub (crate)` ≡ `pub(crate)`).
    let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- pub (crate) struct Old\n+ pub(crate) struct New\n";
    assert!(matches!(
        compile(&hunk_of(text)).unwrap().kind,
        PatternKind::ItemRename { .. }
    ));
}

#[test]
fn const_fn_still_routes_to_fn() {
    // `const fn` must not be captured by the item-rename classifier.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- const fn foo(...) {\n+ const fn bar(...) {\n      ...\n  }\n",
    );
    assert!(
        matches!(compile(&h).unwrap().kind, PatternKind::Fn { .. }),
        "got const-fn routing"
    );
}

#[test]
fn general_coverage_diagnostics_are_pinned() {
    // SPEC 18.6 compile-time rows (C1-C10, C2 closed by WI-5a): each
    // unsupported shape must fail with the exact table diagnostic, so
    // the table cannot drift from the code without a test failing.
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    // C2 (closed by WI-5a): a `let-else` minus now compiles to a
    // General Stmt pattern instead of failing.
    let c2 = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let x = E else {{ return; }};\n+ let y = E;\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&c2)).unwrap().kind,
            PatternKind::General { .. }
        ),
        "C2 let-else minus must compile to General Stmt"
    );
    let cases = [
        // C1: `...` in a general shape.
        (
            format!(
                "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + ...\n+ E\n"
            ),
            "v1 general stmt/expr/type patterns do not support `...` (use a call pattern or a concrete shape)",
        ),
        // C3: `# root=stmt` with a non-`let` shape.
        (
            format!("{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=stmt\n- foo\n+ bar\n"),
            "`# root=stmt` requires a `let` statement pattern (v1 supports `let` with or without `let-else`)",
        ),
        // C4: `# root=expr` with a non-expression shape.
        (
            format!(
                "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=expr\n- let x = 1;\n+ let y = 1;\n"
            ),
            "`# root=expr` requires a Rust expression pattern",
        ),
        // C5: `# root=type` with a non-type shape.
        (
            format!(
                "{head}@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# root=type\n- foo bar\n+ Box<u8>\n"
            ),
            "`# root=type` requires a Rust type pattern",
        ),
        // C6: empty `+` side on an expression.
        (
            format!("{head}@@ -2,1 +2,0 @@ rule=x\n# lang=rust\n# expr E\n- E + 1\n"),
            "v1 expression deletion (empty `+` side) is only supported for `...;` statement shapes; match a `let` statement or write an explicit replacement",
        ),
        // C7: empty `+` side on a type.
        (
            format!("{head}@@ -1,1 +1,0 @@ rule=x\n# lang=rust\n# type T\n- Vec<T>\n"),
            "v1 type patterns cannot be deleted (empty `+` side would produce invalid syntax); write an explicit replacement type",
        ),
        // C8: embedded metavariable inside the `let` bound pattern.
        // (T16a: paren tuples reach the gate too — the `let`-led
        // bypass stops the call router claiming `let(...)`; the
        // slice shape below pins the C8 diagnostic byte-for-byte.)
        (
            format!(
                "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let [E, x] = E;\n+ let [E, x] = E;\n"
            ),
            "v1 `let` patterns support a whole-pattern `# pat P` placeholder, a concrete pattern, or `P: T` with placeholder parts; embedded metavariables elsewhere in the pattern are not supported",
        ),
        // C9: `# returns=none` on a non-function hunk.
        (
            format!(
                "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n# returns=none\n- foo(E)\n+ bar(E)\n"
            ),
            "`# returns=none` requires a function-item pattern",
        ),
        // C14: `# symbol` in a general shape (call-shape-only even
        // with `# fallback=syntax`, where the constraint is unenforceable).
        (
            format!(
                "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n# fallback=syntax\n# symbol Old=crate::old_api\n- E + 1\n+ E + 2\n"
            ),
            "`# symbol` is only supported in call shapes in v0.1",
        ),
    ];
    assert_eq!(
        cases.len(),
        9,
        "SPEC 18.6 lists C1,C3-C10 compile-time rows (C2 closed by WI-5a)"
    );
    for (patch, want) in cases {
        let err = compile(&hunk_of(&patch)).unwrap_err();
        let rendered = err.render();
        assert_eq!(
            err.code,
            rsmpl_patch::ErrorCode::PatternConflict,
            "wrong code for {want:?}:\n{rendered}"
        );
        assert!(
            rendered.contains(want),
            "SPEC 18.6 drift: want {want:?} in:\n{rendered}"
        );
    }
}

#[test]
fn paren_tuple_let_reaches_let_gate() {
    // T16a router-precision pin: `let (E, x)` must reach the general
    // `let` gate (C8) instead of classifying as a `let(...)` call.
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let patch = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let (E, x) = E;\n+ let (E, x) = E;\n"
    );
    let err = compile(&hunk_of(&patch)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render().contains("embedded metavariables elsewhere"),
        "paren-tuple let missed the C8 gate:\n{}",
        err.render()
    );
    // Concrete paren-tuple lets compile to the Stmt shape.
    let ok = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n- let (a, b) = y;\n+ let (c, d) = y;\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&ok)).unwrap().kind,
            PatternKind::General {
                shape: GeneralShape::Stmt,
                ..
            }
        ),
        "concrete paren-tuple let should be General::Stmt"
    );
}

#[test]
fn method_ident_symbol_rejected_at_compile() {
    // T147: a `# symbol` naming the method ident can never match (the
    // method slot compares bare idents, never seeded paths), so fail
    // fast instead of a bare match-cycle `no match found`.
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let patch = format!(
        "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n# symbol old=crate::old_api\n# expr R\n# expr E\n- R.old(E)\n+ R.new(E)\n"
    );
    let err = compile(&hunk_of(&patch)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render().contains("names the method ident"),
        "missing needle:\n{}",
        err.render()
    );
    // Negative control: a receiver symbol is the supported C14 shape
    // and must still compile.
    let ok = format!(
        "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n# symbol R=crate::r::R\n# expr E\n- R.old(E)\n+ R.new(E)\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&ok)).unwrap().kind,
            PatternKind::Expr {
                minus: ExprPatternKind::MethodCall { .. },
                ..
            }
        ),
        "receiver symbols must keep compiling"
    );
    // Second negative control: a plain `# expr` metavar in method
    // position binds any method and must not trip the symbol gate.
    let ok_meta = format!(
        "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n# expr M\n# expr E\n- R.M(E)\n+ R.new(E)\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&ok_meta)).unwrap().kind,
            PatternKind::Expr {
                minus: ExprPatternKind::MethodCall { .. },
                ..
            }
        ),
        "method-position expr metavars must keep compiling"
    );
}

#[test]
fn multi_statement_minus_fails_fast_at_compile() {
    // WI-9c(c): a minus side that parses as two or more Rust statements
    // is a sequence, which v1 general patterns cannot express (no
    // multi-statement kind until WI-9c(a)). Fail fast at compile time
    // naming the pattern instead of degrading to `Raw` and dying at
    // match with a misleading "no match found".
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let patch = format!(
        "{head}@@ -2,2 @@ rule=x\n# lang=rust\n# allow=many\n- let a = foo(E);\n- let b = bar(E);\n+ let a = foo2(E);\n+ let b = bar2(E);\n"
    );
    let err = compile(&hunk_of(&patch)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render()
            .contains("multi-statement sequences are not supported"),
        "missing needle:\n{}",
        err.render()
    );
    assert!(
        err.render().contains("parses as 2 Rust statements"),
        "missing count:\n{}",
        err.render()
    );
    // Negative control: a multi-line SINGLE `let` (line-continued call)
    // still compiles — the sequence gate must never false-positive.
    let ok = format!(
        "{head}@@ -2,3 @@ rule=x\n# lang=rust\n# allow=many\n- let a = foo(\n- E\n- );\n+ let a = foo2(E);\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&ok)).unwrap().kind,
            PatternKind::General {
                shape: GeneralShape::Stmt,
                ..
            }
        ),
        "multi-line single let should be General::Stmt"
    );
}

#[test]
fn inert_directives_fail_fast() {
    // T169: a lone `# fallback=syntax` (no `# symbol`) and a
    // non-default `# attrs=` on call/macro shapes are silent no-ops,
    // so fail fast instead of implying a constraint (SPEC 2.6).
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let cases = [
        (
            format!(
                "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n- foo()\n+ bar()\n"
            ),
            "`# fallback=syntax` without `# symbol` does nothing",
        ),
        (
            format!(
                "{head}@@ -1,2 @@ rule=x\n# lang=rust\n# fallback=syntax\n- fn foo(...) {{\n+ fn bar(...) {{\n      ...\n  }}\n"
            ),
            "`# fallback=syntax` without `# symbol` does nothing",
        ),
        (
            format!(
                "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# attrs=strip\n- foo()\n+ bar()\n"
            ),
            "`# attrs=` is ignored on call/macro shapes",
        ),
        (
            format!(
                "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# allow=many\n# attrs=explicit\n# expr R\n# expr E\n- R.old(E)\n+ R.new(E)\n"
            ),
            "`# attrs=` is ignored on call/macro shapes",
        ),
        (
            format!(
                "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# macro=token_tree\n# attrs=absent\n- M!(...)\n+ N!(...)\n"
            ),
            "`# attrs=` is ignored on call/macro shapes",
        ),
    ];
    for (patch, want) in cases {
        let err = compile(&hunk_of(&patch)).unwrap_err();
        assert_eq!(
            err.code,
            rsmpl_patch::ErrorCode::PatternConflict,
            "wrong code for {want:?}:\n{}",
            err.render()
        );
        assert!(
            err.render().contains(want),
            "missing needle {want:?} in:\n{}",
            err.render()
        );
    }
    // Negative controls: fallback paired with a symbol, explicit
    // preserve on a call, and strip on a `let` must keep compiling.
    let ok_symbol = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n# symbol Old=crate::old_api\n# expr E\n- Old(E)\n+ New(E)\n"
    );
    assert!(
        compile(&hunk_of(&ok_symbol)).is_ok(),
        "fallback+symbol must compile"
    );
    let ok_preserve = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# attrs=preserve\n- foo()\n+ bar()\n"
    );
    assert!(
        compile(&hunk_of(&ok_preserve)).is_ok(),
        "explicit preserve on a call must compile"
    );
    let ok_let = format!(
        "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# attrs=strip\n# expr E\n- let x = E;\n+ let y = E;\n"
    );
    assert!(
        compile(&hunk_of(&ok_let)).is_ok(),
        "strip on a let must compile"
    );
}

#[test]
fn attributes_kind_rejected_at_compile() {
    // T165: `# attributes A` declares an attribute-list abstraction no
    // matcher binds, so fail fast with the repair instead of
    // misrouting (own-line `- A` missed the fn keyword; same-line
    // `- A fn foo` fell through to the general matcher).
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let patch = format!(
        "{head}@@ -1,2 @@ rule=x\n# lang=rust\n# attributes A\n- A\n- fn foo(...) {{\n+ A\n+ fn bar(...) {{\n      ...\n  }}\n"
    );
    let err = compile(&hunk_of(&patch)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render()
            .contains("attribute lists cannot be abstracted"),
        "missing needle:\n{}",
        err.render()
    );
    // Negative control: the literal-mirror spelling the needle points
    // at must keep compiling to a function pattern.
    let ok = format!(
        "{head}@@ -1,2 @@ rule=x\n# lang=rust\n- #[inline]\n- fn foo(...) {{\n+ #[inline]\n+ fn bar(...) {{\n      ...\n  }}\n"
    );
    assert!(
        matches!(compile(&hunk_of(&ok)).unwrap().kind, PatternKind::Fn { .. }),
        "literal attribute mirrors must keep compiling"
    );
}

#[test]
fn unknown_root_values_rejected() {
    // T110: only item/expr/stmt/type have a force path; anything else
    // must fail fast instead of silently falling through to heuristic
    // routing (SPEC 2.6).
    for bad in ["pat", "impl_item", "trait_item", "bogus"] {
        let patch = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root={bad}\n- foo\n+ bar\n"
        );
        let err = compile(&hunk_of(&patch)).unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.render().contains("unknown `# root=` value"),
            "missing needle for root={bad}: {}",
            err.render()
        );
    }
    // Supported roots still route.
    let ok = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=expr\n# expr E\n- foo(E)\n+ bar(E)\n",
    );
    assert!(compile(&ok).is_ok());
}

#[test]
fn unsupported_metavar_kinds_rejected() {
    // T111: identifier/path/lifetime/field have no engine binding; they
    // must fail fast instead of parsing as generic placeholders (SPEC 2.6).
    for kind in ["identifier", "path", "lifetime", "field"] {
        let patch = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# {kind} X\n- foo(X)\n+ bar(X)\n"
        );
        let err = compile(&hunk_of(&patch)).unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.render().contains("is not supported in v0.1"),
            "missing needle for kind={kind}: {}",
            err.render()
        );
    }
    // Supported-kind controls still compile (incl. structural S/M/TT).
    for decl in ["# expr E", "# stmt S", "# macro M", "# token_tree TT"] {
        let patch = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n{decl}\n- foo\n+ bar\n"
        );
        assert!(compile(&hunk_of(&patch)).is_ok(), "decl {decl} broke");
    }
}

#[test]
fn repeat_non_call_shape_rejected() {
    // T20: `# repeat` is a call/method-call feature; T126 extended it
    // to general-expression shapes. On any other shape it would be a
    // silent no-op, so it fails fast (SPEC 2.6, 17.8, 24.4). Pairs
    // differ so the shape needle fires (not an earlier same-name error).
    for (minus, plus, needle) in [
        (
            "struct Old",
            "struct New",
            "`# repeat` requires a call/method-call or general-expression",
        ),
        (
            "let x = E;",
            "let y = E;",
            "`# repeat` is not supported on `let`-statement patterns in v1",
        ),
        (
            "mac!(E)",
            "mac2!(E)",
            "`# repeat` requires a call/method-call or general-expression",
        ),
    ] {
        let patch = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n# expr E\n- {minus}\n+ {plus}\n"
        );
        // `mac!(E)` routes to the macro matcher; the rest are item /
        // general shapes. All must reject `# repeat` with a
        // shape-specific needle.
        let err = compile(&hunk_of(&patch)).unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.render().contains(needle),
            "missing needle {needle:?} for {minus:?}: {}",
            err.render()
        );
    }
    // Type shape rejects with its own needle (needs a `# type`
    // declaration to reach the repeat gate).
    let ty = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n# type T\n- Vec<T>\n+ Box<T>\n",
    );
    let err = compile(&ty).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render()
            .contains("`# repeat` is not supported on type patterns in v1"),
        "missing type needle: {}",
        err.render()
    );
    // Function shape (shared context lines, like `compiles_fn_rename`).
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
    );
    let err = compile(&h).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render()
            .contains("`# repeat` requires a call/method-call or general-expression"),
        "missing fn needle: {}",
        err.render()
    );
    let ok = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n# expr E\n- foo(E)\n+ bar(E)\n",
    );
    assert!(compile(&ok).is_ok());
    // T126: general-expression shapes accept `# repeat`.
    let ok_expr = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n# expr E\n- E + 1\n+ E + 2\n",
    );
    assert!(compile(&ok_expr).is_ok());
}

#[test]
fn trait_context_routes_to_method_context() {
    // T182: a `trait` context header plus a fn pair routes to the shared
    // method-context kind (same variant as `impl`); `// trait` comments
    // and `dyn Trait` bounds never route.
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# lang=rust\n  trait T {\n      ...\n-     fn foo();\n+     fn bar();\n      ...\n  }\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(c.kind, PatternKind::ImplContext { .. }),
        "got: {:?}",
        c.kind
    );
    let h = hunk_of(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- fn foo();\n+ fn bar();\n",
    );
    let c = compile(&h).unwrap();
    assert!(
        matches!(c.kind, PatternKind::Fn { .. }),
        "got: {:?}",
        c.kind
    );
}

#[test]
fn returns_none_accepted_in_method_context() {
    // T209: the `# returns=none` gate used to admit only `PatternKind::Fn`,
    // but impl/trait-context methods share the slot planner and the engine
    // already enforces the constraint there — the gate was the only hole.
    for header in ["  impl S {", "  trait T {"] {
        let h = hunk_of(
            &format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# lang=rust\n# returns=none\n{header}\n      ...\n-     fn foo() {{\n+     fn bar() {{\n          ...\n      }}\n  }}\n"
            ),
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(c.kind, PatternKind::ImplContext { .. }),
            "header {header:?} got: {:?}",
            c.kind
        );
    }
}

#[test]
fn trait_header_ident_cases() {
    // T182: ident extraction for the engine header check — generics,
    // bounds, and `unsafe` strip; non-headers yield `None`.
    use rsmpl_pattern::{is_trait_header, trait_header_ident};
    assert_eq!(trait_header_ident("trait T {"), Some("T".to_string()));
    assert_eq!(
        trait_header_ident("unsafe trait T<U>: Clone {"),
        Some("T".to_string())
    );
    assert_eq!(
        trait_header_ident("trait T: Clone {"),
        Some("T".to_string())
    );
    assert_eq!(trait_header_ident("impl S {"), None);
    assert_eq!(trait_header_ident("#[cfg(x)]"), None);
    assert!(is_trait_header("  unsafe trait T {"));
    assert!(!is_trait_header("  // trait T {"));
    assert!(!is_trait_header("  impl S {"));
}
