//! Control-flow and statement arms for the general matcher (WI-5a).
//!
//! Split from `matcher_general` per D5 (the parent file sits at the
//! 2000-line gate). Owns the statement-level structural matching:
//! `match_local` (`let` with or without `let-else`, `P @ sub`),
//! `match_block` (concrete-length statement sequences), `match_stmt`,
//! `match_if`, and the `@`-subpattern helper `match_pat`.
//! The parent keeps `match_expr`/`match_type` plus collection,
//! substitution, and application; the two modules call each other
//! (circular within the crate is fine).
//!
//! Everything not listed here (`match`, loops, closures, `let`
//! expressions) still rejects in `match_expr` and stays an ordinary
//! hunk (WI-5b owns those arms).

use syn::spanned::Spanned;

use crate::edits::fragments_equal;

use super::matcher_general::{
    as_placeholder, match_expr, match_type, render, render_eq, single_placeholder_type, MatchCx,
};

/// Split a `let` pattern into (inner pattern, optional ascribed type).
fn decompose_pat(pat: &syn::Pat) -> (&syn::Pat, Option<&syn::Type>) {
    if let syn::Pat::Type(pt) = pat {
        (&pt.pat, Some(&pt.ty))
    } else {
        (pat, None)
    }
}

/// Match a `let` statement: bound pattern (whole placeholder,
/// concrete, `P: T`, or WI-5a `P @ sub`), ascribed-type agreement,
/// initializer, and `let-else` diverge (both sides present and
/// structurally equal, or both absent — mixed rejects).
pub(crate) fn match_local(pat: &syn::Local, tgt: &syn::Local, cx: &mut MatchCx) -> bool {
    let (pi, pty) = decompose_pat(&pat.pat);
    let (ti, tty) = decompose_pat(&tgt.pat);
    // Bound (inner) pattern.
    if let syn::Pat::Ident(pid) = pi {
        if let Some(name) = as_placeholder(&cx.reverse, &pid.ident.to_string()).cloned() {
            if let Some((_, sub)) = &pid.subpat {
                // WI-5a `P @ sub`: `P` binds the whole target pattern
                // text; `sub` matches recursively (a target `@` unwraps
                // to its subpattern, otherwise the whole target pattern).
                if !cx.bind_span(&name, ti.span()) {
                    return false;
                }
                let tsub: &syn::Pat = match ti {
                    syn::Pat::Ident(tid) => tid.subpat.as_ref().map(|(_, s)| &**s).unwrap_or(ti),
                    _ => ti,
                };
                if !match_pat(sub, tsub, cx) {
                    return false;
                }
            } else if !cx.bind_span(&name, ti.span()) {
                return false;
            }
        } else if !render_eq(pi, ti) {
            return false;
        }
    } else if !fragments_equal(&render(&pat.pat), &render(&tgt.pat)) {
        return false;
    }
    // Ascribed-type agreement (strict: both absent, or both present and
    // matching). A bare `P` never silently drops an annotation.
    match (pty, tty) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            if let Some(name) = single_placeholder_type(a, cx) {
                if !cx.bind_span(&name, b.span()) {
                    return false;
                }
            } else if !match_type(a, b, cx) {
                return false;
            }
        }
        _ => return false,
    }
    // Initializer.
    match (&pat.init, &tgt.init) {
        (Some(a), Some(b)) => {
            if !match_expr(&a.expr, &b.expr, cx) {
                return false;
            }
            match (&a.diverge, &b.diverge) {
                (None, None) => true,
                // Diverge exprs route through the general matcher: a
                // `Block`/`if` pair matches structurally (WI-5a), anything
                // else rejects — mixed presence rejects too.
                (Some((_, da)), Some((_, db))) => match_expr(da, db, cx),
                _ => false,
            }
        }
        (None, None) => true,
        _ => false,
    }
}

/// Match two statement sequences: lengths must be equal (concrete —
/// this is not ellipsis), then each member matches structurally.
pub(crate) fn match_block(pat: &syn::Block, tgt: &syn::Block, cx: &mut MatchCx) -> bool {
    pat.stmts.len() == tgt.stmts.len()
        && pat
            .stmts
            .iter()
            .zip(tgt.stmts.iter())
            .all(|(a, b)| match_stmt(a, b, cx))
}

/// Match one block member statement.
fn match_stmt(pat: &syn::Stmt, tgt: &syn::Stmt, cx: &mut MatchCx) -> bool {
    match (pat, tgt) {
        (syn::Stmt::Local(a), syn::Stmt::Local(b)) => match_local(a, b, cx),
        (syn::Stmt::Expr(a, a_semi), syn::Stmt::Expr(b, b_semi)) => {
            a_semi.is_some() == b_semi.is_some() && match_expr(a, b, cx)
        }
        // Items compare strictly token-equal: a block that renames an
        // item is two changes, never guessed here.
        (syn::Stmt::Item(a), syn::Stmt::Item(b)) => render_eq(a, b),
        // Macro bodies stay opaque (SPEC 21 stands).
        _ => false,
    }
}

