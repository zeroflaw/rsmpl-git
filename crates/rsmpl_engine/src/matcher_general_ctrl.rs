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
//! The parent keeps `match_expr` plus collection,
//! substitution, and application; the three general-matcher modules
//! call each other (circular within the crate is fine).
//!
//! Everything not listed here still rejects in `match_expr` and stays an
//! ordinary hunk (`async`/`try`/`unsafe`/
//! `yield` live in `matcher_general_async`, WI-6 slice 1; StmtMacro
//! whole-shape opaque and Stmt Item fn-only structural live below,
//! WI-6b).

use syn::spanned::Spanned;

use super::matcher_general::{as_placeholder, match_expr, render, render_eq, MatchCx};
use super::matcher_types::{match_return_type, match_type, single_placeholder_type};

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
    } else if !match_pat(pi, ti, cx) {
        // WI-3: composite bound patterns (`Some(P)`, `(P, Q)`, …)
        // match structurally through the shared pattern core.
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
    // WI-5b-follow: digit-suffixed markers only (see rsmpl_pattern::quote_utils).
    && rsmpl_pattern::is_ellipsis_marker_ident(&p.path.segments[0].ident.to_string()))
}

/// Match one block member statement.
fn match_stmt(pat: &syn::Stmt, tgt: &syn::Stmt, cx: &mut MatchCx) -> bool {
    match (pat, tgt) {
        (syn::Stmt::Local(a), syn::Stmt::Local(b)) => match_local(a, b, cx),
        (syn::Stmt::Expr(a, a_semi), syn::Stmt::Expr(b, b_semi)) => {
            a_semi.is_some() == b_semi.is_some() && match_expr(a, b, cx)
        }
        // WI-6b: macro statements match only whole-shape (path,
        // delimiters, and body token-equal - interiors stay opaque and
        // never host holes, SPEC 21 stands); a divergent macro vetoes
        // while an identical one lets the surrounding pattern match.
        (syn::Stmt::Macro(a), syn::Stmt::Macro(b)) => match_macro_stmt(a, b),
        // WI-6b: `fn` items match with attributes/visibility/signature
        // strict (a signature hole never matches - rename the item
        // separately) and the body structural, so holes may live inside
        // the inner function; every other item kind stays strictly
        // token-equal (an item rename inside a block is two changes).
        (syn::Stmt::Item(a), syn::Stmt::Item(b)) => match_item_stmt(a, b, cx),
        _ => false,
    }
}

/// Match a macro-statement block member opaquely: attributes pairwise
/// strict, semicolon presence agreement, and the whole invocation (path,
/// delimiters, body) token-equal. No holes are bound inside.
fn match_macro_stmt(pat: &syn::StmtMacro, tgt: &syn::StmtMacro) -> bool {
    pat.attrs.len() == tgt.attrs.len()
        && pat
            .attrs
            .iter()
            .zip(tgt.attrs.iter())
            .all(|(a, b)| render_eq(a, b))
        && pat.semi_token.is_some() == tgt.semi_token.is_some()
        && render_eq(&pat.mac, &tgt.mac)
}

