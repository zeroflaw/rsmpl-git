//! Frame unwrapping (WI-4, F4; widened WI-3).
//!
//! Whole-line detectors decline any changed line that does not parse
//! as exactly one expression of their shape; a line carrying a `fn`
//! header (`fn f0() { foo(1) }`) parses as neither `Expr` nor a plain
//! `let`, so compact Rust — the default style of test helpers and
//! adapters — stayed ordinary no matter how uniform.
//!
//! `frame_inner` is the unwrap ladder, tried only after whole-line
//! parses fail (callers: `subline::side_expr`,
//! `general_expr::parse_line_expr`): (1) today's single-statement
//! one-line `fn` frame (unchanged winner); (2) a `let` whose init is a
//! closure, `async` block, or bare block holding exactly one statement,
//! contributing that statement's expression (the closure/block frame
//! itself is token-identical minus/plus outside the subterm — proved
//! by the existing `frame_holds` full-line comparison, no new check);
//! (3) a one-line `match` arm body after stripping the unchanged arm
//! head textually at the first `=>`; (4) a one-line `if`/`else` branch
//! body after stripping at the first `{` (the head must name `if` or
//! `else`). Multi-statement bodies, macro bodies, and item
//! declarations return `None` and stay ordinary (named residue).
//! Emitted hunks carry no frame text (D14); ranges cite the full line,
//! as `fn` frames do today.

/// Unwrap a single-statement one-line `fn` frame to its inner
/// expression, or `None` when the line is not such a frame.
pub(crate) fn fn_frame_inner(line: &str) -> Option<syn::Expr> {
    let text = line.trim();
    if text.is_empty() {
        return None;
    }
    let item: syn::ItemFn = syn::parse_str(text).ok()?;
    if item.block.stmts.len() != 1 {
        return None;
    }
    let stmt = item.block.stmts.into_iter().next()?;
    single_stmt_expr(stmt)
}

/// The unwrap ladder (WI-3): `fn` frame first (unchanged winner),
/// then `let`-closure/async/block init, then match-arm tail, then
/// `if`/`else` branch tail. Each rung returns the inner expression the
/// detectors anti-unify; every consumer proves frame equality on the
/// full line text (`subline::frame_holds` or the control driver gate),
/// so a frame delta alongside the inner rewrite vetoes.
pub(crate) fn frame_inner(line: &str) -> Option<syn::Expr> {
    if let Some(expr) = fn_frame_inner(line) {
        return Some(expr);
    }
    if let Some(expr) = let_frame_inner(line) {
        return Some(expr);
    }
    if let Some(expr) = arm_frame_inner(line) {
        return Some(expr);
    }
    branch_frame_inner(line)
}

/// One statement's expression: `Stmt::Expr` with or without `;`, or a
/// `let`-init with `let-else` declined. Declarations and macro bodies
/// never unwrap: the engine cannot bind inside them, so an inner
/// template must not claim them.
fn single_stmt_expr(stmt: syn::Stmt) -> Option<syn::Expr> {
    match stmt {
        syn::Stmt::Expr(expr, _) => Some(expr),
        syn::Stmt::Local(local) => {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            Some(*init.expr)
        }
        // Declarations and macro bodies never unwrap: the engine
        // cannot bind inside them, so an inner template must not
        // claim them.
        _ => None,
    }
}

/// Rung 2: a `let` whose init is a closure, `async` block, or bare
/// block holding exactly one statement. Bare closure bodies (no block)
/// contribute themselves; macro bodies decline.
fn let_frame_inner(line: &str) -> Option<syn::Expr> {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return None;
    };
    let init = local.init?;
    if init.diverge.is_some() {
        return None;
    }
    match *init.expr {
        syn::Expr::Closure(closure) => match *closure.body {
            syn::Expr::Block(block) => {
                if block.block.stmts.len() != 1 {
                    return None;
                }
                let stmt = block.block.stmts.into_iter().next()?;
                single_stmt_expr(stmt)
            }
            syn::Expr::Macro(_) => None,
            body => Some(body),
        },
        syn::Expr::Async(async_block) => {
            if async_block.block.stmts.len() != 1 {
                return None;
            }
            let stmt = async_block.block.stmts.into_iter().next()?;
            single_stmt_expr(stmt)
        }
        syn::Expr::Block(block) => {
            if block.block.stmts.len() != 1 {
                return None;
            }
            let stmt = block.block.stmts.into_iter().next()?;
            single_stmt_expr(stmt)
        }
        _ => None,
    }
}

