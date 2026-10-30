//! Control-flow and statement arms for the general matcher (WI-5a/5b).
//!
//! Split from `matcher_general` per D5 (the parent file sits at the
//! 2000-line gate). Owns the statement-level structural matching:
//! `match_local` (`let` with or without `let-else`, `P @ sub`),
//! `match_block` (concrete prefix plus an optional trailing `...` tail,
//! WI-5b), `match_stmt`,
//! `match_if`, `match_match` (+ `match_arm`), `match_loop`,
//! `match_while`, `match_for`, `match_closure` (+ param helper),
//! `match_let_expr`, and the `@`-subpattern helper `match_pat`.
//! The parent keeps `match_expr`/`match_type` plus collection,
//! substitution, and application; the two modules call each other
//! (circular within the crate is fine).
//!
//! Everything not listed here (async/try/unsafe blocks, yield, macro
//! types) still rejects in `match_expr` and stays an ordinary hunk.

use syn::spanned::Spanned;

use crate::edits::fragments_equal;

use super::matcher_general::{
    as_placeholder, match_expr, match_return_type, match_type, render, render_eq,
    single_placeholder_type, MatchCx,
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
    // WI-5b: a trailing bare `...` pseudo-statement absorbs any remaining
    // target statements (zero or more). The absorbed tail is sliced in
    // source order for plus-side replay (or dropped when the plus side
    // omits `...`); the concrete prefix still matches pairwise. Any other
    // `...` position fails at compile time (C1), never here.
    let ellipsis = pat.stmts.last().is_some_and(is_block_ellipsis_stmt);
    let prefix = if ellipsis {
        pat.stmts.len() - 1
    } else {
        pat.stmts.len()
    };
    if ellipsis {
        if tgt.stmts.len() < prefix {
            return false;
        }
        if !pat.stmts[..prefix]
            .iter()
            .zip(tgt.stmts.iter())
            .all(|(a, b)| match_stmt(a, b, cx))
        {
            return false;
        }
        return cx.push_block_tail(&tgt.stmts[prefix..]);
    }
    pat.stmts.len() == tgt.stmts.len()
        && pat
            .stmts
            .iter()
            .zip(tgt.stmts.iter())
            .all(|(a, b)| match_stmt(a, b, cx))
}

/// A trailing-block pseudo-statement: a bare `...` expression statement
/// (post-substitution `__RSMPL_ELLIPSIS_N` ident, with or without `;`).
fn is_block_ellipsis_stmt(stmt: &syn::Stmt) -> bool {
    matches!(stmt, syn::Stmt::Expr(syn::Expr::Path(p), _) if p.qself.is_none()
    && p.path.segments.len() == 1
    && p.path.segments[0].arguments.is_none()
    && p.path.segments[0]
        .ident
        .to_string()
        .strip_prefix("__RSMPL_ELLIPSIS")
        .is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }))
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

/// Match a pattern position: a whole-pattern placeholder binds the
/// target pattern text; a placeholder-head `P @ sub` binds `P` to the
/// whole target text and matches `sub` recursively (a target `@`
/// unwraps to its subpattern, otherwise the whole target pattern);
/// anything else must be strictly token-equal (never guessed).
/// Shared by `match_local` bound patterns (WI-5a) and by match arms,
/// for-loop / closure / let-expression positions (WI-5b).
pub(crate) fn match_pat(pat: &syn::Pat, tgt: &syn::Pat, cx: &mut MatchCx) -> bool {
    if let syn::Pat::Ident(pid) = pat {
        if let Some(name) = as_placeholder(&cx.reverse, &pid.ident.to_string()).cloned() {
            if let Some((_, sub)) = &pid.subpat {
                // `P @ sub`: `P` binds the whole target text; `sub`
                // matches the target subpattern (or the whole target
                // pattern when it carries no `@`).
                if !cx.bind_span(&name, tgt.span()) {
                    return false;
                }
                let tsub: &syn::Pat = match tgt {
                    syn::Pat::Ident(tid) => tid.subpat.as_ref().map(|(_, s)| &**s).unwrap_or(tgt),
                    _ => tgt,
                };
                return match_pat(sub, tsub, cx);
            }
            return cx.bind_span(&name, tgt.span());
        }
    }
    render_eq(pat, tgt)
}

/// Strict label equality: a labeled block/loop is a different control
/// construct from an unlabeled one (never guessed).
fn match_label(pat: &Option<syn::Label>, tgt: &Option<syn::Label>, _cx: &mut MatchCx) -> bool {
    match (pat, tgt) {
        (None, None) => true,
        (Some(a), Some(b)) => render_eq(a, b),
        _ => false,
    }
}

