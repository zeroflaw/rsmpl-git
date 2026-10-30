//! T171: pattern-compilation integration tests, moved verbatim out of
//! `src/pattern.rs` (which sat at 1977/2000 LOC under the
//! `no_rust_file_exceeds_2000_lines` repo cap).
//!
//! Each `tests/*.rs` file is its own crate, so the `hunk_of` helper is
//! duplicated here, not shared (split-file precedent: `ellipsis_e2e.rs`).

use rsmpl_patch::{parse_patch, Hunk, RsmplHunk};
use rsmpl_pattern::{
    compile, split_leading_attrs, ArgPat, ExprPatternKind, GeneralShape, PatternKind,
};

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

// WI-5b: trailing block `...` compiles in Stmt and Expr shapes; every
// other `...` position stays C1 (plus-without-minus and count-mismatch
// carry their own C1 reasons; types never admit `...`).
#[test]
fn trailing_block_ellipsis_compiles() {
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    // Stmt shape (`let` init) with replay.
    let stmt = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- let v = if E {{ foo(); ... }} else {{ 0 }};\n+ let v = if E {{ bar(); ... }} else {{ 0 }};\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&stmt)).unwrap().kind,
            PatternKind::General { .. }
        ),
        "stmt trailing ... must compile"
    );
    // Expr shape with drop (no plus `...`).
    let drop = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- if E {{ foo(); ... }}\n+ if E {{ bar(); }}\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&drop)).unwrap().kind,
            PatternKind::General { .. }
        ),
        "drop must compile"
    );
    // A `"..."` string literal is concrete text, never a wildcard: the
    // call arg classifies as Concrete, not Ellipsis.
    let lit = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- log(E, \"...\");\n+ log(E, \"...\");\n"
    );
    match compile(&hunk_of(&lit)).unwrap().kind {
        PatternKind::Expr {
            minus: ExprPatternKind::Call { args, .. },
            ..
        } => assert!(
            args.iter().all(|a| !matches!(a, ArgPat::Ellipsis)),
            "`\"...\"` must stay a concrete arg, got: {args:?}"
        ),
        other => panic!("expected Call, got: {other:?}"),
    }
    // Mid-block `...` stays C1.
    let mid = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- if E {{ ... foo(); }}\n+ if E {{ ... bar(); }}\n"
    );
    let err = compile(&hunk_of(&mid)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render().contains("only as trailing block statements"),
        "mid-block ... must stay C1:\n{}",
        err.render()
    );
    // Plus `...` with no minus `...` names the replay fence.
    let orphan = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- if E {{ foo(); }}\n+ if E {{ bar(); ... }}\n"
    );
    let err = compile(&hunk_of(&orphan)).unwrap_err();
    assert!(
        err.render().contains("no matching `- ...`"),
        "plus-without-minus must name the fence:\n{}",
        err.render()
    );
    // Partial plus count fails loud.
    let cnt = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- if E {{ foo(); ... }} else {{ bar(); ... }}\n+ if E {{ foo2(); ... }} else {{ bar2(); }}\n"
    );
    let err = compile(&hunk_of(&cnt)).unwrap_err();
    assert!(
        err.render().contains("count differs"),
        "count mismatch must fail loud:\n{}",
        err.render()
    );
    // Types never admit `...`.
    let ty = format!("{head}@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# type T\n- Vec<...>\n+ Vec<T>\n");
    let err = compile(&hunk_of(&ty)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render().contains("only as trailing block statements"),
        "type ... must stay C1:\n{}",
        err.render()
    );
    // Match-arm `...` is not a block tail.
    let arm = format!(
        "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- match E {{ ... }}\n+ match E {{ ... }}\n"
    );
    let err = compile(&hunk_of(&arm)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render().contains("only as trailing block statements"),
        "arm ... must stay C1:\n{}",
        err.render()
    );
}

