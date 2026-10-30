//! Block/value arms for the general matcher (WI-6 slice 1).
//!
//! Split from `matcher_general` per D5 (the parent file sits at the
//! 2000-line gate). Owns the structural matching for `async` blocks,
//! `try` blocks, `unsafe` blocks, and `yield` expressions:
//!
//! - `async [move] { ... }`: the `move` qualifier compares strictly (a
//!   different qualifier is a different closure-like construct, mirroring
//!   the `match_closure` posture); the body matches structurally via
//!   `match_block` (trailing `...` rides along).
//! - `try { ... }`: the body matches structurally via `match_block`.
//! - `unsafe { ... }`: the body matches structurally via `match_block`.
//! - `yield` / `yield E`: presence of the value must agree; a present
//!   value matches via the general expression matcher.
//!
//! Everything else still rejects in `match_expr` (macro types stay
//! ordinary hunks, SPEC 21 stands).

use super::matcher_general::{opt_expr_eq, render, MatchCx};
use crate::matcher_general_ctrl::match_block;

/// Match an `async [move] { ... }` block: qualifier strict, body
/// structural.
pub(crate) fn match_async(pat: &syn::ExprAsync, tgt: &syn::ExprAsync, cx: &mut MatchCx) -> bool {
    render(&pat.capture) == render(&tgt.capture) && match_block(&pat.block, &tgt.block, cx)
}

/// Match a `try { ... }` block: body structural.
pub(crate) fn match_try_block(
    pat: &syn::ExprTryBlock,
    tgt: &syn::ExprTryBlock,
    cx: &mut MatchCx,
) -> bool {
    match_block(&pat.block, &tgt.block, cx)
}

/// Match an `unsafe { ... }` block: body structural.
pub(crate) fn match_unsafe(pat: &syn::ExprUnsafe, tgt: &syn::ExprUnsafe, cx: &mut MatchCx) -> bool {
    match_block(&pat.block, &tgt.block, cx)
}

/// Match `yield` / `yield E`: arity must agree, value structural.
pub(crate) fn match_yield(pat: &syn::ExprYield, tgt: &syn::ExprYield, cx: &mut MatchCx) -> bool {
    opt_expr_eq(&pat.expr, &tgt.expr, cx)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rsmpl_patch::parse_patch;

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        let patch = parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files["src/lib.rs"].clone())
    }

    const HEAD: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";

    #[test]
    fn async_body_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- async {{ foo(E) }}\n+ async {{ bar(E) }}\n"
        );
        let src = "fn a(x: i32) {\n    let v = async { foo(x) };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("async { bar(x) }"), "got: {out}");
    }

    #[test]
    fn async_move_qualifier_mismatch_vetoes() {
        // `async move` never matches plain `async` (qualifier-strict).
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- async move {{ foo(E) }}\n+ async move {{ bar(E) }}\n"
        );
        let src = "fn a(x: i32) {\n    let v = async { foo(x) };\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn unsafe_body_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- unsafe {{ foo(E) }}\n+ unsafe {{ bar(E) }}\n"
        );
        let src = "fn a(x: i32) {\n    let v = unsafe { foo(x) };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("unsafe { bar(x) }"), "got: {out}");
    }

    #[test]
    fn yield_value_hole_rewrites() {
        let patch =
            format!("{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- yield E\n+ yield E + 1\n");
        let src = "fn a(x: i32) {\n    yield x;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("yield x + 1"), "got: {out}");
    }

    #[test]
    fn yield_arity_mismatch_vetoes() {
        // Bare `yield` never matches `yield E` (and vice versa).
        let patch =
            format!("{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- yield E\n+ yield E + 1\n");
        let src = "fn a() {\n    yield;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn try_block_body_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- try {{ foo(E) }}\n+ try {{ bar(E) }}\n"
        );
        let src = "fn a(x: i32) {\n    let v = try { foo(x) };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("try { bar(x) }"), "got: {out}");
    }

    #[test]
    fn async_trailing_ellipsis_replays_tail() {
        // Block bodies ride `match_block`, so trailing `...` replays.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- async {{ foo(E); ... }}\n+ async {{ bar(E); ... }}\n"
        );
        let src = "fn a(x: i32) {\n    let v = async { foo(x); extra(); };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("bar(x);"), "got: {out}");
        assert!(out.contains("extra();"), "got: {out}");
        assert!(!out.contains("foo(x);"), "got: {out}");
    }
}