/// Rung 3: a one-line `match` arm body — the text after the first
/// `=>` (first, so an arm body containing its own `match` still strips
/// at the arm head), with one trailing `,`/`;` tolerated. Block bodies
/// unwrap one single-statement level (`1 => { foo(1) },` contributes
/// `foo(1)`); macro bodies decline. A `=>` inside a string literal
/// never yields an expression, so it declines here.
fn arm_frame_inner(line: &str) -> Option<syn::Expr> {
    let at = line.find("=>")?;
    let (head, tail) = line.split_at(at);
    if head.trim().is_empty() {
        return None;
    }
    let tail = tail[2..].trim();
    if tail.is_empty() {
        return None;
    }
    let tail = super::strip_one_terminator(tail)?;
    let tail = tail.trim();
    if tail.is_empty() {
        return None;
    }
    let expr: syn::Expr = syn::parse_str(tail).ok()?;
    match expr {
        syn::Expr::Block(block) => {
            if block.block.stmts.len() != 1 {
                return None;
            }
            let stmt = block.block.stmts.into_iter().next()?;
            single_stmt_expr(stmt)
        }
        syn::Expr::Macro(_) => None,
        other => Some(other),
    }
}

/// Rung 4: a one-line `if`/`else` branch body — the text after the
/// first `{` when the head names `if` or `else` as a word (so struct
/// literals and closures never strip), minus one trailing `}` plus one
/// terminator. Macro bodies decline.
fn branch_frame_inner(line: &str) -> Option<syn::Expr> {
    let at = line.find('{')?;
    let (head, tail) = line.split_at(at);
    let is_branch = head
        .split(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .any(|w| w == "if" || w == "else");
    if !is_branch {
        return None;
    }
    let mut tail = tail[1..].trim();
    if tail.is_empty() {
        return None;
    }
    tail = super::strip_one_terminator(tail)?.trim();
    let tail = tail.strip_suffix('}').map(str::trim).unwrap_or(tail);
    if tail.is_empty() {
        return None;
    }
    let expr: syn::Expr = syn::parse_str(tail).ok()?;
    match expr {
        syn::Expr::Block(block) => {
            if block.block.stmts.len() != 1 {
                return None;
            }
            let stmt = block.block.stmts.into_iter().next()?;
            single_stmt_expr(stmt)
        }
        syn::Expr::Macro(_) => None,
        other => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        arm_frame_inner, branch_frame_inner, fn_frame_inner, frame_inner, let_frame_inner,
    };
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    #[test]
    fn non_fn_lines_return_none() {
        // A bare call is not an `ItemFn` — must decline so whole-line
        // classes keep their winner.
        assert!(fn_frame_inner("    foo(1);").is_none());
        assert!(fn_frame_inner("fn m() {").is_none());
        assert!(fn_frame_inner("").is_none());
    }

    #[test]
    fn single_call_frame_unwraps() {
        let expr = fn_frame_inner("fn f0() { foo(1) }").expect("must unwrap");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
    }

    #[test]
    fn single_let_frame_unwraps_to_init() {
        let expr = fn_frame_inner("fn f0() { let x = foo(1); }").expect("must unwrap let-init");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
    }

    #[test]
    fn tail_expr_frame_unwraps() {
        let expr = fn_frame_inner("fn id(x: i32) -> i32 { x }").expect("must unwrap tail");
        assert_eq!(quote::ToTokens::to_token_stream(&expr).to_string(), "x");
    }

    #[test]
    fn multi_stmt_frame_vetoes() {
        // Named residue: multi-statement frames stay ordinary.
        assert!(fn_frame_inner("fn f() { let x = 1; foo(x) }").is_none());
    }

    #[test]
    fn f4_call_probe_compresses() {
        // F4(a): three `foo(n)` calls inside one-line fns raise to one
        // bare-call sweep and real-apply byte-identical.
        let pre = "fn f0() { foo(1) }\nfn f1() { foo(2) }\nfn f2() { foo(3) }\n";
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn f0() { foo(1) }\n-fn f1() { foo(2) }\n-fn f2() { foo(3) }\n+fn f0() { bar(1) }\n+fn f1() { bar(2) }\n+fn f2() { bar(3) }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("F4 call probe must compress");
        validated_live(&out, &pre_map);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
    }

    #[test]
    fn f4_operator_probe_compresses() {
        // F4(b): three `v > 0` deltas inside one-line fns raise to one
        // general-expression sweep and real-apply byte-identical.
        let pre =
            "fn f0() -> bool { v0 > 0 }\nfn f1() -> bool { v1 > 0 }\nfn f2() -> bool { v2 > 0 }\n";
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn f0() -> bool { v0 > 0 }\n-fn f1() -> bool { v1 > 0 }\n-fn f2() -> bool { v2 > 0 }\n+fn f0() -> bool { v0 >= 0 }\n+fn f1() -> bool { v1 >= 0 }\n+fn f2() -> bool { v2 >= 0 }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("F4 operator probe must compress");
        validated_live(&out, &pre_map);
        assert!(out.contains(">= 0"), "got: {out}");
    }

    #[test]
    fn f4_method_probe_compresses() {
        // Method twin of the call probe.
        let pre = "fn f0() { a.old(1) }\nfn f1() { b.old(2) }\nfn f2() { c.old(3) }\n";
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn f0() { a.old(1) }\n-fn f1() { b.old(2) }\n-fn f2() { c.old(3) }\n+fn f0() { a.new(1) }\n+fn f1() { b.new(2) }\n+fn f2() { c.new(3) }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("F4 method probe must compress");
        validated_live(&out, &pre_map);
        assert!(out.contains(".old("), "got: {out}");
        assert!(out.contains(".new("), "got: {out}");
    }

    #[test]
    fn f4_field_probe_compresses() {
        // Field twin of the call probe (general-expr field family).
        let pre = "fn f0() { foo.timeout }\nfn f1() { bar.timeout }\nfn f2() { baz.timeout }\n";
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn f0() { foo.timeout }\n-fn f1() { bar.timeout }\n-fn f2() { baz.timeout }\n+fn f0() { foo.timeout2 }\n+fn f1() { bar.timeout2 }\n+fn f2() { baz.timeout2 }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("F4 field probe must compress");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn frame_header_delta_vetoes() {
        // A `fn` name delta alongside the inner rewrite is a frame
        // delta — stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn f0() { foo(1) }\n-fn f1() { foo(2) }\n-fn f2() { foo(3) }\n+fn g0() { bar(1) }\n+fn g1() { bar(2) }\n+fn g2() { bar(3) }\n",
        );
        assert!(
            super::super::infer_semantic(std::slice::from_ref(&raw)).is_none(),
            "frame delta must veto"
        );
    }

    #[test]
    fn frame_distractor_vetoes() {
        // Same-line distractor: one call changes, its twin does not —
        // stays ordinary.
        let pre = "fn f0() { foo(1) + foo(2) }\nfn f1() { foo(3) + foo(4) }\n";
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-fn f0() { foo(1) + foo(2) }\n-fn f1() { foo(3) + foo(4) }\n+fn f0() { bar(1) + foo(2) }\n+fn f1() { bar(3) + foo(4) }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "distractor must veto"
        );
    }

    // WI-3 ladder order: the `fn` rung wins first; lines no earlier
    // rung claims fall through rung by rung.
    #[test]
    fn ladder_fn_rung_wins_first() {
        let expr = frame_inner("fn f0() { foo(1) }").expect("fn rung must unwrap");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // A `let` rung shape that is also a valid `fn` body stays on
        // the `fn` rung (same inner expression either way).
        let expr = frame_inner("fn f0() { let x = foo(1); }").expect("must unwrap");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
    }

    #[test]
    fn ladder_let_closure_rungs_unwrap() {
        // Bare closure body.
        let expr = let_frame_inner("let f = || foo(1);").expect("closure rung");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // Block-bodied closure, single statement.
        let expr = let_frame_inner("let f = || { foo(1) };").expect("block closure rung");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // `async` block init.
        let expr = let_frame_inner("let a = async { foo(1) };").expect("async rung");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // Bare block init.
        let expr = let_frame_inner("let a = { foo(1) };").expect("block rung");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // Non-closure init never reaches this rung.
        assert!(let_frame_inner("let a = foo(1);").is_none());
        assert!(frame_inner("let a = foo(1);").is_none());
    }

    #[test]
    fn ladder_let_rung_vetoes() {
        // Multi-statement bodies stay ordinary (named residue).
        assert!(let_frame_inner("let f = || { setup(); foo(1) };").is_none());
        assert!(let_frame_inner("let a = async { setup(); foo(1) };").is_none());
        assert!(frame_inner("let f = || { setup(); foo(1) };").is_none());
        // Macro bodies never unwrap.
        assert!(let_frame_inner("let f = || foo!(1);").is_none());
        // `let-else` never qualifies.
        assert!(let_frame_inner("let Some(x) = opt else { return; };").is_none());
    }

    #[test]
    fn ladder_arm_rung_unwraps() {
        let expr = arm_frame_inner("        1 => foo(1),").expect("arm rung");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // Guarded arms strip at the same first `=>`.
        let expr = arm_frame_inner("        _ if c => foo(1),").expect("guarded arm");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // Block arm bodies unwrap one single-statement level.
        let expr = arm_frame_inner("        1 => { foo(1) },").expect("block arm");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // An arm body containing its own `match` still strips at the
        // arm head (first `=>`).
        let expr =
            arm_frame_inner("        1 => match y { _ => foo(1) },").expect("nested match arm");
        assert!(quote::ToTokens::to_token_stream(&expr)
            .to_string()
            .contains("match"));
        // No pattern head, or nothing after the arrow: decline.
        assert!(arm_frame_inner("=> foo(1),").is_none());
        assert!(arm_frame_inner("        1 => ,").is_none());
        assert!(arm_frame_inner("        1 => ").is_none());
        // Macro arm bodies never unwrap.
        assert!(arm_frame_inner("        1 => foo!(1),").is_none());
        // Multi-statement block arms stay ordinary.
        assert!(arm_frame_inner("        1 => { setup(); foo(1) },").is_none());
    }

    #[test]
    fn ladder_branch_rung_unwraps() {
        let expr = branch_frame_inner("    } else { foo(1) }").expect("else branch rung");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // Struct literals and closures never strip (no `if`/`else`
        // word in the head).
        assert!(branch_frame_inner("    Foo { x: foo(1) }").is_none());
        assert!(branch_frame_inner("    foo({ bar })").is_none());
        // A whole-line `if` unwraps identically here, but callers only
        // reach this rung after their whole-line parse fails (ladder
        // order is the caller's, not the rung's).
        let expr = branch_frame_inner("    if c { foo(1) }").expect("if unwraps too");
        assert_eq!(
            quote::ToTokens::to_token_stream(&expr).to_string(),
            "foo (1)"
        );
        // Macro branch bodies never unwrap.
        assert!(branch_frame_inner("    } else { foo!(1) }").is_none());
    }

    #[test]
    fn ladder_string_arrow_is_safe() {
        // A `=>` inside a string literal never yields an expression.
        assert!(arm_frame_inner("    let s = \"a => b\";").is_none());
        assert!(frame_inner("    let s = \"a => b\";").is_none());
    }

    // WI-3 A1: match-arm call twins raise to the same sweep as their
    // own-line twins, real-apply byte-identical.
    #[test]
    fn wi3_arm_call_probe_compresses() {
        let pre = "fn m(x: i32) {\n    match x {\n        1 => foo(1),\n        2 => foo(2),\n        _ => foo(3),\n    }\n}\n";
        let raw = section(
            "f.rs",
            "@@ -3,3 +3,3 @@\n         match x {\n-        1 => foo(1),\n-        2 => foo(2),\n-        _ => foo(3),\n+        1 => bar(1),\n+        2 => bar(2),\n+        _ => bar(3),\n         }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("WI-3 arm call probe must compress");
        validated_live(&out, &pre_map);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi3_arm_method_probe_compresses() {
        let pre = "fn m(x: i32) {\n    match x {\n        1 => a.old(1),\n        2 => b.old(2),\n        _ => c.old(3),\n    }\n}\n";
        let raw = section(
            "f.rs",
            "@@ -3,3 +3,3 @@\n         match x {\n-        1 => a.old(1),\n-        2 => b.old(2),\n-        _ => c.old(3),\n+        1 => a.new(1),\n+        2 => b.new(2),\n+        _ => c.new(3),\n         }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("WI-3 arm method probe must compress");
        validated_live(&out, &pre_map);
        assert!(out.contains("Receiver.old(E0)"), "got: {out}");
    }

    #[test]
    fn wi3_branch_call_probe_compresses() {
        // K&R `} else {` lines carry the call delta (three singleton
        // hunks compress via the per-hunk fallback and merge).
        let pre = "fn m(c: bool, d: bool, e: bool) {\n    if c {\n        setup()\n    } else { foo(1) }\n    if d {\n        setup()\n    } else { foo(2) }\n    if e {\n        setup()\n    } else { foo(3) }\n}\n";
        let raw = section(
            "f.rs",
            "@@ -4,1 +4,1 @@\n         setup()\n-    } else { foo(1) }\n+    } else { bar(1) }\n     }\n@@ -7,1 +7,1 @@\n         setup()\n-    } else { foo(2) }\n+    } else { bar(2) }\n     }\n@@ -10,1 +10,1 @@\n         setup()\n-    } else { foo(3) }\n+    } else { bar(3) }\n     }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("WI-3 branch call probe must compress");
        validated_live(&out, &pre_map);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
    }

    // WI-3 A2: frame deltas alongside the inner rewrite veto — the
    // template rewrites only the subterm, so a frame delta must never
    // be silently dropped.
    #[test]
    fn wi3_closure_binder_delta_vetoes_lossy_control() {
        // Closure param renamed alongside an operator delta: the
        // body-only template would leave the binder stale — stays
        // ordinary.
        let pre = "fn m() {\n    let f0 = |x| x > 0;\n    let f1 = |x| x > 1;\n    let f2 = |x| x > 2;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,3 +2,3 @@\n fn m() {\n-    let f0 = |x| x > 0;\n-    let f1 = |x| x > 1;\n-    let f2 = |x| x > 2;\n+    let f0 = |y| y > 0;\n+    let f1 = |y| y > 1;\n+    let f2 = |y| y > 2;\n }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "binder delta must veto"
        );
    }

    #[test]
    fn wi3_fn_header_delta_vetoes_operator() {
        // A `fn` name delta alongside an operator delta: the general
        // path has the same frame rule as the sub-line path now.
        let pre =
            "fn f0() -> bool { v0 > 0 }\nfn f1() -> bool { v1 > 0 }\nfn f2() -> bool { v2 > 0 }\n";
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn f0() -> bool { v0 > 0 }\n-fn f1() -> bool { v1 > 0 }\n-fn f2() -> bool { v2 > 0 }\n+fn g0() -> bool { v0 >= 0 }\n+fn g1() -> bool { v1 >= 0 }\n+fn g2() -> bool { v2 >= 0 }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "fn header delta must veto the operator sweep"
        );
    }

    #[test]
    fn wi3_let_wrapper_delta_vetoes() {
        // Renamed bindings alongside the init delta: the bare init
        // template would leave the wrappers stale — stays ordinary
        // (joints are WI-4's domain).
        let pre = "fn m() {\n    let a = v0 > 0;\n    let b = v1 > 0;\n    let c = v2 > 0;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,3 +2,3 @@\n fn m() {\n-    let a = v0 > 0;\n-    let b = v1 > 0;\n-    let c = v2 > 0;\n+    let d = v0 >= 0;\n+    let e = v1 >= 0;\n+    let f = v2 >= 0;\n }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "wrapper delta must veto"
        );
    }

    #[test]
    fn wi3_arm_pattern_delta_vetoes() {
        // Arm patterns renamed alongside the body delta — stays ordinary.
        let pre = "fn m(x: i32) {\n    match x {\n        1 => v0 > 0,\n        2 => v1 > 0,\n        _ => v2 > 0,\n    }\n}\n";
        let raw = section(
            "f.rs",
            "@@ -3,3 +3,3 @@\n         match x {\n-        1 => v0 > 0,\n-        2 => v1 > 0,\n-        _ => v2 > 0,\n+        9 => v0 >= 0,\n+        8 => v1 >= 0,\n+        _ => v2 >= 0,\n         }\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "arm pattern delta must veto"
        );
    }

    #[test]
    fn wi3_if_while_keyword_delta_vetoes() {
        // `if` → `while` alongside the condition delta — stays ordinary.
        let pre =
            "fn m() {\n    if v0 > 0 {\n    }\n    if v1 > 0 {\n    }\n    if v2 > 0 {\n    }\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,5 +2,5 @@\n fn m() {\n-    if v0 > 0 {\n-    }\n-    if v1 > 0 {\n-    }\n-    if v2 > 0 {\n-    }\n+    while v0 >= 0 {\n+    }\n+    while v1 >= 0 {\n+    }\n+    while v2 >= 0 {\n+    }\n }\n",
        );
        // Two hunks (condition lines + closing braces pair separately);
        // the multi-line shape is outside the single-line control
        // position — stays ordinary.
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .is_none(),
            "keyword delta must veto"
        );
    }
}