// WI-6b: the `...` position validator walks pattern lines byte-wise —
// multi-byte text on the same line (an em dash, CJK ident, emoji)
// must slice on char boundaries, never panic inside a codepoint.
#[test]
fn ellipsis_check_is_char_boundary_safe() {
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let patch = format!(
        "{head}@@ -2,1 @@ rule=x\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); ... }} // caf\u{00e9} \u{2014} done\n+ if E {{ bar(); ... }} // caf\u{00e9} \u{2014} done\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&patch)).unwrap().kind,
            PatternKind::General { .. }
        ),
        "unicode beside ... must compile, not panic"
    );
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
        // C1: `...` in a general shape (WI-5b: only trailing block
        // statements are admitted; a binary-operand `...` still fails).
        (
            format!(
                "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + ...\n+ E\n"
            ),
            "v1 general patterns support `...` only as trailing block statements",
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
    // WI-9c(c) + WI-5c narrowing: a minus side that parses as two or more
    // Rust statements outside the 2–4 plain-`let` sequence shape fails
    // fast at compile time naming the pattern instead of degrading to
    // `Raw` and dying at match with a misleading "no match found".
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let patch = format!(
        "{head}@@ -2,2 @@ rule=x\n# lang=rust\n# allow=many\n- let a = foo(E);\n- bar(E);\n+ let a = foo2(E);\n+ bar2(E);\n"
    );
    let err = compile(&hunk_of(&patch)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render()
            .contains("v1 statement sequences need 2–4 plain `let` statements"),
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
fn plain_let_pair_compiles_to_sequence() {
    // WI-5c: two plain `let` statements compile to `GeneralShape::Sequence`
    // (the C18 pin above covers the still-rejected shapes).
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let patch = format!(
        "{head}@@ -2,2 @@ rule=x\n# lang=rust\n# allow=many\n- let a = foo(E);\n- let b = bar(E);\n+ let a = foo2(E);\n+ let b = bar2(E);\n"
    );
    assert!(
        matches!(
            compile(&hunk_of(&patch)).unwrap().kind,
            PatternKind::General {
                shape: GeneralShape::Sequence,
                ..
            }
        ),
        "two plain lets should be General::Sequence"
    );
    // Five lets exceed K ≤ 4 — still C18.
    let patch = format!(
        "{head}@@ -2,5 @@ rule=x\n# lang=rust\n# allow=many\n- let a = 1;\n- let b = 2;\n- let c = 3;\n- let d = 4;\n- let e = 5;\n+ let a = 6;\n+ let b = 7;\n+ let c = 8;\n+ let d = 9;\n+ let e = 0;\n"
    );
    let err = compile(&hunk_of(&patch)).unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render().contains("parses as 5 Rust statements"),
        "missing count:\n{}",
        err.render()
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

#[test]
fn evolution_template_accepts_restricted_visibility() {
    // WI-history-70d72c0: `diffrs` emits `pub(crate)`-prefixed evolution
    // pastes for vis-only fn moves; the template gate must accept every
    // valid Rust vis/qualifier prefix, not just bare `pub`.
    use rsmpl_pattern::parse_evolution_template;
    for prefix in [
        "",
        "pub",
        "pub(crate)",
        "pub(super)",
        "pub(self)",
        "pub(in crate::x)",
        "pub(crate) async",
        "pub unsafe",
    ] {
        let tmpl = if prefix.is_empty() {
            "fn new(...) { ... }".to_string()
        } else {
            format!("{prefix} fn new(...) {{ ... }}")
        };
        let (pre, name) =
            parse_evolution_template(&tmpl).unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert_eq!(name, "new", "got: {tmpl:?}");
        assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
    }
    // Garbage prefixes still rejected; missing holes still rejected
    // (`(...)` carries the `...` substring, so the params hole is the
    // effective body gate — pre-existing behavior, unchanged here).
    assert!(parse_evolution_template("foo( fn new(...) { ... }").is_none());
    assert!(parse_evolution_template("a;b fn new(...) { ... }").is_none());
    assert!(parse_evolution_template("pub fn new(x: i32)").is_none());
    assert!(parse_evolution_template("pub(crate) fn new(x: i32)").is_none());
}

#[test]
fn fn_full_header_template_accepts_concrete_signatures() {
    // WI-8 slice 9: signature-evolving moves paste the plus-side header
    // concretely (`+ pub fn new(x: i64) -> i64 { ... }`). The parser
    // accepts fully spelled single-line headers and declines the
    // abstract `(...)` form (owned by `parse_evolution_template`),
    // marker-less lines, multi-line text, and bad prefixes.
    use rsmpl_pattern::parse_fn_full_header_template;
    for tmpl in [
        "fn new(x: i64) -> i64 { ... }",
        "pub fn new(x: i64) -> i64 { ... }",
        "pub(crate) async fn new<T>(x: T) -> T where T: Clone { ... }",
        "fn old() { ... }",
    ] {
        let (pre, name) =
            parse_fn_full_header_template(tmpl).unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert!(!name.is_empty(), "got: {tmpl:?}");
        assert!(tmpl.starts_with(pre.trim()), "got: {tmpl:?}");
    }
    let (pre, name) = parse_fn_full_header_template("pub fn new(x: i64) -> i64 { ... }").unwrap();
    assert_eq!((pre.trim(), name.as_str()), ("pub", "new"));
    // The abstract form belongs to the abstract parser.
    assert!(parse_fn_full_header_template("pub fn new(...) { ... }").is_none());
    assert!(parse_fn_full_header_template("fn new(...) { ... }").is_none());
    // Marker-less concrete lines decline (they stay loud downstream).
    assert!(parse_fn_full_header_template("pub fn new(x: i32)").is_none());
    assert!(parse_fn_full_header_template("pub fn new(x: i32) {}").is_none());
    assert!(parse_fn_full_header_template("pub fn new(x: i32) -> i32 { }").is_none());
    // Multi-line text and bad prefixes decline.
    assert!(parse_fn_full_header_template("pub fn new(\n    x: i32,\n) -> i32 { ... }").is_none());
    assert!(parse_fn_full_header_template("foo( fn new(x: i32) { ... }").is_none());
    assert!(parse_fn_full_header_template("fn (x: i32) { ... }").is_none());
    // Non-`fn` templates never cross-accept.
    assert!(parse_fn_full_header_template("pub struct New { ... }").is_none());
}

#[test]
fn struct_evolution_template_accepts_visibility() {
    // WI-8 slice 1: `struct` evolution pastes accept visibility
    // prefixes and require a `...` body marker.
    use rsmpl_pattern::parse_struct_evolution_template;
    for prefix in ["", "pub", "pub(crate)", "pub(super)", "pub(in crate::x)"] {
        let tmpl = if prefix.is_empty() {
            "struct New { ... }".to_string()
        } else {
            format!("{prefix} struct New {{ ... }}")
        };
        let (pre, name) = parse_struct_evolution_template(&tmpl)
            .unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert_eq!(name, "New", "got: {tmpl:?}");
        assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
    }
    // `fn` templates are a different parser — never cross-accept.
    assert!(parse_struct_evolution_template("pub fn new(...) { ... }").is_none());
    // Missing marker, garbage prefix, multi-line all rejected.
    assert!(parse_struct_evolution_template("pub struct New").is_none());
    assert!(parse_struct_evolution_template("a;b struct New { ... }").is_none());
    assert!(parse_struct_evolution_template("pub struct New {\n ... \n}").is_none());
}

#[test]
fn enum_evolution_template_accepts_visibility() {
    // WI-8 slice 2: `enum` evolution pastes accept visibility
    // prefixes and require a `...` body marker.
    use rsmpl_pattern::parse_enum_evolution_template;
    for prefix in ["", "pub", "pub(crate)", "pub(super)", "pub(in crate::x)"] {
        let tmpl = if prefix.is_empty() {
            "enum New { ... }".to_string()
        } else {
            format!("{prefix} enum New {{ ... }}")
        };
        let (pre, name) =
            parse_enum_evolution_template(&tmpl).unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert_eq!(name, "New", "got: {tmpl:?}");
        assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
    }
    // `fn`/`struct` templates are different parsers — never cross-accept.
    assert!(parse_enum_evolution_template("pub fn new(...) { ... }").is_none());
    assert!(parse_enum_evolution_template("pub struct New { ... }").is_none());
    // Missing marker, garbage prefix, multi-line all rejected.
    assert!(parse_enum_evolution_template("pub enum New").is_none());
    assert!(parse_enum_evolution_template("a;b enum New { ... }").is_none());
    assert!(parse_enum_evolution_template("pub enum New {\n ... \n}").is_none());
}

#[test]
fn union_evolution_template_accepts_visibility() {
    // WI-8 slice 3: `union` evolution pastes accept visibility
    // prefixes and require a `...` body marker.
    use rsmpl_pattern::parse_union_evolution_template;
    for prefix in ["", "pub", "pub(crate)", "pub(super)", "pub(in crate::x)"] {
        let tmpl = if prefix.is_empty() {
            "union New { ... }".to_string()
        } else {
            format!("{prefix} union New {{ ... }}")
        };
        let (pre, name) =
            parse_union_evolution_template(&tmpl).unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert_eq!(name, "New", "got: {tmpl:?}");
        assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
    }
    // `fn`/`struct`/`enum` templates are different parsers — never cross-accept.
    assert!(parse_union_evolution_template("pub fn new(...) { ... }").is_none());
    assert!(parse_union_evolution_template("pub struct New { ... }").is_none());
    assert!(parse_union_evolution_template("pub enum New { ... }").is_none());
    // Missing marker, garbage prefix, multi-line all rejected.
    assert!(parse_union_evolution_template("pub union New").is_none());
    assert!(parse_union_evolution_template("a;b union New { ... }").is_none());
    assert!(parse_union_evolution_template("pub union New {\n ... \n}").is_none());
}

#[test]
fn type_evolution_template_accepts_visibility() {
    // WI-8 slice 4: `type` evolution pastes accept visibility
    // prefixes and require an `= ...;` body marker.
    use rsmpl_pattern::parse_type_evolution_template;
    for prefix in ["", "pub", "pub(crate)", "pub(super)", "pub(in crate::x)"] {
        let tmpl = if prefix.is_empty() {
            "type New = ...;".to_string()
        } else {
            format!("{prefix} type New = ...;")
        };
        let (pre, name) =
            parse_type_evolution_template(&tmpl).unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert_eq!(name, "New", "got: {tmpl:?}");
        assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
    }
    // `fn`/`struct`/`enum`/`union` templates are different parsers — never cross-accept.
    assert!(parse_type_evolution_template("pub fn new(...) { ... }").is_none());
    assert!(parse_type_evolution_template("pub struct New { ... }").is_none());
    assert!(parse_type_evolution_template("pub enum New { ... }").is_none());
    assert!(parse_type_evolution_template("pub union New { ... }").is_none());
    assert!(parse_type_evolution_template("pub const New = ...;").is_none());
    // Missing marker, missing `=`, garbage prefix, multi-line all rejected.
    assert!(parse_type_evolution_template("pub type New").is_none());
    assert!(parse_type_evolution_template("pub type New ...;").is_none());
    assert!(parse_type_evolution_template("a;b type New = ...;").is_none());
    assert!(parse_type_evolution_template("pub type New = ...;\nfoo").is_none());
}

#[test]
fn const_evolution_template_accepts_visibility() {
    // WI-8 slice 5: `const` evolution pastes accept visibility
    // prefixes and require an `= ...;` body marker.
    use rsmpl_pattern::parse_const_evolution_template;
    for prefix in ["", "pub", "pub(crate)", "pub(super)", "pub(in crate::x)"] {
        let tmpl = if prefix.is_empty() {
            "const New = ...;".to_string()
        } else {
            format!("{prefix} const New = ...;")
        };
        let (pre, name) =
            parse_const_evolution_template(&tmpl).unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert_eq!(name, "New", "got: {tmpl:?}");
        assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
    }
    // `fn`/`struct`/`enum`/`union`/`type` templates are different parsers — never cross-accept.
    assert!(parse_const_evolution_template("pub fn new(...) { ... }").is_none());
    assert!(parse_const_evolution_template("pub struct New { ... }").is_none());
    assert!(parse_const_evolution_template("pub enum New { ... }").is_none());
    assert!(parse_const_evolution_template("pub union New { ... }").is_none());
    assert!(parse_const_evolution_template("pub type New = ...;").is_none());
    assert!(parse_const_evolution_template("pub static New = ...;").is_none());
    assert!(parse_const_evolution_template("pub static mut New = ...;").is_none());
    assert!(parse_const_evolution_template("pub mod New { ... }").is_none());
    assert!(parse_const_evolution_template("pub unsafe mod New { ... }").is_none());
    assert!(parse_const_evolution_template("pub trait New { ... }").is_none());
    assert!(parse_const_evolution_template("pub unsafe trait New { ... }").is_none());
    // Missing marker, missing `=`, garbage prefix, multi-line all rejected.
    assert!(parse_const_evolution_template("pub const New").is_none());
    assert!(parse_const_evolution_template("pub const New ...;").is_none());
    assert!(parse_const_evolution_template("a;b const New = ...;").is_none());
    assert!(parse_const_evolution_template("pub const New = ...;\nfoo").is_none());
}

#[test]
fn static_evolution_template_accepts_visibility_and_mut() {
    // WI-8 slice 6: `static` evolution pastes accept visibility
    // prefixes, an optional `mut`, and require an `= ...;` body marker.
    use rsmpl_pattern::parse_static_evolution_template;
    for prefix in ["", "pub", "pub(crate)", "pub(super)", "pub(in crate::x)"] {
        for (suffix, want_mut) in [("", false), ("mut ", true)] {
            let tmpl = if prefix.is_empty() {
                format!("static {suffix}New = ...;")
            } else {
                format!("{prefix} static {suffix}New = ...;")
            };
            let (pre, name, is_mut) = parse_static_evolution_template(&tmpl)
                .unwrap_or_else(|| panic!("must accept {tmpl:?}"));
            assert_eq!(name, "New", "got: {tmpl:?}");
            assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
            assert_eq!(is_mut, want_mut, "got: {tmpl:?}");
        }
    }
    // `fn`/`struct`/`enum`/`union`/`type`/`const` templates are different
    // parsers — never cross-accept.
    assert!(parse_static_evolution_template("pub fn new(...) { ... }").is_none());
    assert!(parse_static_evolution_template("pub struct New { ... }").is_none());
    assert!(parse_static_evolution_template("pub enum New { ... }").is_none());
    assert!(parse_static_evolution_template("pub union New { ... }").is_none());
    assert!(parse_static_evolution_template("pub type New = ...;").is_none());
    assert!(parse_static_evolution_template("pub const New = ...;").is_none());
    // Missing marker, missing `=`, garbage prefix, multi-line all rejected.
    assert!(parse_static_evolution_template("pub static New").is_none());
    assert!(parse_static_evolution_template("pub static mut New").is_none());
    assert!(parse_static_evolution_template("pub static New ...;").is_none());
    assert!(parse_static_evolution_template("a;b static New = ...;").is_none());
    assert!(parse_static_evolution_template("pub static New = ...;\nfoo").is_none());
    // `mutate` is an ident, not `mut` + `ate`: no false `mut` split.
    let (pre, name, is_mut) =
        parse_static_evolution_template("pub static mutate = ...;").expect("must accept");
    assert_eq!(
        (pre.trim(), name.as_str(), is_mut),
        ("pub", "mutate", false)
    );
}
#[test]
fn mod_evolution_template_accepts_visibility_and_unsafe() {
    // WI-8 slice 7: `mod` evolution pastes accept visibility
    // prefixes, an optional `unsafe`, and require a `...` body marker.
    use rsmpl_pattern::parse_mod_evolution_template;
    for prefix in ["", "pub", "pub(crate)", "pub unsafe", "pub(crate) unsafe"] {
        let tmpl = if prefix.is_empty() {
            "mod New { ... }".to_string()
        } else {
            format!("{prefix} mod New {{ ... }}")
        };
        let (pre, name) =
            parse_mod_evolution_template(&tmpl).unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert_eq!(name, "New", "got: {tmpl:?}");
        assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
    }
    // `fn`/`struct`/`enum`/`union`/`type`/`const`/`static` templates are
    // different parsers — never cross-accept.
    assert!(parse_mod_evolution_template("pub fn new(...) { ... }").is_none());
    assert!(parse_mod_evolution_template("pub struct New { ... }").is_none());
    assert!(parse_mod_evolution_template("pub enum New { ... }").is_none());
    assert!(parse_mod_evolution_template("pub union New { ... }").is_none());
    assert!(parse_mod_evolution_template("pub type New = ...;").is_none());
    assert!(parse_mod_evolution_template("pub const New = ...;").is_none());
    assert!(parse_mod_evolution_template("pub static New = ...;").is_none());
    assert!(parse_mod_evolution_template("pub static mut New = ...;").is_none());
    // Missing marker, garbage prefix, multi-line all rejected.
    assert!(parse_mod_evolution_template("pub mod New").is_none());
    assert!(parse_mod_evolution_template("pub mod New { }").is_none());
    assert!(parse_mod_evolution_template("a;b mod New { ... }").is_none());
    assert!(parse_mod_evolution_template("pub mod New { ... }\nfoo").is_none());
    // `trait` templates never cross-accept the `mod` parser (and vice versa).
    assert!(parse_mod_evolution_template("pub trait New { ... }").is_none());
    assert!(parse_mod_evolution_template("pub unsafe trait New { ... }").is_none());
}

#[test]
fn trait_evolution_template_accepts_visibility_and_unsafe() {
    // WI-8 slice 8: `trait` evolution pastes accept visibility
    // prefixes, an optional `unsafe`, and require a `...` body marker.
    use rsmpl_pattern::parse_trait_evolution_template;
    for prefix in ["", "pub", "pub(crate)", "pub unsafe", "pub(crate) unsafe"] {
        let tmpl = if prefix.is_empty() {
            "trait New { ... }".to_string()
        } else {
            format!("{prefix} trait New {{ ... }}")
        };
        let (pre, name) =
            parse_trait_evolution_template(&tmpl).unwrap_or_else(|| panic!("must accept {tmpl:?}"));
        assert_eq!(name, "New", "got: {tmpl:?}");
        assert_eq!(pre.trim(), prefix, "got: {tmpl:?}");
    }
    // `fn`/`struct`/`enum`/`union`/`type`/`const`/`static`/`mod` templates are
    // different parsers — never cross-accept.
    assert!(parse_trait_evolution_template("pub fn new(...) { ... }").is_none());
    assert!(parse_trait_evolution_template("pub struct New { ... }").is_none());
    assert!(parse_trait_evolution_template("pub enum New { ... }").is_none());
    assert!(parse_trait_evolution_template("pub union New { ... }").is_none());
    assert!(parse_trait_evolution_template("pub type New = ...;").is_none());
    assert!(parse_trait_evolution_template("pub const New = ...;").is_none());
    assert!(parse_trait_evolution_template("pub static New = ...;").is_none());
    assert!(parse_trait_evolution_template("pub static mut New = ...;").is_none());
    assert!(parse_trait_evolution_template("pub mod New { ... }").is_none());
    assert!(parse_trait_evolution_template("pub unsafe mod New { ... }").is_none());
    // Missing marker, garbage prefix, multi-line all rejected.
    assert!(parse_trait_evolution_template("pub trait New").is_none());
    assert!(parse_trait_evolution_template("pub trait New { }").is_none());
    assert!(parse_trait_evolution_template("a;b trait New { ... }").is_none());
    assert!(parse_trait_evolution_template("pub trait New { ... }\nfoo").is_none());
}

// Repair-keying follow-up: no `PatternConflict` repair echoes its own
// reason — `write_instead` is always a concrete, non-empty template an
// agent can apply (SPEC D.2). Covers the keyed fn/item/use gates plus
// the generic compile-boundary fallback (declared-vs-used kind
// disagreement, which carries no gate-specific repair).
#[test]
fn keyed_repairs_never_echo_the_reason() {
    let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
    let patches = [
        // fn trailing-tokens gate (the REVIEW gap 1 example).
        format!(
            "{head}@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- fn foo(...)\n+ fn bar(...)\n      ...\n"
        ),
        // fn missing-parameter-list gate.
        format!(
            "{head}@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n- fn foo<T> {{\n+ fn bar<T> {{\n      ...\n  }}\n"
        ),
        // item concrete-shape gate.
        format!(
            "{head}@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct Old {{ x: i32 }}\n+ struct New {{ x: i32 }}\n"
        ),
        // use metavariable gate.
        format!(
            "{head}@@ -1,1 +1,1 @@ rule=x\n# allow=many\n# expr M\n- use crate::M\n+ use crate::new\n"
        ),
        // generic fallback: no gate-specific repair exists.
        format!("{head}@@ -1,1 +1,1 @@ rule=x\n# expr T\n- Vec<T>\n+ Box<[T]>\n"),
    ];
    let codes = ["fn-pattern", "fn-pattern", "pattern", "pattern", "pattern"];
    for (patch, code) in patches.iter().zip(codes) {
        let err = compile(&hunk_of(patch)).unwrap_err();
        assert_eq!(
            err.code,
            rsmpl_patch::ErrorCode::PatternConflict,
            "got: {patch:?}"
        );
        let repair = err.repair.expect("PatternConflict must carry repair");
        assert_eq!(
            repair["action"].as_str(),
            Some("fix_pattern"),
            "got: {patch:?}"
        );
        assert_eq!(repair["code"].as_str(), Some(code), "got: {patch:?}");
        let write_instead = repair["write_instead"].as_str().expect("template");
        assert!(!write_instead.is_empty(), "got: {patch:?}");
        assert_ne!(write_instead, err.message, "echoes reason: {patch:?}");
    }
    // The fallback template is the WI-2b generic filler text.
    let err = compile(&hunk_of(&patches[4])).unwrap_err();
    let repair = err.repair.expect("fallback must carry repair");
    assert_eq!(
        repair["write_instead"].as_str(),
        Some("rewrite the pattern to a supported shape (see SPEC 18.6) or use an ordinary hunk"),
    );
}