/// Match an `if` expression: condition via the general expression
/// matcher, then-block structurally, else branch recursively (covers
/// both `else { }` blocks and `else if` chains).
pub(crate) fn match_if(pat: &syn::ExprIf, tgt: &syn::ExprIf, cx: &mut MatchCx) -> bool {
    if !match_expr(&pat.cond, &tgt.cond, cx) {
        return false;
    }
    if !match_block(&pat.then_branch, &tgt.then_branch, cx) {
        return false;
    }
    match (&pat.else_branch, &tgt.else_branch) {
        (None, None) => true,
        (Some((_, a)), Some((_, b))) => match_expr(a, b, cx),
        _ => false,
    }
}

/// Match a `let` subpattern under `P @ sub`: a whole-pattern
/// placeholder binds the target pattern text; anything else must be
/// strictly token-equal (never guessed).
pub(crate) fn match_pat(pat: &syn::Pat, tgt: &syn::Pat, cx: &mut MatchCx) -> bool {
    if let syn::Pat::Ident(pid) = pat {
        if pid.subpat.is_none() {
            if let Some(name) = as_placeholder(&cx.reverse, &pid.ident.to_string()).cloned() {
                return cx.bind_span(&name, tgt.span());
            }
        }
    }
    render_eq(pat, tgt)
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
    fn if_condition_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- if E {{ 1 }} else {{ 2 }}\n+ if E {{ 1 }} else {{ 3 }}\n"
        );
        let src = "fn a(x: bool) {\n    let v = if x { 1 } else { 2 };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("else { 3 }"), "got: {out}");
    }

    #[test]
    fn if_field_divergence_vetoes() {
        // Then-block divergence fails loud instead of matching.
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- if E {{ 1 }} else {{ 2 }}\n+ if E {{ 1 }} else {{ 3 }}\n"
        );
        let src = "fn a(x: bool) {\n    let v = if x { 9 } else { 2 };\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn if_else_arity_mismatch_vetoes() {
        // Missing `else` never matches an `else` pattern.
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- if E {{ 1 }} else {{ 2 }}\n+ if E {{ 1 }} else {{ 3 }}\n"
        );
        let src = "fn a(x: bool) {\n    let v = if x { 1 };\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn else_if_chain_recurses() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n# expr F\n- if E {{ 1 }} else if F {{ 2 }} else {{ 3 }}\n+ if E {{ 1 }} else if F {{ 2 }} else {{ 4 }}\n"
        );
        let src = "fn a(x: bool, y: bool) {\n    let v = if x { 1 } else if y { 2 } else { 3 };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("else { 4 }"), "got: {out}");
    }

    #[test]
    fn block_length_mismatch_vetoes() {
        // Concrete sequences: a 2-stmt block never matches a 1-stmt body.
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- if E {{ 1 }}\n+ if E {{ 2 }}\n"
        );
        let src = "fn a(x: bool) {\n    let v = if x { 1; 2 };\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn block_member_concrete_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- if E {{ 1 }}\n+ if E {{ 2 }}\n"
        );
        let src = "fn a(x: bool) {\n    let v = if x { 9 };\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn let_else_diverge_fires() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- let x = E else {{ return 0 }};\n+ let y = E else {{ return 0 }};\n"
        );
        let src = "fn a(o: Option<i32>) -> i32 {\n    let x = o else { return 0 };\n    x\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let y = o else { return 0 };"), "got: {out}");
    }

    #[test]
    fn let_else_mixed_diverge_vetoes() {
        // Pattern without `else` never matches a `let-else` target.
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n"
        );
        let src = "fn a(o: Option<i32>) -> i32 {\n    let x = o else { return 0 };\n    x\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn at_pattern_concrete_sub_fires() {
        // `P` binds the whole target pattern text, so the plus side
        // reuses `P` alone (repeating the `@ sub` spelling would
        // duplicate it). Fixed-point output proves the match.
        let patch = format!(
            "{HEAD}@@ -1,2 @@ rule=r\n# lang=rust\n# pat P\n# expr E\n- let P @ Some(x) = E;\n+ let P = E;\n"
        );
        let src = "fn a(o: Option<i32>) {\n    let v @ Some(x) = o;\n    let _ = (v, x);\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let v @ Some(x) = o;"), "got: {out}");
    }

    #[test]
    fn at_pattern_placeholder_sub_fires() {
        let patch = format!(
            "{HEAD}@@ -1,2 @@ rule=r\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- let P @ Q = E;\n+ let P = E;\n"
        );
        let src = "fn a(o: Option<i32>) {\n    let v @ Some(x) = o;\n    let _ = (v, x);\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let v @ Some(x) = o;"), "got: {out}");
    }

    #[test]
    fn at_pattern_concrete_head_with_placeholder_sub_stays_c8() {
        // Concrete head + placeholder sub is still an embedded
        // metavariable (C8): fail at compile, not at match.
        let patch = format!(
            "{HEAD}@@ -1,2 @@ rule=r\n# lang=rust\n# pat Q\n# expr E\n- let q @ Q = E;\n+ let q = E;\n"
        );
        let src = "fn a(o: Option<i32>) {\n    let q @ Some(x) = o;\n    let _ = (q, x);\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }
}
