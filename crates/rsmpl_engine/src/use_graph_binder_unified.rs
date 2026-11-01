//! WI-3: one binder path keyed on the compiled binder (IMPROVE.md WI-3, D11).
//!
//! [`rsmpl_pattern::binder::binder_rename`] gives every binder spelling a
//! single identity; this module is the engine side: the downgrade gate,
//! binder-ident span location, and completion (`follow` edits) for every
//! non-`let` scope. `let` keeps the WI-2b block walker; every other scope
//! completes over its already-evidenced interior (closure body, arm
//! guard+body, loop body, then-branch, fn body).
//!
//! Emission discipline (Design 1): a binder rename touches only the
//! binder ident (labelled `rename`, parity with fn/item renames) plus
//! one `follow` edit per unshadowed in-scope value use. The whole-node
//! replacement is redundant — minus/plus are token-equal outside the
//! binder by construction — and dropping it keeps target formatting
//! intact. Shadowed, macro-hidden, nested-fn, qself, and type-position
//! uses stay orphans (reported, never rewritten).

use std::collections::HashMap;

use rsmpl_pattern::binder::{BinderRename, BinderScope};

/// Single binder entry over a compiled pattern plus the optional hunk
/// (the fn-param case needs the hunk: params live in `FnPattern`, not
/// in `PatternKind::Fn`).
pub fn binder_rename(
    compiled: &rsmpl_pattern::CompiledPattern,
    hunk: Option<&rsmpl_patch::RsmplHunk>,
) -> Option<BinderRename> {
    if let Some(b) = rsmpl_pattern::binder::binder_rename(&compiled.kind) {
        return Some(b);
    }
    if let Some(rh) = hunk {
        if let Some((old, new)) = crate::use_graph_binder::fn_param_binder_rename_from_hunk(rh) {
            return Some(BinderRename {
                old,
                new,
                scope: BinderScope::FnParam,
            });
        }
    }
    None
}

/// True when the hunk renames exactly one bound binder (any spelling).
/// Single gate: the no-hunk shapes defer to
/// [`crate::use_graph_local::is_local_binder_rename`], the hunk-needing
/// fn-param case is checked inline — exactly the shapes
/// [`binder_rename`] pairs.
pub fn is_binder_rename(
    compiled: &rsmpl_pattern::CompiledPattern,
    hunk: Option<&rsmpl_patch::RsmplHunk>,
) -> bool {
    crate::use_graph_local::is_local_binder_rename(compiled)
        || hunk.is_some_and(|rh| {
            crate::use_graph_binder::fn_param_binder_rename_from_hunk(rh).is_some()
        })
}

