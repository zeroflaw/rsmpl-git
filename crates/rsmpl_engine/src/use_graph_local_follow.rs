//! WI-2b follow completion: block-scoped `let`-binder use rewrites.
//!
//! Split from [`crate::use_graph_local`] (D5 hygiene: that file grows toward
//! the 2000-line gate; behaviour-preserving move, no semantic change).
//! [`crate::use_graph_local`] keeps dispatching (`general_orphans`,
//! binder detectors) and re-exports [`let_binder_follow_edits`] so
//! existing paths keep working.

use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// WI-2b: block-scoped follow edits for a plain binder rename.
///
/// Returns one `follow`-labelled ident replacement per unshadowed value
/// use of `old` inside the smallest `Block` containing the matched `let`.
/// Shadowing (`let old` / closure-param `old`), nested-`fn` bodies, and
/// macro interiors are never rewritten (they stay orphans via
/// [`general_orphans`]). Any other same-name binder after the outer one
/// (`for old`, match arms, fn params, … — WI-2c territory) aborts the
/// whole completion (empty) so an unmodelled shadow never mis-resolves.
pub fn let_binder_follow_edits(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    binder_start: usize,
    binder_end: usize,
    old: &str,
    new: &str,
) -> Vec<TextEdit> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    struct Scopes<'a> {
        original: &'a str,
        index: &'a LineIndex,
        old: &'a str,
        blocks: Vec<(usize, usize)>,
        item_fns: Vec<(usize, usize)>,
        macros: Vec<(usize, usize)>,
        closures: Vec<(usize, usize)>,
        let_defs: Vec<usize>,
        closure_params: Vec<usize>,
        all_pats: Vec<usize>,
    }

    impl<'ast, 'a> Visit<'ast> for Scopes<'a> {
        fn visit_block(&mut self, node: &'ast syn::Block) {
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                // `syn::Block` span covers `{ … }`; fall back to brace
                // token when the joined span mis-maps (never guess).
                let _ = (a, b);
                self.blocks.push((a, b));
            }
            syn::visit::visit_block(self, node);
        }
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                self.item_fns.push((a, b));
            }
            syn::visit::visit_item_fn(self, node);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                self.macros.push((a, b));
            }
        }
        fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                self.closures.push((a, b));
            }
            for inp in &node.inputs {
                for off in crate::use_graph_local::pat_old_offsets(
                    inp,
                    self.old,
                    self.original,
                    self.index,
                ) {
                    self.closure_params.push(off);
                }
            }
            syn::visit::visit_expr_closure(self, node);
        }
        fn visit_local(&mut self, node: &'ast syn::Local) {
            for off in crate::use_graph_local::pat_old_offsets(
                &node.pat,
                self.old,
                self.original,
                self.index,
            ) {
                self.let_defs.push(off);
            }
            syn::visit::visit_local(self, node);
        }
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            if node.ident == self.old {
                let sp = node.ident.span();
                if let (Some(a), Some(b)) = (
                    self.index.offset(sp.start().line as u32, sp.start().column),
                    self.index.offset(sp.end().line as u32, sp.end().column),
                ) {
                    if self.original.get(a..b) == Some(self.old) {
                        self.all_pats.push(a);
                    }
                }
            }
            syn::visit::visit_pat_ident(self, node);
        }
    }

    let mut scopes = Scopes {
        original,
        index,
        old,
        blocks: Vec::new(),
        item_fns: Vec::new(),
        macros: Vec::new(),
        closures: Vec::new(),
        let_defs: Vec::new(),
        closure_params: Vec::new(),
        all_pats: Vec::new(),
    };
    scopes.visit_file(parsed);

    let within = |r: (usize, usize), off: usize| r.0 <= off && off < r.1;
    // Smallest block containing the matched `let` is the scope.
    let mut enclosing: Option<(usize, usize)> = None;
    for b in &scopes.blocks {
        if b.0 <= binder_start && binder_end <= b.1 {
            let narrower = match enclosing {
                None => true,
                Some(e) => (b.1 - b.0) < (e.1 - e.0),
            };
            if narrower {
                enclosing = Some(*b);
            }
        }
    }
    let Some(enc) = enclosing else {
        return Vec::new();
    };
    // Nested fns (strictly inside the block) are a different scope; the
    // outer function owning the block is not excluded.
    let nested_fns: Vec<(usize, usize)> = scopes
        .item_fns
        .iter()
        .copied()
        .filter(|r| enc.0 <= r.0 && r.1 <= enc.1)
        .collect();
    let in_nested_fn = |off: usize| nested_fns.iter().any(|r| r.0 <= off && off < r.1);
    let in_excluded =
        |off: usize| in_nested_fn(off) || scopes.macros.iter().any(|r| r.0 <= off && off < r.1);
    // Deduplicate offset lists (local + pat-ident double visit).
    scopes.let_defs.sort_unstable();
    scopes.let_defs.dedup();
    scopes.closure_params.sort_unstable();
    scopes.closure_params.dedup();
    scopes.all_pats.sort_unstable();
    scopes.all_pats.dedup();

    // Outer binder itself lives inside [binder_start, binder_end).
    let is_outer = |off: usize| binder_start <= off && off < binder_end;
    let inner_lets: Vec<usize> = scopes
        .let_defs
        .iter()
        .copied()
        .filter(|o| *o > binder_end && within(enc, *o) && !in_excluded(*o) && !is_outer(*o))
        .collect();
    let inner_closure_params: Vec<usize> = scopes
        .closure_params
        .iter()
        .copied()
        .filter(|o| *o > binder_end && within(enc, *o) && !in_excluded(*o) && !is_outer(*o))
        .collect();

    // WI-13: modelled shadows for same-name binders outside the
    // `let` / closure-param family (`for` / `if let` / `while let` /
    // match-arm / fn-param defs with their construct scope ends — the
    // same ranges the orphan scan labels `shadowed`). Uses inside these
    // ranges name the inner binder and stay orphans; uses outside them
    // name the outer binder and rewrite below. Filters mirror the
    // orphan side exactly (post-binder, in-block, outside nested-fn /
    // macro interiors, never the outer def itself).
    let extra: Vec<(usize, usize)> =
        crate::use_graph_local_orphan::extra_binder_shadow_ranges(parsed, original, index, old)
            .into_iter()
            .filter(|(d, _)| {
                *d > binder_end && within(enc, *d) && !in_excluded(*d) && !is_outer(*d)
            })
            .collect();
    let extra_defs: Vec<usize> = extra.iter().map(|(d, _)| *d).collect();

    // Residual fail-safe: a post-binder same-name binder covered by no
    // modelled range (unmodellable syntax the shadow walks cannot see)
    // still vetoes completion — safe downgrade instead of a
    // mis-resolved rewrite. All six binder kinds are modelled, so this
    // is unreachable via current syntax; it stays as insurance.
    for o in &scopes.all_pats {
        if *o <= binder_end || !within(enc, *o) || is_outer(*o) || in_excluded(*o) {
            continue;
        }
        if inner_lets.contains(o) || inner_closure_params.contains(o) || extra_defs.contains(o) {
            continue;
        }
        return Vec::new();
    }

    // Shadow scopes: inner `let` governs to its innermost block end,
    // closure params govern to their closure end.
    let mut shadows: Vec<(usize, usize)> = Vec::new();
    for d in &inner_lets {
        let mut best: Option<(usize, usize)> = None;
        for b in &scopes.blocks {
            if b.0 <= *d && *d < b.1 && within(enc, b.0) {
                let narrower = match best {
                    None => true,
                    Some(e) => (b.1 - b.0) < (e.1 - e.0),
                };
                if narrower {
                    best = Some(*b);
                }
            }
        }
        if let Some(b) = best {
            shadows.push((*d, b.1));
        } else {
            shadows.push((*d, enc.1));
        }
    }
    for d in &inner_closure_params {
        let mut best: Option<(usize, usize)> = None;
        for c in &scopes.closures {
            if c.0 <= *d && *d < c.1 {
                let narrower = match best {
                    None => true,
                    Some(e) => (c.1 - c.0) < (e.1 - e.0),
                };
                if narrower {
                    best = Some(*c);
                }
            }
        }
        if let Some(c) = best {
            shadows.push((*d, c.1));
        } else {
            shadows.push((*d, enc.1));
        }
    }
    // WI-13 modelled shadows: unmodelled-binder defs already carry
    // their construct scope end (never widened — uses past the
    // construct resume the outer binder and must rewrite).
    for (d, e) in &extra {
        if *e > *d {
            shadows.push((*d, *e));
        }
    }

    struct Uses<'a, 'b> {
        original: &'a str,
        index: &'a LineIndex,
        old: &'a str,
        new: &'a str,
        enc: (usize, usize),
        binder_end: usize,
        item_fns: &'b [(usize, usize)],
        macros: &'b [(usize, usize)],
        shadows: &'b [(usize, usize)],
        out: Vec<(usize, usize)>,
        /// Shorthand struct-literal sites (`S { old }`) already claimed
        /// as `{old}: {new}` expansions. Their value `ExprPath(old)`
        /// must not also emit a plain `new` edit.
        shorthand_skip: Vec<(usize, usize)>,
        shorthand_edits: Vec<(usize, usize, String)>,
    }
    impl<'ast> Visit<'ast> for Uses<'_, '_> {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            use syn::spanned::Spanned;
            // Only nested fns (strictly inside the enclosing block) are a
            // different scope. The outer function owning the block must
            // still be descended into — otherwise no use is ever seen.
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                if self.enc.0 <= a && b <= self.enc.1 {
                    return;
                }
            }
            syn::visit::visit_item_fn(self, node);
        }
        fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
            if node.qself.is_none()
                && node.path.leading_colon.is_none()
                && node.path.segments.len() == 1
                && node.path.segments[0].ident == self.old
                && node.path.segments[0].arguments.is_none()
            {
                let sp = node.path.segments[0].ident.span();
                if let (Some(a), Some(b)) = (
                    self.index.offset(sp.start().line as u32, sp.start().column),
                    self.index.offset(sp.end().line as u32, sp.end().column),
                ) {
                    // Shorthand struct values (`S { old }`) are claimed by
                    // `visit_expr_struct` as `{old}: {new}` expansions —
                    // never a plain `new` (that would rename the field).
                    let is_shorthand = self.shorthand_skip.iter().any(|(s, e)| a < *e && *s < b);
                    if !is_shorthand
                        && self.original.get(a..b) == Some(self.old)
                        && a >= self.binder_end
                        && self.enc.0 <= a
                        && b <= self.enc.1
                        && !self.item_fns.iter().any(|r| r.0 <= a && a < r.1)
                        && !self.macros.iter().any(|r| r.0 <= a && a < r.1)
                        && !self.shadows.iter().any(|(d, e)| *d < a && a < *e)
                    {
                        self.out.push((a, b));
                    }
                }
            }
            syn::visit::visit_expr_path(self, node);
        }
        /// WI-2b shorthand: `S { old }` means field `old` + value variable
        /// `old`. A binder rename keeps the field and updates the value,
        /// so the single token expands to `S { old: new }` (member-shorthand
        /// discipline, opposite direction from field renames which emit
        /// `S { new: old }`). Explicit `S { old: old }` needs no handling:
        /// the field label is a `Member` (never claimed) and the value
        /// path rewrites plainly to `S { old: new }`.
        fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
            for field in &node.fields {
                if field.colon_token.is_some() {
                    continue;
                }
                let syn::Member::Named(member) = &field.member else {
                    continue;
                };
                if member != self.old {
                    continue;
                }
                let syn::Expr::Path(ep) = &field.expr else {
                    continue;
                };
                if ep.qself.is_some()
                    || ep.path.leading_colon.is_some()
                    || ep.path.segments.len() != 1
                    || ep.path.segments[0].ident != self.old
                    || !ep.path.segments[0].arguments.is_none()
                {
                    continue;
                }
                let Some((a, b)) =
                    crate::matcher_item::span_bytes(self.original, self.index, member.span())
                else {
                    continue;
                };
                if self.original.get(a..b) != Some(self.old)
                    || a < self.binder_end
                    || a < self.enc.0
                    || b > self.enc.1
                    || self.item_fns.iter().any(|r| r.0 <= a && a < r.1)
                    || self.macros.iter().any(|r| r.0 <= a && a < r.1)
                    || self.shadows.iter().any(|(d, e)| *d < a && a < *e)
                {
                    continue;
                }
                self.shorthand_skip.push((a, b));
                self.shorthand_edits
                    .push((a, b, format!("{}: {}", self.old, self.new)));
            }
            syn::visit::visit_expr_struct(self, node);
        }
        // Macro interiors stay evidence (opaque, never rewritten).
        fn visit_macro(&mut self, _node: &'ast syn::Macro) {}
    }

    let mut uses = Uses {
        original,
        index,
        old,
        new,
        enc,
        binder_end,
        item_fns: &nested_fns,
        macros: &scopes.macros,
        shadows: &shadows,
        out: Vec::new(),
        shorthand_skip: Vec::new(),
        shorthand_edits: Vec::new(),
    };
    uses.visit_file(parsed);
    uses.out.sort_unstable();
    uses.out.dedup();
    uses.shorthand_edits.sort();
    uses.shorthand_edits.dedup();
    let mut edits: Vec<TextEdit> = uses
        .out
        .into_iter()
        .map(|(a, b)| TextEdit::new(a, b, new.to_string()).with_label("follow"))
        .collect();
    edits.extend(
        uses.shorthand_edits
            .into_iter()
            .map(|(a, b, r)| TextEdit::new(a, b, r).with_label("follow")),
    );
    edits.sort_by_key(|e| (e.start, e.end));
    edits.dedup_by(|a, b| a.start == b.start && a.end == b.end);
    edits
}

