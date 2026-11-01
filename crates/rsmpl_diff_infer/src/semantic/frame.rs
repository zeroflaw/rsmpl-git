//! Fn-frame unwrapping (WI-4, F4).
//!
//! Whole-line detectors decline any changed line that does not parse
//! as exactly one expression of their shape; a line carrying a `fn`
//! header (`fn f0() { foo(1) }`) parses as neither `Expr` nor a plain
//! `let`, so compact Rust — the default style of test helpers and
//! adapters — stayed ordinary no matter how uniform the rewrite.
//!
//! `fn_frame_inner` parses one such line as a `syn::ItemFn` whose body
//! holds exactly one statement and returns that statement's expression
//! (`Stmt::Expr` with or without `;`, or a `let`-init with `let-else`
//! declined). Callers (`subline::side_expr`,
//! `general_expr::parse_line_expr`) fall back to it only after their
//! whole-line parses fail, so no own-line class changes winner.
//! Frame equality needs no new check: the unchanged `frame_holds`
//! comparison runs on the full line text, so a header delta (`fn f0`
//! `fn g0`) alongside the inner rewrite vetoes, exactly as a frame
//! delta around sub-line sites does today. Multi-statement bodies
//! return `None` and stay ordinary (named residue).

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

#[cfg(test)]
mod tests {
    use super::fn_frame_inner;
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
}