/// Recorded-binding adjustment for the postcondition probe (WI-3).
///
/// Completion rewrites unshadowed in-scope uses, so a recorded binding
/// whose text lies in the binder's scope re-matches in the post-image
/// with the rename applied. Returns the recorded map with exactly the
/// in-scope holes' values carrying the rename (AST-aware, mirroring
/// the follow walk: shadowed, macro-hidden, nested-fn, qself, and
/// type-position uses are left alone). Out-of-scope holes (`for`
/// iterators, scrutinees, `let` inits, types) re-match byte-identical
/// and are never adjusted. Identity for non-binder hunks and for
/// scopes with no in-scope holes (`Let`, `FnParam`).
pub fn probe_bindings(
    compiled: &rsmpl_pattern::CompiledPattern,
    hunk: Option<&rsmpl_patch::RsmplHunk>,
    recorded: &HashMap<String, String>,
) -> HashMap<String, String> {
    let Some(b) = binder_rename(compiled, hunk) else {
        return recorded.clone();
    };
    let holes = in_scope_holes(compiled, &b);
    if holes.is_empty() {
        return recorded.clone();
    }
    recorded
        .iter()
        .map(|(k, v)| {
            if holes.contains(k) {
                (
                    k.clone(),
                    rename_uses_in_fragment(v, &b.old, &b.new).unwrap_or_else(|| v.to_string()),
                )
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect()
}

/// Names of minus-side holes whose bound text lies in the binder's
/// scope (and only there): a hole occurring both inside and outside
/// the scope stays out (fail-safe — an unadjusted value mismatches
/// rather than mis-verifies).
fn in_scope_holes(
    compiled: &rsmpl_pattern::CompiledPattern,
    b: &BinderRename,
) -> std::collections::HashSet<String> {
    use rsmpl_pattern::GeneralShape;
    let mut out = std::collections::HashSet::new();
    let rsmpl_pattern::PatternKind::General {
        shape,
        minus_sub,
        placeholders,
        ..
    } = &compiled.kind
    else {
        return out;
    };
    // (parse, in-scope subtree render) per scope; `None` = no in-scope
    // holes (`Let` inits and `FnParam` bodies are out of scope).
    //
    // NOTE: `let ... else` keeps this edition-2021 compatible (no
    // let-chains); each arm falls back to `None` (no adjustment,
    // fail-safe) when the minus side does not parse as expected.
    let in_text: Option<String> = match (shape, b.scope) {
        (GeneralShape::Stmt, BinderScope::LetClosure) => {
            // In-scope = the init-closure body only (other params are
            // patterns, never rewritten).
            let Ok(syn::Stmt::Local(l)) = syn::parse_str::<syn::Stmt>(minus_sub) else {
                return out;
            };
            let init = match l.init.as_ref() {
                Some(i) => &*i.expr,
                None => return out,
            };
            // NOTE: field access must happen outside `quote!` —
            // `#c.body` would interpolate `c` plus a literal `.body`.
            match as_closure(init) {
                Some(c) => {
                    let body = &*c.body;
                    Some(quote::quote!(#body).to_string())
                }
                None => None,
            }
        }
        (GeneralShape::Expr, BinderScope::Closure) => {
            let Ok(syn::Expr::Closure(c)) = syn::parse_str::<syn::Expr>(minus_sub) else {
                return out;
            };
            let body = &*c.body;
            Some(quote::quote!(#body).to_string())
        }
        (GeneralShape::Expr, BinderScope::MatchArm) => {
            let Ok(syn::Expr::Match(m)) = syn::parse_str::<syn::Expr>(minus_sub) else {
                return out;
            };
            let mut parts = Vec::new();
            for arm in &m.arms {
                if single_ident(&arm.pat).map(|(n, _)| n) == Some(b.old.clone()) {
                    if let Some((_, g)) = &arm.guard {
                        parts.push(quote::quote!(#g).to_string());
                    }
                    let arm_body = &*arm.body;
                    parts.push(quote::quote!(#arm_body).to_string());
                }
            }
            Some(parts.join(" "))
        }
        (GeneralShape::Expr, BinderScope::For) => {
            let Ok(syn::Expr::ForLoop(f)) = syn::parse_str::<syn::Expr>(minus_sub) else {
                return out;
            };
            let body = &f.body;
            Some(quote::quote!(#body).to_string())
        }
        (GeneralShape::Expr, BinderScope::IfLet) => {
            let Ok(syn::Expr::If(i)) = syn::parse_str::<syn::Expr>(minus_sub) else {
                return out;
            };
            let then_branch = &i.then_branch;
            Some(quote::quote!(#then_branch).to_string())
        }
        (GeneralShape::Expr, BinderScope::WhileLet) => {
            let Ok(syn::Expr::While(w)) = syn::parse_str::<syn::Expr>(minus_sub) else {
                return out;
            };
            let body = &w.body;
            Some(quote::quote!(#body).to_string())
        }
        _ => None,
    };
    let Some(in_text) = in_text else {
        return out;
    };
    // Whole-minus token sets: a hole is in-scope iff one of its
    // placeholder idents occurs inside the in-scope subtree and none
    // occurs outside it.
    let in_tokens = ident_tokens(&in_text);
    let mut out_tokens = ident_tokens(minus_sub);
    for t in &in_tokens {
        out_tokens.remove(t);
    }
    for (name, ph) in placeholders {
        if in_tokens.contains(ph) && !out_tokens.contains(ph) {
            out.insert(name.clone());
        }
    }
    out
}

/// Whole ident tokens of `s` (placeholder search is token-exact, so
/// `__RSMPL_E` never fires inside `__RSMPL_ELLIPSIS_0`).
fn ident_tokens(s: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let mut cur = String::new();
    for ch in s.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else if !cur.is_empty() {
            set.insert(std::mem::take(&mut cur));
        }
    }
    set
}

/// Rename unshadowed `old` value uses in a bound fragment (the probe
/// side of completion). Mirrors the follow walk exactly: plain paths
/// and shorthand struct values rewrite; shadowed, macro-hidden,
/// nested-fn, qself, and type-position uses are left alone. Returns
/// `None` when the fragment is not a standalone expression (the caller
/// keeps the recorded value — fail-safe mismatch, never mis-verify).
fn rename_uses_in_fragment(text: &str, old: &str, new: &str) -> Option<String> {
    use syn::visit::Visit;
    let expr: syn::Expr = syn::parse_str(text).ok()?;
    // Fragment-relative byte offsets from proc-macro line:col spans
    // (columns are byte-based, so line starts + column = offset).
    let mut line_starts = vec![0usize];
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            line_starts.push(i + 1);
        }
    }
    let off = |sp: proc_macro2::Span| -> Option<usize> {
        let lc = sp.start();
        line_starts.get(lc.line - 1).map(|base| base + lc.column)
    };
    struct Walk<'a> {
        text: &'a str,
        old: &'a str,
        new: &'a str,
        off: &'a dyn Fn(proc_macro2::Span) -> Option<usize>,
        inner_defs: Vec<usize>,
        out: Vec<(usize, usize)>,
        shorthand_skip: Vec<(usize, usize)>,
        shorthand: Vec<(usize, usize, String)>,
    }
    impl Walk<'_> {
        fn ident_range(&self, sp: proc_macro2::Span) -> Option<(usize, usize)> {
            let a = (self.off)(sp)?;
            let b = a + self.old.len();
            if self.text.get(a..b) == Some(self.old) {
                Some((a, b))
            } else {
                None
            }
        }
    }
    impl<'ast> Visit<'ast> for Walk<'_> {
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            if node.ident == self.old {
                if let Some((a, _)) = self.ident_range(node.ident.span()) {
                    self.inner_defs.push(a);
                }
            }
            syn::visit::visit_pat_ident(self, node);
        }
        fn visit_item_fn(&mut self, _node: &'ast syn::ItemFn) {}
        fn visit_macro(&mut self, _node: &'ast syn::Macro) {}
        fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
            if node.qself.is_none()
                && node.path.leading_colon.is_none()
                && node.path.segments.len() == 1
                && node.path.segments[0].ident == self.old
                && node.path.segments[0].arguments.is_none()
            {
                if let Some((a, b)) = self.ident_range(node.path.segments[0].ident.span()) {
                    let is_shorthand = self.shorthand_skip.iter().any(|(s, e)| a < *e && *s < b);
                    if !is_shorthand && !self.inner_defs.iter().any(|d| *d < a) {
                        self.out.push((a, b));
                    }
                }
            }
            syn::visit::visit_expr_path(self, node);
        }
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
                if let Some((a, b)) = self.ident_range(member.span()) {
                    if !self.inner_defs.iter().any(|d| *d < a) {
                        self.shorthand_skip.push((a, b));
                        self.shorthand
                            .push((a, b, format!("{}: {}", self.old, self.new)));
                    }
                }
            }
            syn::visit::visit_expr_struct(self, node);
        }
    }
    let mut walk = Walk {
        text,
        old,
        new,
        off: &off,
        inner_defs: Vec::new(),
        out: Vec::new(),
        shorthand_skip: Vec::new(),
        shorthand: Vec::new(),
    };
    walk.visit_expr(&expr);
    // Phase two: the defs above are complete, so re-derive uses with
    // full shadow information (single pass suffices — rerun the walk).
    let inner_defs = {
        let mut v = walk.inner_defs.clone();
        v.sort_unstable();
        v.dedup();
        v
    };
    let mut walk2 = Walk {
        text,
        old,
        new,
        off: &off,
        inner_defs,
        out: Vec::new(),
        shorthand_skip: Vec::new(),
        shorthand: Vec::new(),
    };
    walk2.visit_expr(&expr);
    let mut edits: Vec<(usize, usize, String)> = walk2
        .out
        .into_iter()
        .map(|(a, b)| (a, b, new.to_string()))
        .collect();
    edits.extend(walk2.shorthand);
    edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = text.to_string();
    for (a, b, r) in edits {
        out.replace_range(a..b, &r);
    }
    Some(out)
}

/// Byte span of the renamed binder ident inside a matched `let`
/// statement target: the `let` pat ident (`Let`), or the init-closure
/// parameter (`LetClosure`, one paren layer tolerated).
pub fn local_binder_span(
    tgt: &syn::Local,
    scope: BinderScope,
    old: &str,
    original: &str,
    index: &crate::lines::LineIndex,
) -> Option<(usize, usize)> {
    match scope {
        BinderScope::Let => {
            let (name, _) = single_ident(&tgt.pat)?;
            if name != old {
                return None;
            }
            ident_span_bytes(original, index, &pat_ident_span(&tgt.pat)?)
        }
        BinderScope::LetClosure => {
            let init = tgt.init.as_ref().map(|i| &*i.expr)?;
            let closure = as_closure(init)?;
            for inp in &closure.inputs {
                if let Some((name, _)) = single_ident(inp) {
                    if name == old {
                        return pat_ident_span(inp)
                            .and_then(|sp| ident_span_bytes(original, index, &sp));
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Byte span of the renamed binder ident inside a matched expression
/// target of the given scope.
pub fn expr_binder_span(
    tgt: &syn::Expr,
    scope: BinderScope,
    old: &str,
    original: &str,
    index: &crate::lines::LineIndex,
) -> Option<(usize, usize)> {
    match (tgt, scope) {
        (syn::Expr::Closure(c), BinderScope::Closure) => {
            for inp in &c.inputs {
                if single_ident(inp).map(|(n, _)| n) == Some(old.to_string()) {
                    return pat_ident_span(inp)
                        .and_then(|sp| ident_span_bytes(original, index, &sp));
                }
            }
            None
        }
        (syn::Expr::Match(m), BinderScope::MatchArm) => {
            for arm in &m.arms {
                if single_ident(&arm.pat).map(|(n, _)| n) == Some(old.to_string()) {
                    return pat_ident_span(&arm.pat)
                        .and_then(|sp| ident_span_bytes(original, index, &sp));
                }
            }
            None
        }
        (syn::Expr::ForLoop(f), BinderScope::For) => {
            if single_ident(&f.pat).map(|(n, _)| n) == Some(old.to_string()) {
                pat_ident_span(&f.pat).and_then(|sp| ident_span_bytes(original, index, &sp))
            } else {
                None
            }
        }
        (syn::Expr::If(i), BinderScope::IfLet) => {
            let syn::Expr::Let(l) = &*i.cond else {
                return None;
            };
            if single_ident(&l.pat).map(|(n, _)| n) == Some(old.to_string()) {
                pat_ident_span(&l.pat).and_then(|sp| ident_span_bytes(original, index, &sp))
            } else {
                None
            }
        }
        (syn::Expr::While(w), BinderScope::WhileLet) => {
            let syn::Expr::Let(l) = &*w.cond else {
                return None;
            };
            if single_ident(&l.pat).map(|(n, _)| n) == Some(old.to_string()) {
                pat_ident_span(&l.pat).and_then(|sp| ident_span_bytes(original, index, &sp))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Byte span of the renamed fn-param binder ident in `sig`.
pub fn fn_param_binder_span(
    sig: &syn::Signature,
    old: &str,
    original: &str,
    index: &crate::lines::LineIndex,
) -> Option<(usize, usize)> {
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(t) = arg {
            if single_ident(&t.pat).map(|(n, _)| n) == Some(old.to_string()) {
                return pat_ident_span(&t.pat)
                    .and_then(|sp| ident_span_bytes(original, index, &sp));
            }
        }
    }
    None
}

/// Completion edits for a non-`let` binder scope: one `follow`-labelled
/// ident replacement per unshadowed in-scope value use of `old`.
///
/// `def_spans` are this match's definition spans (binder ident, plus the
/// matched node span in expression shapes); regions are the scope nodes
/// *containing* them (reversed containment: definition edits are
/// binder-ident tight, never whole-node). Shadowed, macro-hidden,
/// nested-fn, qself, and type-position uses are excluded — they stay
/// orphans via the scope scans.
pub fn scope_follow_edits(
    parsed: &syn::File,
    original: &str,
    index: &crate::lines::LineIndex,
    scope: BinderScope,
    old: &str,
    new: &str,
    def_spans: &[(usize, usize)],
) -> Vec<crate::edits::TextEdit> {
    if scope == BinderScope::Let {
        return Vec::new();
    }
    let scopes = scope_regions(parsed, original, index, scope, old, def_spans);
    if scopes.is_empty() {
        return Vec::new();
    }
    follow_in_regions(parsed, original, index, old, new, &scopes)
}

/// Per-scope regions: (in-scope regions, renamed binder ident spans).
type NodeScopes = Vec<(Vec<(usize, usize)>, Vec<(usize, usize)>)>;

/// Scope nodes containing a definition span and binding `old`.
fn scope_regions(
    parsed: &syn::File,
    original: &str,
    index: &crate::lines::LineIndex,
    scope: BinderScope,
    old: &str,
    def_spans: &[(usize, usize)],
) -> NodeScopes {
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    struct Grab<'a> {
        original: &'a str,
        index: &'a crate::lines::LineIndex,
        old: &'a str,
        def_spans: &'a [(usize, usize)],
        scope: BinderScope,
        kept: Vec<(usize, usize)>,
        scopes: NodeScopes,
    }
    impl Grab<'_> {
        /// Reversed containment: the scope node contains a definition
        /// span (binder-ident tight emission never covers the node).
        fn contains_def(&self, a: usize, b: usize) -> bool {
            self.def_spans.iter().any(|(s, e)| a <= *s && *e <= b)
        }
        fn fresh(&self, a: usize, b: usize) -> bool {
            !self.kept.iter().any(|(s, e)| *s <= a && b <= *e)
        }
        fn binders_of(&self, pat: &syn::Pat) -> Vec<(usize, usize)> {
            let mut out = Vec::new();
            for off in
                crate::use_graph_local::pat_old_offsets(pat, self.old, self.original, self.index)
            {
                let end = off + self.old.len();
                if self.original.get(off..end) == Some(self.old) {
                    out.push((off, end));
                }
            }
            out
        }
        fn claim(
            &mut self,
            node: (usize, usize),
            regions: Vec<(usize, usize)>,
            binders: Vec<(usize, usize)>,
        ) {
            if binders.is_empty() || regions.is_empty() {
                return;
            }
            self.kept.push(node);
            self.scopes.push((regions, binders));
        }
        fn body_of(&self, block: &syn::Block) -> Option<(usize, usize)> {
            use syn::spanned::Spanned;
            crate::matcher_item::span_bytes(self.original, self.index, block.span())
        }
    }
    impl<'ast> Visit<'ast> for Grab<'_> {
        fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
            if matches!(self.scope, BinderScope::Closure | BinderScope::LetClosure) {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contains_def(ab.0, ab.1) && self.fresh(ab.0, ab.1) {
                        let mut binders = Vec::new();
                        for inp in &node.inputs {
                            binders.extend(self.binders_of(inp));
                        }
                        let regions = crate::matcher_item::span_bytes(
                            self.original,
                            self.index,
                            node.body.span(),
                        )
                        .map(|r| vec![r])
                        .unwrap_or_default();
                        self.claim(ab, regions, binders);
                    }
                }
            }
            syn::visit::visit_expr_closure(self, node);
        }
        fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
            if self.scope == BinderScope::MatchArm {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contains_def(ab.0, ab.1) && self.fresh(ab.0, ab.1) {
                        let mut binders = Vec::new();
                        let mut regions = Vec::new();
                        for arm in &node.arms {
                            if !matches!(&arm.pat, syn::Pat::Ident(pid) if pid.subpat.is_none()) {
                                continue;
                            }
                            let arm_binders = self.binders_of(&arm.pat);
                            if arm_binders.is_empty() {
                                continue;
                            }
                            if let Some((_, guard)) = &arm.guard {
                                if let Some(g) = crate::matcher_item::span_bytes(
                                    self.original,
                                    self.index,
                                    guard.span(),
                                ) {
                                    regions.push(g);
                                }
                            }
                            if let Some(body) = crate::matcher_item::span_bytes(
                                self.original,
                                self.index,
                                arm.body.span(),
                            ) {
                                regions.push(body);
                                binders.extend(arm_binders);
                            }
                        }
                        self.claim(ab, regions, binders);
                    }
                }
            }
            syn::visit::visit_expr_match(self, node);
        }
        fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
            if self.scope == BinderScope::For {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contains_def(ab.0, ab.1) && self.fresh(ab.0, ab.1) {
                        let binders = self.binders_of(&node.pat);
                        let regions = self
                            .body_of(&node.body)
                            .map(|r| vec![r])
                            .unwrap_or_default();
                        self.claim(ab, regions, binders);
                    }
                }
            }
            syn::visit::visit_expr_for_loop(self, node);
        }
        fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
            if self.scope == BinderScope::IfLet {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contains_def(ab.0, ab.1) && self.fresh(ab.0, ab.1) {
                        if let syn::Expr::Let(let_) = &*node.cond {
                            let binders = self.binders_of(&let_.pat);
                            let regions = crate::matcher_item::span_bytes(
                                self.original,
                                self.index,
                                node.then_branch.span(),
                            )
                            .map(|r| vec![r])
                            .unwrap_or_default();
                            self.claim(ab, regions, binders);
                        }
                    }
                }
            }
            syn::visit::visit_expr_if(self, node);
        }
        fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
            if self.scope == BinderScope::WhileLet {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contains_def(ab.0, ab.1) && self.fresh(ab.0, ab.1) {
                        if let syn::Expr::Let(let_) = &*node.cond {
                            let binders = self.binders_of(&let_.pat);
                            let regions = self
                                .body_of(&node.body)
                                .map(|r| vec![r])
                                .unwrap_or_default();
                            self.claim(ab, regions, binders);
                        }
                    }
                }
            }
            syn::visit::visit_expr_while(self, node);
        }
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            if self.scope == BinderScope::FnParam {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contains_def(ab.0, ab.1) && self.fresh(ab.0, ab.1) {
                        let mut binders = Vec::new();
                        for arg in &node.sig.inputs {
                            if let syn::FnArg::Typed(t) = arg {
                                binders.extend(self.binders_of(&t.pat));
                            }
                        }
                        let regions = self
                            .body_of(&node.block)
                            .map(|r| vec![r])
                            .unwrap_or_default();
                        self.claim(ab, regions, binders);
                    }
                }
            }
            syn::visit::visit_item_fn(self, node);
        }
        fn visit_local(&mut self, node: &'ast syn::Local) {
            // LetClosure: the `let` statement contains the init closure;
            // claim the closure through the statement (the grab above
            // already handles bare-closure shapes via containment).
            if self.scope == BinderScope::LetClosure {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if !self.contains_def(ab.0, ab.1) {
                        syn::visit::visit_local(self, node);
                        return;
                    }
                    if let Some(init) = node.init.as_ref().map(|i| &*i.expr) {
                        if let Some(closure) = as_closure(init) {
                            if let Some(cb) = crate::matcher_item::span_bytes(
                                self.original,
                                self.index,
                                closure.span(),
                            ) {
                                if self.fresh(cb.0, cb.1) {
                                    let mut binders = Vec::new();
                                    for inp in &closure.inputs {
                                        binders.extend(self.binders_of(inp));
                                    }
                                    let regions = crate::matcher_item::span_bytes(
                                        self.original,
                                        self.index,
                                        closure.body.span(),
                                    )
                                    .map(|r| vec![r])
                                    .unwrap_or_default();
                                    self.claim(cb, regions, binders);
                                }
                            }
                        }
                    }
                }
            }
            syn::visit::visit_local(self, node);
        }
    }
    let mut grab = Grab {
        original,
        index,
        old,
        def_spans,
        scope,
        kept: Vec::new(),
        scopes: Vec::new(),
    };
    grab.visit_file(parsed);
    grab.scopes
}

/// Emit `follow` edits for unshadowed value-path uses of `old` in
/// `scopes` (mirror of the WI-2b `Uses` walk, minus the block gate:
/// regions are already scope-precise). Shorthand struct values expand
/// (`S { old }` → `S { old: new }`); shadowed, macro-hidden, nested-fn,
/// qself, and type-position uses are left for the orphan scans.
fn follow_in_regions(
    parsed: &syn::File,
    original: &str,
    index: &crate::lines::LineIndex,
    old: &str,
    new: &str,
    scopes: &NodeScopes,
) -> Vec<crate::edits::TextEdit> {
    use syn::visit::Visit;

    /// Phase one: collect every inner `old` binder in the regions.
    /// (Shadowing must be fully known before any use classifies.)
    struct Defs<'a, 'b> {
        original: &'a str,
        index: &'a crate::lines::LineIndex,
        old: &'a str,
        scopes: &'b NodeScopes,
        inner_defs: Vec<usize>,
    }
    impl<'ast> Visit<'ast> for Defs<'_, '_> {
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            if node.ident == self.old {
                let sp = node.ident.span();
                if let (Some(a), Some(b)) = (
                    self.index.offset(sp.start().line as u32, sp.start().column),
                    self.index.offset(sp.end().line as u32, sp.end().column),
                ) {
                    let in_region = self.scopes.iter().any(|(rs, bs)| {
                        rs.iter().any(|(s, e)| *s <= a && b <= *e)
                            && !bs.iter().any(|(s, e)| a < *e && *s < b)
                    });
                    if self.original.get(a..b) == Some(self.old) && in_region {
                        self.inner_defs.push(a);
                    }
                }
            }
            syn::visit::visit_pat_ident(self, node);
        }
    }

    // Nested fns + macros intersecting the regions (excluded from follows).
    struct Fences<'a> {
        original: &'a str,
        index: &'a crate::lines::LineIndex,
        scopes: &'a NodeScopes,
        item_fns: Vec<(usize, usize)>,
        macros: Vec<(usize, usize)>,
    }
    impl<'ast> Visit<'ast> for Fences<'_> {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            use syn::spanned::Spanned;
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                let inside = self
                    .scopes
                    .iter()
                    .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e));
                if inside {
                    self.item_fns.push((a, b));
                }
            }
            syn::visit::visit_item_fn(self, node);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            use syn::spanned::Spanned;
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                let inside = self
                    .scopes
                    .iter()
                    .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e));
                if inside {
                    self.macros.push((a, b));
                }
            }
        }
    }
    let mut fences = Fences {
        original,
        index,
        scopes,
        item_fns: Vec::new(),
        macros: Vec::new(),
    };
    fences.visit_file(parsed);

    // Phase one: the full inner-def set (shadowing must be known before
    // any use classifies; visit order alone cannot guarantee defs come
    // first).
    let mut defs = Defs {
        original,
        index,
        old,
        scopes,
        inner_defs: Vec::new(),
    };
    defs.visit_file(parsed);
    let mut inner_defs = defs.inner_defs;
    inner_defs.sort_unstable();
    inner_defs.dedup();
    // Phase two: the use walk with complete shadow information. Shadow
    // discipline mirrors `interior_reports` exactly (same predicate, so
    // follow-set and orphan-set partition the interior uses): a use past
    // any inner def is `shadowed` (left for orphans), anything else
    // unshadowed rewrites here.
    struct Uses2<'a, 'b> {
        original: &'a str,
        index: &'a crate::lines::LineIndex,
        old: &'a str,
        new: &'a str,
        scopes: &'b NodeScopes,
        item_fns: Vec<(usize, usize)>,
        macros: Vec<(usize, usize)>,
        inner_defs: Vec<usize>,
        shorthand_skip: Vec<(usize, usize)>,
        out: Vec<(usize, usize)>,
        shorthand_edits: Vec<(usize, usize, String)>,
    }
    impl Uses2<'_, '_> {
        fn in_region(&self, a: usize, b: usize) -> bool {
            self.scopes.iter().any(|(rs, bs)| {
                rs.iter().any(|(s, e)| *s <= a && b <= *e)
                    && !bs.iter().any(|(s, e)| a < *e && *s < b)
            })
        }
        fn shadowed(&self, a: usize) -> bool {
            self.inner_defs.iter().any(|d| *d < a)
        }
        fn fenced(&self, a: usize) -> bool {
            self.item_fns.iter().any(|r| r.0 <= a && a < r.1)
                || self.macros.iter().any(|r| r.0 <= a && a < r.1)
        }
        fn strictly_inside_region(&self, a: usize, b: usize) -> bool {
            self.scopes
                .iter()
                .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e))
        }
    }
    impl<'ast> Visit<'ast> for Uses2<'_, '_> {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            use syn::spanned::Spanned;
            // Nested fns are a different scope; the region-owning fn (for
            // `FnParam`) must still be descended into, so only skip fns
            // strictly inside a region.
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                if self.strictly_inside_region(a, b) {
                    return;
                }
            }
            syn::visit::visit_item_fn(self, node);
        }
        fn visit_macro(&mut self, _node: &'ast syn::Macro) {}
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
                    let is_shorthand = self.shorthand_skip.iter().any(|(s, e)| a < *e && *s < b);
                    if !is_shorthand
                        && self.original.get(a..b) == Some(self.old)
                        && self.in_region(a, b)
                        && !self.fenced(a)
                        && !self.shadowed(a)
                    {
                        self.out.push((a, b));
                    }
                }
            }
            syn::visit::visit_expr_path(self, node);
        }
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
                    || !self.in_region(a, b)
                    || self.fenced(a)
                    || self.shadowed(a)
                {
                    continue;
                }
                self.shorthand_skip.push((a, b));
                self.shorthand_edits
                    .push((a, b, format!("{}: {}", self.old, self.new)));
            }
            syn::visit::visit_expr_struct(self, node);
        }
    }
    let mut uses2 = Uses2 {
        original,
        index,
        old,
        new,
        scopes,
        item_fns: fences.item_fns,
        macros: fences.macros,
        inner_defs,
        shorthand_skip: Vec::new(),
        out: Vec::new(),
        shorthand_edits: Vec::new(),
    };
    uses2.visit_file(parsed);
    let mut edits: Vec<crate::edits::TextEdit> = uses2
        .out
        .into_iter()
        .map(|(a, b)| crate::edits::TextEdit::new(a, b, new.to_string()).with_label("follow"))
        .collect();
    edits.extend(
        uses2
            .shorthand_edits
            .into_iter()
            .map(|(a, b, r)| crate::edits::TextEdit::new(a, b, r).with_label("follow")),
    );
    edits.sort_by_key(|e| (e.start, e.end));
    edits.dedup_by(|a, b| a.start == b.start && a.end == b.end);
    edits
}