#[cfg(test)]
mod tests {
    use super::let_binder_follow_edits;
    fn follow_edits_for(src: &str) -> Vec<crate::edits::TextEdit> {
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        let stmt_start = src.find("let total").unwrap();
        // Statement bytes: `let total = 1;` through `;`.
        let stmt_end = src[stmt_start..].find(';').unwrap() + stmt_start + 1;
        let_binder_follow_edits(&parsed, src, &index, stmt_start, stmt_end, "total", "sum")
    }

    #[test]
    fn follow_rewrites_plain_uses() {
        let src = "fn main() {\n    let total = 1;\n    let x = total + total;\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 2);
        for e in &edits {
            assert_eq!(e.replacement, "sum");
            assert_eq!(e.label, Some("follow"));
            assert_eq!(&src[e.start..e.end], "total");
        }
    }

    #[test]
    fn follow_empty_when_no_uses() {
        let src = "fn main() {\n    let total = 1;\n}\n";
        assert!(follow_edits_for(src).is_empty());
    }

    #[test]
    fn follow_ignores_use_before_binder() {
        // `total` on line 2 refers to an outer scope; only the use after
        // the binder rewrites.
        let src = "fn main() {\n    let y = total;\n    let total = 1;\n    let z = total;\n}\n";
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        let stmt_start = src.find("let total").unwrap();
        let stmt_end = src[stmt_start..].find(';').unwrap() + stmt_start + 1;
        let edits =
            let_binder_follow_edits(&parsed, src, &index, stmt_start, stmt_end, "total", "sum");
        assert_eq!(edits.len(), 1);
        assert!(src[edits[0].start..edits[0].end] == *"total");
        assert!(edits[0].start > stmt_end);
    }