/// Match an item-statement block member: `fn` vs `fn` compares the
/// header strictly and the body via `match_block` (trailing ... rides
/// along); anything else is token-equal, as before.
fn match_item_stmt(pat: &syn::Item, tgt: &syn::Item, cx: &mut MatchCx) -> bool {
    match (pat, tgt) {
        (syn::Item::Fn(a), syn::Item::Fn(b)) => {
            a.attrs.len() == b.attrs.len()
                && a.attrs
                    .iter()
                    .zip(b.attrs.iter())
                    .all(|(x, y)| render_eq(x, y))
                && render_eq(&a.vis, &b.vis)
                && render_eq(&a.sig, &b.sig)
                && match_block(&a.block, &b.block, cx)
        }
        _ => render_eq(pat, tgt),
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
/// composite patterns (`Some(P)`, `(P, Q)`, `S { f: P }`, `&P`, `[P]`,
/// `(P)`) match structurally with holes bound per position; an `Or`
/// pattern or anything else must be strictly token-equal (never
/// guessed, WI-3). Shared by `match_local` bound patterns (WI-5a) and
/// by match arms, for-loop / closure / let-expression positions
/// (WI-5b).
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
    match_pat_structural(pat, tgt, cx)
}

/// WI-3 structural composite matching: the six bounded arms. A leaf
/// placeholder `Ident` binds the target fragment at that position
/// (repeat occurrences enforce structural equality via `bind`); a
/// concrete leaf compares token-equal. `Or` and every other variant
/// stay token-equal — `Or`-with-hole is rejected at compile time (C8).
fn match_pat_structural(pat: &syn::Pat, tgt: &syn::Pat, cx: &mut MatchCx) -> bool {
    // A placeholder ident in any leaf position binds the whole target
    // fragment (flags such as `mut`/`ref` ride the binding).
    if let syn::Pat::Ident(pid) = pat {
        if let Some(name) = as_placeholder(&cx.reverse, &pid.ident.to_string()).cloned() {
            debug_assert!(pid.subpat.is_none(), "P @ sub handled above");
            return cx.bind_span(&name, tgt.span());
        }
    }
    match (pat, tgt) {
        (syn::Pat::TupleStruct(a), syn::Pat::TupleStruct(b)) => {
            match_pat_path(&a.path, &b.path, cx)
                && a.elems.len() == b.elems.len()
                && a.elems
                    .iter()
                    .zip(b.elems.iter())
                    .all(|(x, y)| match_pat(x, y, cx))
        }
        (syn::Pat::Tuple(a), syn::Pat::Tuple(b)) => {
            a.elems.len() == b.elems.len()
                && a.elems
                    .iter()
                    .zip(b.elems.iter())
                    .all(|(x, y)| match_pat(x, y, cx))
        }
        (syn::Pat::Struct(a), syn::Pat::Struct(b)) => {
            match_pat_path(&a.path, &b.path, cx) && match_pat_struct_fields(a, b, cx)
        }
        (syn::Pat::Paren(a), syn::Pat::Paren(b)) => match_pat(&a.pat, &b.pat, cx),
        (syn::Pat::Reference(a), syn::Pat::Reference(b)) => {
            a.mutability.is_some() == b.mutability.is_some() && match_pat(&a.pat, &b.pat, cx)
        }
        (syn::Pat::Slice(a), syn::Pat::Slice(b)) => {
            a.elems.len() == b.elems.len()
                && a.elems
                    .iter()
                    .zip(b.elems.iter())
                    .all(|(x, y)| match_pat(x, y, cx))
        }
        _ => render_eq(pat, tgt),
    }
}

/// WI-3 pattern-path filter: a path that is a single bare placeholder
/// ident (a declared hole naming the whole path, e.g. `# type S` over
/// `S { f: P }`) binds the target path span; anything else compares
/// token-equal. Multi-segment or generic-arg paths with holes stay
/// concrete (never guessed).
fn match_pat_path(pat: &syn::Path, tgt: &syn::Path, cx: &mut MatchCx) -> bool {
    if pat.segments.len() == 1 {
        let seg = &pat.segments[0];
        if matches!(seg.arguments, syn::PathArguments::None) {
            if let Some(name) = as_placeholder(&cx.reverse, &seg.ident.to_string()).cloned() {
                return cx.bind_span(&name, tgt.span());
            }
        }
    }
    render_eq(pat, tgt)
}

/// WI-3 struct-pattern fields: matched by member name (source order
/// irrelevant), each pair recursing. `..`-rest presence must agree on
/// both sides; with rest on both, the pattern fields must be a subset
/// of the target fields (rest absorbs the surplus); without rest, the
/// member sets must be exactly equal.
fn match_pat_struct_fields(a: &syn::PatStruct, b: &syn::PatStruct, cx: &mut MatchCx) -> bool {
    if a.rest.is_some() != b.rest.is_some() {
        return false;
    }
    if a.rest.is_none() && a.fields.len() != b.fields.len() {
        return false;
    }
    if a.fields.len() > b.fields.len() {
        return false;
    }
    // Roll back partial bindings on failure: a member-name miss after
    // earlier fields bound must not poison the candidate.
    let checkpoint = cx.bindings.clone();
    for f in &a.fields {
        let member = render(&f.member);
        let Some(g) = b.fields.iter().find(|g| render(&g.member) == member) else {
            cx.bindings = checkpoint;
            return false;
        };
        if f.colon_token.is_some() != g.colon_token.is_some() {
            cx.bindings = checkpoint;
            return false;
        }
        if !match_pat(&f.pat, &g.pat, cx) {
            cx.bindings = checkpoint;
            return false;
        }
    }
    true
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

    // WI-6b: an identical macro statement lets the surrounding block
    // pattern match (the macro itself is opaque — no holes inside).
    #[test]
    fn stmt_macro_opaque_allows_surrounding_rewrite() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n# expr F\n- if E {{ log!(\"hit\"); F }} else {{ 0 }}\n+ if E {{ log!(\"hit\"); F + 1 }} else {{ 0 }}\n"
        );
        let src = "fn a(c: bool, v: i32) -> i32 {\n    if c {\n        log!(\"hit\");\n        v\n    } else {\n        0\n    }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("log!(\"hit\");"), "got: {out}");
        assert!(out.contains("v + 1"), "got: {out}");
    }

    // WI-6b: a divergent macro body vetoes (opaque means token-equal).
    #[test]
    fn stmt_macro_body_divergence_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n# expr F\n- if E {{ log!(\"hit\"); F }} else {{ 0 }}\n+ if E {{ log!(\"hit\"); F + 1 }} else {{ 0 }}\n"
        );
        let src = "fn a(c: bool, v: i32) -> i32 {\n    if c {\n        log!(\"miss\");\n        v\n    } else {\n        0\n    }\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-6b: a divergent macro path vetoes.
    #[test]
    fn stmt_macro_path_divergence_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n# expr F\n- if E {{ log!(\"hit\"); F }} else {{ 0 }}\n+ if E {{ log!(\"hit\"); F + 1 }} else {{ 0 }}\n"
        );
        let src = "fn a(c: bool, v: i32) -> i32 {\n    if c {\n        warn!(\"hit\");\n        v\n    } else {\n        0\n    }\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-6b: `;` presence agrees — a semicolon-less macro invocation is
    // an expression statement, never a macro statement.
    #[test]
    fn stmt_macro_semi_divergence_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n- if E {{ log!(\"hit\"); }}\n+ if E {{ log!(\"hit\"); }}\n"
        );
        let src = "fn a(c: bool) {\n    if c {\n        log!(\"hit\")\n    }\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-6b-follow: an explicit `# expr LOG` hole never leaks into an
    // opaque macro statement — the declared ident substitutes to a
    // placeholder the whole-shape `render_eq` never matches, so the
    // hunk vetoes loud against a lowercase target (and never binds).
    #[test]
    fn stmt_macro_declared_hole_never_binds() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n# expr F\n# expr LOG\n- if E {{ LOG!(\"hit\"); F }} else {{ 0 }}\n+ if E {{ LOG!(\"hit\"); F + 1 }} else {{ 0 }}\n"
        );
        let src = "fn a(c: bool, v: i32) -> i32 {\n    if c {\n        log!(\"hit\");\n        v\n    } else {\n        0\n    }\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    // WI-6b-follow: even same-spelling `LOG!(...)` stays unmatchable —
    // the pattern holds `__RSMPL_LOG`, never literal `LOG`, so the
    // opaque comparison vetoes instead of binding the macro path.
    #[test]
    fn stmt_macro_declared_hole_same_spelling_still_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr E\n# expr F\n# expr LOG\n- if E {{ LOG!(\"hit\"); F }} else {{ 0 }}\n+ if E {{ LOG!(\"hit\"); F + 1 }} else {{ 0 }}\n"
        );
        let src = "fn a(c: bool, v: i32) -> i32 {\n    if c {\n        LOG!(\"hit\");\n        v\n    } else {\n        0\n    }\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    // WI-6b: holes may live inside an inner `fn` body (header strict,
    // body structural).
    #[test]
    fn stmt_fn_item_body_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr F\n- {{ fn helper() -> i32 {{ F }} helper() }}\n+ {{ fn helper() -> i32 {{ F + 1 }} helper() }}\n"
        );
        let src = "fn a() -> i32 {\n    let w = {\n        fn helper() -> i32 {\n            1\n        }\n        helper()\n    };\n    w\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("1 + 1"), "got: {out}");
    }

    // WI-6b: an inner-`fn` signature divergence vetoes (rename the item
    // separately — never guessed here).
    #[test]
    fn stmt_fn_item_sig_divergence_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr F\n- {{ fn helper() -> String {{ F }} helper() }}\n+ {{ fn helper() -> String {{ F + 1 }} helper() }}\n"
        );
        let src = "fn a() -> i32 {\n    let w = {\n        fn helper() -> i32 {\n            1\n        }\n        helper()\n    };\n    w\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-6b: non-`fn` items stay strictly token-equal (unchanged).
    #[test]
    fn stmt_non_fn_item_token_equal_fires() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# expr F\n- {{ const C: i32 = 1; F }}\n+ {{ const C: i32 = 1; F + 1 }}\n"
        );
        let src =
            "fn a() -> i32 {\n    let w = {\n        const C: i32 = 1;\n        C\n    };\n    w\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("C + 1"), "got: {out}");
    }

    // WI-3: tuple-struct hole binds the inner fragment; the plus side
    // replays it through the same hole.
    #[test]
    fn tuple_struct_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let Some(P) = E;\n+ let Some(P) = E.unwrap_or(0);\n"
        );
        let src = "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let _ = (v, w);\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let Some(v) = o.unwrap_or(0);"), "got: {out}");
        assert!(out.contains("let Some(w) = p.unwrap_or(0);"), "got: {out}");
    }

    // WI-3: a tuple-struct head mismatch vetoes (never guessed).
    #[test]
    fn tuple_struct_head_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let Some(P) = E;\n+ let Some(P) = E.unwrap_or(0);\n"
        );
        let src = "fn a(o: Option<i32>) {\n    let None = o;\n    let _ = 1;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-3: tuple holes bind per position.
    #[test]
    fn tuple_holes_rewrite() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- let (P, Q) = E;\n+ let (Q, P) = E;\n"
        );
        let src = "fn a(t: (i32, i32)) {\n    let (a, b) = t;\n    let _ = (a, b);\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let (b, a) = t;"), "got: {out}");
    }

    // WI-3: repeated holes enforce structural equality (SPEC 15.7).
    #[test]
    fn repeated_hole_requires_equal_fragments() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let (P, P) = E;\n+ let (P, P) = E.clone();\n"
        );
        let same = "fn a(t: (i32, i32)) {\n    let (a, a) = t;\n    let _ = 1;\n}\n";
        let out = apply(&patch, same).unwrap();
        assert!(out.contains("let (a, a) = t.clone();"), "got: {out}");
        let diff = "fn a(t: (i32, i32)) {\n    let (a, b) = t;\n    let _ = 1;\n}\n";
        assert!(apply(&patch, diff).is_err());
    }

    // WI-3: struct fields match by member name — source order is
    // irrelevant. (The plus side swaps the holes so the rewrite is
    // not an identity, which the sweep no-op gate rejects.)
    #[test]
    fn struct_fields_match_reordered() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- let S {{ f: P, g: Q }} = E;\n+ let S {{ f: Q, g: P }} = E;\n"
        );
        let src = "fn a(s: S) {\n    let S { g: 1, f: v } = s;\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let S { f: 1, g: v } = s;"), "got: {out}");
    }

    // WI-3: `..`-rest presence must agree on both sides.
    #[test]
    fn struct_rest_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let S {{ f: P, .. }} = E;\n+ let S {{ f: P, .. }} = E;\n"
        );
        let src = "fn a(s: S) {\n    let S { f: v } = s;\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-3: an unknown member never matches.
    #[test]
    fn struct_unknown_member_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let S {{ f: P }} = E;\n+ let S {{ f: P }} = E;\n"
        );
        let src = "fn a(s: S) {\n    let S { g: v } = s;\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-3: reference mutability is strict, the inner hole binds.
    // (The plus side drops the `&`, so the rewrite is not an
    // identity, which the sweep no-op gate rejects.)
    #[test]
    fn reference_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let &P = E;\n+ let P = E;\n"
        );
        let src = "fn a(r: &i32) {\n    let &v = r;\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let v = r;"), "got: {out}");
    }

    #[test]
    fn reference_mutability_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let &P = E;\n+ let &P = E;\n"
        );
        let src = "fn a(r: &mut i32) {\n    let &mut v = r;\n    let _ = v;\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-3: slice arity is exact, holes bind per element.
    #[test]
    fn slice_holes_rewrite() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- let [P, Q] = E;\n+ let [Q, P] = E;\n"
        );
        let src = "fn a(a: [i32; 2]) {\n    let [x, y] = a;\n    let _ = (x, y);\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let [y, x] = a;"), "got: {out}");
    }

    #[test]
    fn slice_arity_mismatch_vetoes() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- let [P, Q] = E;\n+ let [Q, P] = E;\n"
        );
        let src = "fn a(a: [i32; 3]) {\n    let [x, y, z] = a;\n    let _ = (x, y, z);\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    // WI-3: paren unwraps one level. (The plus side drops the parens,
    // so the rewrite is not an identity, which the sweep no-op gate
    // rejects.)
    #[test]
    fn paren_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let (P) = E;\n+ let P = E;\n"
        );
        let src = "fn a(x: i32) {\n    let (v) = x;\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let v = x;"), "got: {out}");
    }

    // WI-3: nested composites recurse (tuple inside tuple-struct).
    #[test]
    fn nested_composite_hole_rewrites() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n# lang=rust\n# pat P\n# expr E\n- let Some((P, 1)) = E;\n+ let Some((P, 2)) = E;\n"
        );
        let src = "fn a(o: Option<(i32, i32)>) {\n    let Some((v, 1)) = o;\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let Some((v, 2)) = o;"), "got: {out}");
    }

    // WI-3: composite holes work in match-arm position too.
    #[test]
    fn composite_hole_in_match_arm_fires() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# pat P\n# expr E\n- match E {{ Some(P) => 1, _ => 2 }}\n+ match E {{ Some(P) => 9, _ => 2 }}\n"
        );
        let src = "fn a(o: Option<i32>) {\n    let v = match o { Some(v) => 1, _ => 2 };\n    let _ = v;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("Some(v) => 9"), "got: {out}");
    }

    // WI-3: composite holes work in `for`-loop position.
    #[test]
    fn composite_hole_in_for_loop_fires() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- for (P, Q) in E {{ break; }}\n+ for (P, Q) in E {{ continue; }}\n"
        );
        let src = "fn a() {\n    for (k, v) in m { break; }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("for (k, v) in m { continue; }"), "got: {out}");
    }

    // WI-3: composite holes work in closure-param position.
    #[test]
    fn composite_hole_in_closure_param_fires() {
        let patch = format!(
            "{HEAD}@@ -2,1 @@ rule=r\n# lang=rust\n# pat P\n# pat Q\n# expr E\n- |(P, Q)| E\n+ |(P, Q)| E + 1\n"
        );
        let src = "fn a() {\n    let f = |(x, y)| x;\n    let _ = f;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("|(x, y)| x + 1"), "got: {out}");
    }
}