/// Single `x` binder (no subpattern); placeholder idents never count.
fn single_ident(pat: &syn::Pat) -> Option<(String, bool)> {
    match pat {
        syn::Pat::Ident(pid) => {
            if pid.subpat.is_some() {
                return None;
            }
            let name = pid.ident.to_string();
            if name.starts_with("__RSMPL_") {
                return None;
            }
            Some((name, pid.mutability.is_some()))
        }
        syn::Pat::Type(pt) => single_ident(&pt.pat),
        _ => None,
    }
}

/// Span of the bound ident itself (through `Type` wrappers).
fn pat_ident_span(pat: &syn::Pat) -> Option<proc_macro2::Span> {
    match pat {
        syn::Pat::Ident(pid) => Some(pid.ident.span()),
        syn::Pat::Type(pt) => pat_ident_span(&pt.pat),
        _ => None,
    }
}

fn ident_span_bytes(
    original: &str,
    index: &crate::lines::LineIndex,
    span: &proc_macro2::Span,
) -> Option<(usize, usize)> {
    let (a, b) = crate::matcher_item::span_bytes(original, index, *span)?;
    if original.get(a..b).is_some() {
        Some((a, b))
    } else {
        None
    }
}

/// Unwrap one layer: closures directly in the init, or parenthesised.
fn as_closure(expr: &syn::Expr) -> Option<&syn::ExprClosure> {
    match expr {
        syn::Expr::Closure(c) => Some(c),
        syn::Expr::Paren(p) => as_closure(&p.expr),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(src: &str) -> (syn::File, crate::lines::LineIndex) {
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        (parsed, index)
    }

    #[test]
    fn closure_scope_rewrites_body_uses() {
        let src = "fn main() {\n    let f = |total: u32| total + total;\n    let _ = f;\n}\n";
        let (parsed, index) = file(src);
        let cl = src.find("|total").unwrap();
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::Closure,
            "total",
            "sum",
            &[(cl, cl + 7)],
        );
        assert_eq!(edits.len(), 2, "got: {edits:?}");
        for e in &edits {
            assert_eq!(e.replacement, "sum");
            assert_eq!(e.label, Some("follow"));
        }
    }

    #[test]
    fn closure_scope_shadowed_use_excluded() {
        let src =
            "fn main() {\n    let f = |total: u32| { let total = 1; total };\n    let _ = f;\n}\n";
        let (parsed, index) = file(src);
        let cl = src.find("|total").unwrap();
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::Closure,
            "total",
            "sum",
            &[(cl, cl + 7)],
        );
        assert!(edits.is_empty(), "got: {edits:?}");
    }

    #[test]
    fn closure_scope_macro_use_excluded() {
        let src = "fn main() {\n    let f = |total: u32| { println!(\"{}\", total); total };\n    let _ = f;\n}\n";
        let (parsed, index) = file(src);
        let cl = src.find("|total").unwrap();
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::Closure,
            "total",
            "sum",
            &[(cl, cl + 7)],
        );
        assert_eq!(edits.len(), 1, "got: {edits:?}");
    }

    #[test]
    fn for_scope_rewrites_body() {
        let src = "fn main() {\n    for total in 0..3 {\n        let x = total;\n    }\n}\n";
        let (parsed, index) = file(src);
        let pat = src.find("total in").unwrap();
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::For,
            "total",
            "sum",
            &[(pat, pat + 5)],
        );
        assert_eq!(edits.len(), 1, "got: {edits:?}");
        assert_eq!(&src[edits[0].start..edits[0].end], "total");
    }

    #[test]
    fn fn_param_scope_rewrites_body() {
        let src = "fn foo(total: u32) -> u32 {\n    total + 1\n}\n";
        let (parsed, index) = file(src);
        let param = src.find("total").unwrap();
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::FnParam,
            "total",
            "sum",
            &[(param, param + 5)],
        );
        assert_eq!(edits.len(), 1, "got: {edits:?}");
    }

    #[test]
    fn fn_param_scope_ignores_other_fn() {
        let src = "fn foo(total: u32) -> u32 {\n    1\n}\nfn bar() {\n    let total = 1;\n    let _ = total;\n}\n";
        let (parsed, index) = file(src);
        let param = src.find("total").unwrap();
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::FnParam,
            "total",
            "sum",
            &[(param, param + 5)],
        );
        assert!(edits.is_empty(), "got: {edits:?}");
    }

    #[test]
    fn let_scope_never_emits_here() {
        let src = "fn main() {\n    let total = 1;\n    let x = total;\n}\n";
        let (parsed, index) = file(src);
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::Let,
            "total",
            "sum",
            &[(0, 1)],
        );
        assert!(edits.is_empty());
    }

    #[test]
    fn shorthand_expands_in_closure_body() {
        let src = "fn main() {\n    let f = |total: u32| S { total };\n    let _ = f;\n}\n";
        let (parsed, index) = file(src);
        let cl = src.find("|total").unwrap();
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::Closure,
            "total",
            "sum",
            &[(cl, cl + 7)],
        );
        assert_eq!(edits.len(), 1, "got: {edits:?}");
        assert_eq!(edits[0].replacement, "total: sum");
    }

    #[test]
    fn match_arm_scope_rewrites_body() {
        let src =
            "fn main() {\n    let v = match Some(1) { total => total + 1, _ => 0 };\n    let _ = v;\n}\n";
        let (parsed, index) = file(src);
        let mstart = src.find("match").unwrap();
        let mend = src.find("};").unwrap() + 1;
        let edits = scope_follow_edits(
            &parsed,
            src,
            &index,
            BinderScope::MatchArm,
            "total",
            "sum",
            &[(mstart, mend)],
        );
        assert_eq!(edits.len(), 1, "got: {edits:?}");
        assert_eq!(&src[edits[0].start..edits[0].end], "total");
    }

    #[test]
    fn unified_identity_covers_all_spellings() {
        // The single entry agrees with each legacy detector on its shape.
        let mk = |kind| rsmpl_pattern::CompiledPattern {
            kind,
            table: Default::default(),
            minus_bindings: Default::default(),
            plus_bindings: Default::default(),
            unbound_plus: Vec::new(),
            repeated: Vec::new(),
            root: None,
        };
        let stmt = |m: &str, p: &str| {
            mk(rsmpl_pattern::PatternKind::General {
                shape: rsmpl_pattern::GeneralShape::Stmt,
                minus_sub: m.to_string(),
                plus_raw: p.to_string(),
                plus_body: p.to_string(),
                placeholders: vec![("E".to_string(), "__RSMPL_E0".to_string())],
                literals: Vec::new(),
                minus_attrs: Vec::new(),
                plus_attrs: Vec::new(),
                had_semi: false,
            })
        };
        assert!(is_binder_rename(
            &stmt("let total = __RSMPL_E0;", "let sum = E;"),
            None
        ));
        assert!(is_binder_rename(
            &stmt("let g = | total | __RSMPL_E0 ;", "let g = | sum | E ;"),
            None
        ));
        assert!(!is_binder_rename(
            &stmt("let total = __RSMPL_E0;", "let sum = 2;"),
            None
        ));
    }
}