    #[test]
    fn follow_inner_let_shadows_nested_uses() {
        let src = "fn main() {\n    let total = 1;\n    let a = total;\n    {\n        let total = 2;\n        let b = total;\n    }\n    let c = total;\n}\n";
        let edits = follow_edits_for(src);
        // Outer use before the shadow + outer use after the inner block;
        // the use inside the shadowing block stays for the inner binder.
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn follow_closure_param_shadows_body() {
        let src = "fn main() {\n    let total = 1;\n    let f = |total: u32| total + 1;\n    let g = total;\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn follow_skips_nested_fn() {
        let src = "fn main() {\n    let total = 1;\n    fn inner() -> u32 { total }\n    let g = total;\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn follow_skips_macro_interiors() {
        let src = "fn main() {\n    let total = 1;\n    println!(\"{}\", total);\n    let g = total;\n}\n";
        let edits = follow_edits_for(src);
        // Only the plain `let g = total` use rewrites; the macro arg
        // stays evidence (`macro-hidden` via `general_orphans`).
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn follow_for_shadow_with_no_outer_use_stays_empty() {
        // Every post-binder use sits inside the shadowing loop body, so
        // no follow edit fires (the use stays a `shadowed` orphan).
        let src = "fn main() {\n    let total = 1;\n    for total in 0..3 {\n        let x = total;\n    }\n}\n";
        assert!(follow_edits_for(src).is_empty());
    }

    #[test]
    fn follow_for_shadow_rewrites_outer_use_after_loop() {
        // WI-13: the shadowing `for total` governs only the loop body —
        // the outer use after the loop still names the outer binder and
        // rewrites; the in-body use stays for the inner binder.
        let src = "fn main() {\n    let total = 1;\n    for total in 0..3 {\n        let x = total;\n    }\n    let y = total;\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 1, "got: {edits:?}");
        assert_eq!(&src[edits[0].start..edits[0].end], "total");
        assert!(src[..edits[0].start].contains("for total"));
    }

    #[test]
    fn follow_match_arm_shadow_rewrites_outer_use_after_match() {
        // WI-13: the shadowing arm binder governs only its arm guard +
        // body — the outer use after the `match` rewrites.
        let src = "fn main() {\n    let total = 1;\n    let v = match 1 { total => total + 1, _ => 0 };\n    let y = total;\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 1, "got: {edits:?}");
        assert_eq!(&src[edits[0].start..edits[0].end], "total");
        assert!(src[..edits[0].start].contains("match 1"));
    }

    #[test]
    fn follow_if_let_shadow_rewrites_outer_use_after_if() {
        // WI-13: the shadowing `if let total` governs only the
        // then-branch — the outer use after the whole `if` rewrites.
        let src = "fn main() {\n    let total = 1;\n    if let total = Some(1) { let x = total; } else { let y = 0; }\n    let z = total;\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 1, "got: {edits:?}");
        assert_eq!(&src[edits[0].start..edits[0].end], "total");
        assert!(src[..edits[0].start].contains("if let total"));
    }

    #[test]
    fn follow_while_let_shadow_rewrites_outer_use_after_loop() {
        // WI-13: the shadowing `while let total` governs only the loop
        // body — the outer use after the loop rewrites.
        let src = "fn main() {\n    let total = check();\n    while let total = Some(1) { let x = total; break; }\n    let z = total;\n}\n";
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        let stmt_start = src.find("let total").unwrap();
        let stmt_end = src[stmt_start..].find(';').unwrap() + stmt_start + 1;
        let edits =
            let_binder_follow_edits(&parsed, src, &index, stmt_start, stmt_end, "total", "sum");
        assert_eq!(edits.len(), 1, "got: {edits:?}");
        assert_eq!(&src[edits[0].start..edits[0].end], "total");
        assert!(src[..edits[0].start].contains("while let total"));
    }

    #[test]
    fn follow_field_and_method_receivers_rewrite() {
        // WI-13: uses through a field / method receiver headed by the
        // binder are binder uses and rewrite in-hunk.
        let src = "fn main() {\n    let total = make();\n    let x = total.field;\n    let y = total.method();\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 2, "got: {edits:?}");
        for e in &edits {
            assert_eq!(e.replacement, "sum");
            assert_eq!(&src[e.start..e.end], "total");
        }
    }

    #[test]
    fn follow_nested_closure_capture_rewrites() {
        // WI-13: a capture inside a doubly-nested closure still names
        // the outer binder and rewrites.
        let src = "fn main() {\n    let total = 1;\n    let f = || { let g = || total + 1; g() };\n    let h = total;\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 2, "got: {edits:?}");
        for e in &edits {
            assert_eq!(e.replacement, "sum");
            assert_eq!(&src[e.start..e.end], "total");
        }
    }

    #[test]
    fn follow_shorthand_expands_to_field_value() {
        // Binder keeps the field, updates the value: `S { total }` →
        // `S { total: sum }` (not `S { sum }`, which would rename the
        // field and break `cargo check` with E0560).
        let src =
            "fn main() {\n    let total = 1;\n    let x = S { total };\n    let y = total;\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 2);
        let short = edits
            .iter()
            .find(|e| src[e.start..e.end] == *"total" && e.replacement.contains(':'))
            .expect("shorthand expansion present");
        assert_eq!(short.replacement, "total: sum");
        assert_eq!(short.label, Some("follow"));
        // Applying the edits yields the compilable shape.
        let post = crate::edits::apply_edits(
            src,
            edits
                .iter()
                .map(|e| crate::edits::TextEdit::new(e.start, e.end, e.replacement.clone()))
                .collect(),
        )
        .unwrap();
        assert!(post.contains("S { total: sum }"), "got: {post}");
        assert!(!post.contains("S { sum }"));
    }

    #[test]
    fn follow_explicit_struct_value_rewrites_plainly() {
        // `S { total: total }` already separates label from value: only
        // the value rewrites, via the plain path (no expansion needed).
        let src = "fn main() {\n    let total = 1;\n    let x = S { total: total };\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].replacement, "sum");
    }

    #[test]
    fn follow_shorthand_respects_shadow() {
        // Inner `let total` owns the inner `S { total }`; only the outer
        // shorthand expands.
        let src = "fn main() {\n    let total = 1;\n    {\n        let total = 2;\n        let b = S { total };\n    }\n    let d = S { total };\n}\n";
        let edits = follow_edits_for(src);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].replacement, "total: sum");
    }
}