/// Match a `match` expression: scrutinee via the general expression
/// matcher, arms pairwise (equal count, source order). Each arm matches
/// its pattern via `match_pat` (placeholder binds, concrete strict),
/// its guard structurally, and its body as an expression.
pub(crate) fn match_match(pat: &syn::ExprMatch, tgt: &syn::ExprMatch, cx: &mut MatchCx) -> bool {
    if !match_expr(&pat.expr, &tgt.expr, cx) {
        return false;
    }
    pat.arms.len() == tgt.arms.len()
        && pat
            .arms
            .iter()
            .zip(tgt.arms.iter())
            .all(|(a, b)| match_arm(a, b, cx))
}

fn match_arm(pat: &syn::Arm, tgt: &syn::Arm, cx: &mut MatchCx) -> bool {
    // Arm attributes compare strictly: an attributed arm is a different
    // item, never guessed. Trailing-comma presence is formatting only.
    if pat.attrs.len() != tgt.attrs.len() {
        return false;
    }
    if !pat
        .attrs
        .iter()
        .zip(tgt.attrs.iter())
        .all(|(a, b)| render_eq(a, b))
    {
        return false;
    }
    if !match_pat(&pat.pat, &tgt.pat, cx) {
        return false;
    }
    match (&pat.guard, &tgt.guard) {
        (None, None) => {}
        (Some((_, a)), Some((_, b))) => {
            if !match_expr(a, b, cx) {
                return false;
            }
        }
        _ => return false,
    }
    match_expr(&pat.body, &tgt.body, cx)
}

/// Match a `loop` expression: label strictly, body structurally.
pub(crate) fn match_loop(pat: &syn::ExprLoop, tgt: &syn::ExprLoop, cx: &mut MatchCx) -> bool {
    match_label(&pat.label, &tgt.label, cx) && match_block(&pat.body, &tgt.body, cx)
}

/// Match a `while` expression: label strictly, condition via the
/// general matcher, body structurally.
pub(crate) fn match_while(pat: &syn::ExprWhile, tgt: &syn::ExprWhile, cx: &mut MatchCx) -> bool {
    match_label(&pat.label, &tgt.label, cx)
        && match_expr(&pat.cond, &tgt.cond, cx)
        && match_block(&pat.body, &tgt.body, cx)
}

/// Match a `for` loop: label strictly, bound pattern via `match_pat`,
/// iterator via the general matcher, body structurally.
pub(crate) fn match_for(pat: &syn::ExprForLoop, tgt: &syn::ExprForLoop, cx: &mut MatchCx) -> bool {
    match_label(&pat.label, &tgt.label, cx)
        && match_pat(&pat.pat, &tgt.pat, cx)
        && match_expr(&pat.expr, &tgt.expr, cx)
        && match_block(&pat.body, &tgt.body, cx)
}

/// Match a closure: async/move/capture qualifiers plus lifetimes and
/// constness compare strictly (a different qualifier is a different
/// closure); params pairwise via the decomposed helper (mirrors the
/// `P: T` rule of `match_local`); return type structurally; body as an
/// expression.
pub(crate) fn match_closure(
    pat: &syn::ExprClosure,
    tgt: &syn::ExprClosure,
    cx: &mut MatchCx,
) -> bool {
    if render(&pat.asyncness) != render(&tgt.asyncness) {
        return false;
    }
    if render(&pat.movability) != render(&tgt.movability) {
        return false;
    }
    if render(&pat.lifetimes) != render(&tgt.lifetimes) {
        return false;
    }
    if render(&pat.constness) != render(&tgt.constness) {
        return false;
    }
    if pat.inputs.len() != tgt.inputs.len() {
        return false;
    }
    if !pat
        .inputs
        .iter()
        .zip(tgt.inputs.iter())
        .all(|(a, b)| match_closure_param(a, b, cx))
    {
        return false;
    }
    match_return_type(&pat.output, &tgt.output, cx) && match_expr(&pat.body, &tgt.body, cx)
}

/// Match one closure parameter: decompose `P: T` like `match_local`
/// (bare `P` never drops an annotation), inner pattern via `match_pat`,
/// ascribed type via placeholder-or-structural agreement.
fn match_closure_param(pat: &syn::Pat, tgt: &syn::Pat, cx: &mut MatchCx) -> bool {
    let (pi, pty) = match pat {
        syn::Pat::Type(pt) => (&*pt.pat, Some(&*pt.ty)),
        _ => (pat, None),
    };
    let (ti, tty) = match tgt {
        syn::Pat::Type(pt) => (&*pt.pat, Some(&*pt.ty)),
        _ => (tgt, None),
    };
    if !match_pat(pi, ti, cx) {
        return false;
    }
    match (pty, tty) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if let Some(name) = single_placeholder_type(a, cx) {
                return cx.bind_span(&name, b.span());
            }
            match_type(a, b, cx)
        }
        _ => false,
    }
}

/// Match a `let` expression (`if let` / `while let` conditions): pattern
/// via `match_pat`, scrutinee via the general matcher.
pub(crate) fn match_let_expr(pat: &syn::ExprLet, tgt: &syn::ExprLet, cx: &mut MatchCx) -> bool {
    match_pat(&pat.pat, &tgt.pat, cx) && match_expr(&pat.expr, &tgt.expr, cx)
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

    #[test]
    fn match_scrutinee_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- match E {{ Some(x) => 1, _ => 2 }}\n+ match E {{ Some(x) => 1, _ => 3 }}\n"
        );
        let src = "fn a(o: Option<i32>) {\n    let v = match o { Some(x) => 1, _ => 2 };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("_ => 3"), "got: {out}");
    }

    #[test]
    fn match_arm_count_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- match E {{ Some(x) => 1 }}\n+ match E {{ Some(x) => 2 }}\n"
        );
        let src = "fn a(o: Option<i32>) {\n    let v = match o { Some(x) => 1, _ => 2 };\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn match_guard_divergence_vetoes() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- match E {{ x if x > 0 => 1, _ => 2 }}\n+ match E {{ x if x > 0 => 1, _ => 3 }}\n"
        );
        let src =
            "fn a(x: i32) {\n    let v = match x { x if x > 1 => 1, _ => 2 };\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn loop_body_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n- loop {{ break; }}\n+ loop {{ continue; }}\n"
        );
        let src = "fn a() {\n    loop { break; }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("continue;"), "got: {out}");
    }

    #[test]
    fn loop_label_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n- loop {{ break; }}\n+ loop {{ continue; }}\n"
        );
        let src = "fn a() {\n    'outer loop { break; }\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn while_cond_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- while E {{ break; }}\n+ while E {{ continue; }}\n"
        );
        let src = "fn a(x: bool) {\n    while x { break; }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("continue;"), "got: {out}");
    }

    #[test]
    fn for_iter_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# pat P\n# expr E\n- for P in E {{ break; }}\n+ for P in E {{ continue; }}\n"
        );
        let src = "fn a() {\n    for x in y { break; }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("for x in y { continue; }"), "got: {out}");
    }

    #[test]
    fn closure_param_and_body_holes_rewrite() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# pat P\n# expr E\n- |P| E\n+ |P| E + 1\n"
        );
        let src = "fn a() {\n    let f = |x| x;\n    let _ = f;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("|x| x + 1"), "got: {out}");
    }

    #[test]
    fn closure_async_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# pat P\n# expr E\n- |P| E\n+ |P| E + 1\n"
        );
        let src = "fn a() {\n    let f = async |x| x;\n    let _ = f;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn let_expr_scrutinee_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- if let Some(x) = E {{ 1 }} else {{ 2 }}\n+ if let Some(x) = E {{ 1 }} else {{ 3 }}\n"
        );
        let src = "fn a(y: Option<i32>) {\n    let v = if let Some(x) = y { 1 } else { 2 };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("else { 3 }"), "got: {out}");
    }

    #[test]
    fn let_expr_pat_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- if let Some(x) = E {{ 1 }} else {{ 2 }}\n+ if let Some(x) = E {{ 1 }} else {{ 3 }}\n"
        );
        let src = "fn a(y: Option<i32>) {\n    let v = if let None = y { 1 } else { 2 };\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn at_pattern_in_match_arm_fires() {
        // Placeholder-head `@` unwraps in arm positions too; the plus
        // side reuses `P` alone (repeating `@` would double-splice,
        // the SPEC R4 plus-side rule).
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- match E {{ P @ Q => 1, _ => 2 }}\n+ match E {{ P => 1, _ => 3 }}\n"
        );
        let src = "fn a(o: Option<i32>) {\n    let v = match o { v @ Some(x) => 1, _ => 2 };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("_ => 3"), "got: {out}");
    }

    // WI-5b: trailing block `...` replays the absorbed tail into the plus
    // `...` slot, preserving the tail verbatim (indent included).
    #[test]
    fn block_trailing_ellipsis_replays_tail() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); ... }}\n+ if E {{ bar(); ... }}\n"
        );
        let src = "fn f(c: bool) {\n    if c {\n        foo();\n        extra1();\n        extra2();\n    }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("bar();"), "got: {out}");
        assert!(out.contains("extra1();"), "got: {out}");
        assert!(out.contains("extra2();"), "got: {out}");
        assert!(!out.contains("foo();"), "got: {out}");
    }

    // WI-5b: omitting the plus `...` deliberately drops the tail.
    #[test]
    fn block_trailing_ellipsis_drop_omits_tail() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); ... }}\n+ if E {{ bar(); }}\n"
        );
        let src = "fn f(c: bool) {\n    if c {\n        foo();\n        extra1();\n    }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("bar();"), "got: {out}");
        assert!(!out.contains("extra1();"), "got: {out}");
        assert!(!out.contains("foo();"), "got: {out}");
    }

    // WI-5b: an exact-length target matches with an empty tail.
    #[test]
    fn block_trailing_ellipsis_zero_tail_matches() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); ... }}\n+ if E {{ bar(); ... }}\n"
        );
        let src = "fn f(c: bool) {\n    if c {\n        foo();\n    }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("bar();"), "got: {out}");
        assert!(!out.contains("foo();"), "got: {out}");
    }

    // WI-5b: a shorter target (fewer than the concrete prefix) vetoes.
    #[test]
    fn block_trailing_ellipsis_short_target_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); extra(); ... }}\n+ if E {{ bar(); ... }}\n"
        );
        let src = "fn f(c: bool) {\n    if c {\n        foo();\n    }\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-5b: a divergent concrete prefix never matches, tail or not.
    #[test]
    fn block_trailing_ellipsis_prefix_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); ... }}\n+ if E {{ bar(); ... }}\n"
        );
        let src = "fn f(c: bool) {\n    if c {\n        other();\n        extra();\n    }\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-5b: two trailing blocks map tails in source order.
    #[test]
    fn block_trailing_ellipsis_two_blocks_replay_in_order() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); ... }} else {{ bar(); ... }}\n+ if E {{ foo2(); ... }} else {{ bar2(); ... }}\n"
        );
        let src = "fn f(c: bool) {\n    if c {\n        foo();\n        a1();\n    } else {\n        bar();\n        b1();\n    }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("foo2();"), "got: {out}");
        assert!(out.contains("bar2();"), "got: {out}");
        assert!(out.contains("a1();"), "got: {out}");
        assert!(out.contains("b1();"), "got: {out}");
        assert!(!out.contains("foo();"), "got: {out}");
    }

    // WI-5b: `...` outside the trailing position fails at compile time.
    #[test]
    fn block_mid_ellipsis_is_compile_conflict() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ ... foo(); }}\n+ if E {{ ... bar(); }}\n"
        );
        let src = "fn f(c: bool) {\n    if c {\n        foo();\n    }\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("trailing block"), "got: {err}");
    }

    // WI-5b: a plus `...` with no minus `...` names the replay fence.
    #[test]
    fn block_plus_ellipsis_without_minus_is_conflict() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); }}\n+ if E {{ bar(); ... }}\n"
        );
        let src = "fn f(c: bool) {\n    if c {\n        foo();\n    }\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("no matching `- ...`"), "got: {err}");
    }

    // WI-5b: replay is all-or-nothing — a partial plus count fails loud
    // instead of misaligning tails.
    #[test]
    fn block_ellipsis_count_mismatch_is_conflict() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ foo(); ... }} else {{ bar(); ... }}\n+ if E {{ foo2(); ... }} else {{ bar2(); }}\n"
        );
        let src =
            "fn f(c: bool) {\n    if c {\n        foo();\n    } else {\n        bar();\n    }\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("count differs"), "got: {err}");
    }

    // WI-5b: `let` initializers ride the same arm (Stmt shape).
    #[test]
    fn block_ellipsis_in_let_init_replays() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- let v = if E {{ foo(); ... }} else {{ 0 }};\n+ let v = if E {{ bar(); ... }} else {{ 0 }};\n"
        );
        let src = "fn f(c: bool) {\n    let v = if c { foo(); extra(); } else { 0 };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("bar();"), "got: {out}");
        assert!(out.contains("extra();"), "got: {out}");
        assert!(!out.contains("foo();"), "got: {out}");
    }
}
