//! WI-2c: scope-interior orphan evidence for closure-param / match-arm
//! binder renames (IMPROVE.md WI-2 slice, third cut).
//!
//! Sibling of [`crate::use_graph_local`] (D5: new code lands in new
//! modules). A closure parameter is visible only in the closure body;
//! a match-arm binder only in its arm's guard+body — so uses outside
//! the rewritten node can never dangle and only the rewritten node's
//! interior is scanned (see `closure_or_arm_orphans`).

use std::collections::HashMap;

use crate::edits::TextEdit;
use crate::lines::{FileState, LineIndex};
use crate::use_graph_local::{literal_captures, orphan_line_text, pat_old_offsets};

/// WI-2c: per-node scope evidence — one (scope regions, renamed
/// binder ident spans) pair per matched closure/match node.
type NodeScopes = Vec<(Vec<(usize, usize)>, Vec<(usize, usize)>)>;

/// WI-2c: scope-interior orphan evidence for closure-param / match-arm
/// binder renames.
///
/// A closure parameter is visible only in the closure body; a match-arm
/// binder only in its arm's guard+body. Uses outside the rewritten node
/// can never dangle, so unlike the `let` scan this reports only the
/// `old` uses *inside* the rewritten node that the pattern preserved
/// (hole-bound regions). The renamed binder ident itself is excluded;
/// uses past an inner same-name binder in the region are `shadowed`,
/// other interior uses are `off-target`; macro interiors and inline
/// `{old}` captures in scope are `macro-hidden`. Only the hunk's own
/// file is scanned — cross-file leftovers are impossible by scope, and
/// scanning other files would flag unrelated same-name bindings.
pub(crate) fn closure_or_arm_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    old: &str,
    is_closure: bool,
) -> Vec<crate::OrphanReport> {
    use syn::visit::Visit;

    let Some(st) = states.get(path) else {
        return Vec::new();
    };
    let Some(parsed) = st.parsed.as_ref() else {
        return Vec::new();
    };
    let original = st.original.as_str();
    let index = &st.index;
    let spans: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    if spans.is_empty() {
        return Vec::new();
    }

    // (scope regions, renamed binder ident spans) per matched node.
    // syn visits pre-order, so the outermost contained node (the
    // matched one) records before any nested same-kind node; a node
    // inside an already-kept node belongs to its preserved body and
    // is covered by the shadow discipline, never analysed twice.
    struct ScopeGrab<'a> {
        original: &'a str,
        index: &'a LineIndex,
        old: &'a str,
        spans: &'a [(usize, usize)],
        is_closure: bool,
        kept: Vec<(usize, usize)>,
        scopes: NodeScopes,
    }
    impl ScopeGrab<'_> {
        // WI-3 reversed containment: definition edits are binder-ident
        // tight (never whole-node), so the scope node *contains* a
        // definition span. Whole-node spans still satisfy this
        // (equality keeps the legacy tests green).
        fn contained(&self, a: usize, b: usize) -> bool {
            self.spans.iter().any(|(s, e)| a <= *s && *e <= b)
        }
        fn binders_of(&self, pat: &syn::Pat) -> Vec<(usize, usize)> {
            let mut out = Vec::new();
            for off in pat_old_offsets(pat, self.old, self.original, self.index) {
                let end = off + self.old.len();
                if self.original.get(off..end) == Some(self.old) {
                    out.push((off, end));
                }
            }
            out
        }
    }
    impl<'ast> Visit<'ast> for ScopeGrab<'_> {
        fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
            use syn::spanned::Spanned;
            if self.is_closure {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contained(ab.0, ab.1)
                        && !self.kept.iter().any(|(s, e)| *s <= ab.0 && ab.1 <= *e)
                    {
                        let mut binders: Vec<(usize, usize)> = Vec::new();
                        for inp in &node.inputs {
                            binders.extend(self.binders_of(inp));
                        }
                        if !binders.is_empty() {
                            if let Some(body) = crate::matcher_item::span_bytes(
                                self.original,
                                self.index,
                                node.body.span(),
                            ) {
                                self.kept.push(ab);
                                self.scopes.push((vec![body], binders));
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr_closure(self, node);
        }
        fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
            use syn::spanned::Spanned;
            if !self.is_closure {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contained(ab.0, ab.1)
                        && !self.kept.iter().any(|(s, e)| *s <= ab.0 && ab.1 <= *e)
                    {
                        // Every single-ident `old` arm pattern in the
                        // node is a renamed binder (the pattern renames
                        // positionally; extra same-name arms are
                        // unreachable-but-parseable twins).
                        let mut binders: Vec<(usize, usize)> = Vec::new();
                        let mut regions: Vec<(usize, usize)> = Vec::new();
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
                            let Some(body) = crate::matcher_item::span_bytes(
                                self.original,
                                self.index,
                                arm.body.span(),
                            ) else {
                                continue;
                            };
                            regions.push(body);
                            binders.extend(arm_binders);
                        }
                        if !binders.is_empty() && !regions.is_empty() {
                            self.kept.push(ab);
                            self.scopes.push((regions, binders));
                        }
                    }
                }
            }
            syn::visit::visit_expr_match(self, node);
        }
    }
    let mut grab = ScopeGrab {
        original,
        index,
        old,
        spans: &spans,
        is_closure,
        kept: Vec::new(),
        scopes: Vec::new(),
    };
    grab.visit_file(parsed);
    let scopes = grab.scopes;
    if scopes.is_empty() {
        return Vec::new();
    }

    interior_reports(
        parsed,
        original,
        index,
        path,
        &scopes,
        old,
        &follow_spans(file_edits, path, edits_before),
    )
}

/// Shared scope-interior report builder (WI-2c + WI-2d): value-path
/// uses in the regions (binder spans excluded) are `off-target` past
/// no inner binder and `shadowed` past one; in-scope macro interiors
/// and inline `{old}` captures are `macro-hidden`.
///
/// WI-3 `rewritten` are the hunk's `follow`-labelled spans: uses the
/// completion already rewrote are not orphans (the follow walk and this
/// scan implement the same shadow predicate, so the sets partition).
fn interior_reports(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    path: &str,
    scopes: &NodeScopes,
    old: &str,
    rewritten: &[(usize, usize)],
) -> Vec<crate::OrphanReport> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    struct Interior<'a> {
        original: &'a str,
        index: &'a LineIndex,
        old: &'a str,
        scopes: &'a NodeScopes,
        rewritten: &'a [(usize, usize)],
        uses: Vec<(proc_macro2::Span, usize, usize)>,
        inner_defs: Vec<(usize, usize)>,
        opaque_lines: Vec<u32>,
    }
    impl<'ast> Visit<'ast> for Interior<'_> {
        fn visit_path(&mut self, node: &'ast syn::Path) {
            for seg in &node.segments {
                if seg.ident == self.old {
                    let sp = seg.ident.span();
                    if let Some((a, b)) =
                        crate::matcher_item::span_bytes(self.original, self.index, sp)
                    {
                        // WI-3: completed uses are evidence, not orphans.
                        let done = self.rewritten.iter().any(|(s, e)| a < *e && *s < b);
                        if !done
                            && self.original.get(a..b) == Some(self.old)
                            && self.scopes.iter().any(|(rs, bs)| {
                                rs.iter().any(|(s, e)| *s <= a && b <= *e)
                                    && !bs.iter().any(|(s, e)| a < *e && *s < b)
                            })
                        {
                            self.uses.push((sp, a, b));
                        }
                    }
                }
            }
            syn::visit::visit_path(self, node);
        }
        fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
            if node.qself.is_some() {
                let mut hit = false;
                for seg in &node.path.segments {
                    if seg.ident == self.old {
                        hit = true;
                        // One entry per matching ident line (WI-2b
                        // ident-line parity — not the qself start line),
                        // gated on the ident itself sitting in scope.
                        if let Some((a, b)) = crate::matcher_item::span_bytes(
                            self.original,
                            self.index,
                            seg.ident.span(),
                        ) {
                            if self
                                .scopes
                                .iter()
                                .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e))
                            {
                                self.opaque_lines.push(seg.ident.span().start().line as u32);
                            }
                        }
                    }
                }
                if hit {
                    return;
                }
            }
            syn::visit::visit_type_path(self, node);
        }
        fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
            if node.qself.is_some() {
                let mut hit = false;
                for seg in &node.path.segments {
                    if seg.ident == self.old {
                        hit = true;
                        if let Some((a, b)) = crate::matcher_item::span_bytes(
                            self.original,
                            self.index,
                            seg.ident.span(),
                        ) {
                            if self
                                .scopes
                                .iter()
                                .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e))
                            {
                                self.opaque_lines.push(seg.ident.span().start().line as u32);
                            }
                        }
                    }
                }
                if hit {
                    return;
                }
            }
            syn::visit::visit_expr_path(self, node);
        }
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            if node.ident == self.old {
                let sp = node.ident.span();
                if let (Some(a), Some(b)) = (
                    self.index.offset(sp.start().line as u32, sp.start().column),
                    self.index.offset(sp.end().line as u32, sp.end().column),
                ) {
                    if self.original.get(a..b) == Some(self.old)
                        && self.scopes.iter().any(|(rs, bs)| {
                            rs.iter().any(|(s, e)| *s <= a && b <= *e)
                                && !bs.iter().any(|(s, e)| a < *e && *s < b)
                        })
                    {
                        self.inner_defs.push((a, b));
                    }
                }
            }
            syn::visit::visit_pat_ident(self, node);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                if self
                    .scopes
                    .iter()
                    .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e))
                {
                    for line in macro_opaque_lines(&node.tokens, self.old) {
                        self.opaque_lines.push(line);
                    }
                }
            }
            // Macro interiors stay opaque (never descended into).
        }
        fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
            // Attribute interiors are opaque, never claimed as uses (an
            // ident inside `#[doc = old]` never resolves to the binder).
            // Gate on the attribute sitting in-scope (same containment as
            // macros): out-of-scope attributes (e.g. on the closure node
            // itself) name the outer scope and never dangle. String
            // literals stay silent by construction (`Literal`, never
            // `Ident`).
            use syn::spanned::Spanned;
            let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            else {
                return;
            };
            if !self
                .scopes
                .iter()
                .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e))
            {
                return;
            }
            for seg in node.path().segments.iter() {
                if seg.ident == self.old {
                    self.opaque_lines.push(seg.ident.span().start().line as u32);
                }
            }
            match &node.meta {
                syn::Meta::NameValue(nv) => {
                    struct ValuePaths<'o> {
                        old: &'o str,
                        opaque: Vec<u32>,
                    }
                    impl<'ast> syn::visit::Visit<'ast> for ValuePaths<'_> {
                        fn visit_path(&mut self, node: &'ast syn::Path) {
                            for seg in &node.segments {
                                if seg.ident == self.old {
                                    self.opaque.push(seg.ident.span().start().line as u32);
                                }
                            }
                            syn::visit::visit_path(self, node);
                        }
                        fn visit_macro(&mut self, node: &'ast syn::Macro) {
                            for line in attr_ident_lines(&node.tokens, self.old) {
                                self.opaque.push(line);
                            }
                        }
                    }
                    let mut values = ValuePaths {
                        old: self.old,
                        opaque: Vec::new(),
                    };
                    values.visit_expr(&nv.value);
                    self.opaque_lines.extend(values.opaque);
                }
                syn::Meta::List(list) => {
                    self.opaque_lines
                        .extend(attr_ident_lines(&list.tokens, self.old));
                }
                syn::Meta::Path(_) => {}
            }
            // Never descended into (opaque).
        }
    }

    let mut interior = Interior {
        original,
        index,
        old,
        scopes,
        rewritten,
        uses: Vec::new(),
        inner_defs: Vec::new(),
        opaque_lines: Vec::new(),
    };
    interior.visit_file(parsed);

    let mut lines: Vec<(u32, &'static str)> = Vec::new();
    for (sp, a, _b) in &interior.uses {
        let line = sp.start().line as u32;
        let reason = if interior.inner_defs.iter().any(|(d, _)| *d < *a) {
            "shadowed"
        } else {
            "off-target"
        };
        lines.push((line, reason));
    }
    for line in interior.opaque_lines {
        lines.push((line, "macro-hidden"));
    }
    lines.sort_unstable();
    lines.dedup();
    let mut out: Vec<crate::OrphanReport> = Vec::new();
    for (line, reason) in lines {
        out.push(crate::OrphanReport {
            file: path.to_string(),
            line,
            text: orphan_line_text(original, index, line),
            reason: reason.to_string(),
        });
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}

/// WI-2d dispatcher: detect a `for` / `if let` / `while let` binder
/// rename from the compiled pattern and scan its scope interior.
/// Returns empty for every other shape (never alters verdicts).
pub(crate) fn wi2d_orphans_for_compiled(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Vec<crate::OrphanReport> {
    if let Some((old, _)) = crate::use_graph_binder::for_loop_binder_rename(&compiled.kind) {
        return wi2d_expr_orphans(
            states,
            file_edits,
            path,
            edits_before,
            &old,
            Wi2dExprKind::For,
        );
    }
    if let Some((old, _)) = crate::use_graph_binder::if_let_binder_rename(&compiled.kind) {
        return wi2d_expr_orphans(
            states,
            file_edits,
            path,
            edits_before,
            &old,
            Wi2dExprKind::IfLet,
        );
    }
    if let Some((old, _)) = crate::use_graph_binder::while_let_binder_rename(&compiled.kind) {
        return wi2d_expr_orphans(
            states,
            file_edits,
            path,
            edits_before,
            &old,
            Wi2dExprKind::WhileLet,
        );
    }
    Vec::new()
}

/// WI-2d expression-binder kind selector (for / `if let` / `while let`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wi2dExprKind {
    For,
    IfLet,
    WhileLet,
}

/// WI-2d: `for` / `if let` / `while let` binder renames. Same duty
/// as [`closure_or_arm_orphans`]: only the rewritten node's interior
/// that can dangle is scanned — the loop body (`for`, `while let`) or
/// the then-branch (`if let`). Iter/scrutinee/`else`/outer uses can
/// never dangle and are never orphans.
pub(crate) fn wi2d_expr_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    old: &str,
    kind: Wi2dExprKind,
) -> Vec<crate::OrphanReport> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    let Some(st) = states.get(path) else {
        return Vec::new();
    };
    let Some(parsed) = st.parsed.as_ref() else {
        return Vec::new();
    };
    let original = st.original.as_str();
    let index = &st.index;
    let spans: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    if spans.is_empty() {
        return Vec::new();
    }

    struct ScopeGrab<'a> {
        original: &'a str,
        index: &'a LineIndex,
        old: &'a str,
        spans: &'a [(usize, usize)],
        kind: Wi2dExprKind,
        kept: Vec<(usize, usize)>,
        scopes: NodeScopes,
    }
    impl ScopeGrab<'_> {
        // WI-3 reversed containment (see the closure/arm grab above).
        fn contained(&self, a: usize, b: usize) -> bool {
            self.spans.iter().any(|(s, e)| a <= *s && *e <= b)
        }
        fn binders_of(&self, pat: &syn::Pat) -> Vec<(usize, usize)> {
            let mut out = Vec::new();
            for off in pat_old_offsets(pat, self.old, self.original, self.index) {
                let end = off + self.old.len();
                if self.original.get(off..end) == Some(self.old) {
                    out.push((off, end));
                }
            }
            out
        }
    }
    impl<'ast> Visit<'ast> for ScopeGrab<'_> {
        fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
            if self.kind == Wi2dExprKind::For {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contained(ab.0, ab.1)
                        && !self.kept.iter().any(|(s, e)| *s <= ab.0 && ab.1 <= *e)
                    {
                        let binders = self.binders_of(&node.pat);
                        if !binders.is_empty() {
                            if let Some(body) = crate::matcher_item::span_bytes(
                                self.original,
                                self.index,
                                node.body.span(),
                            ) {
                                self.kept.push(ab);
                                self.scopes.push((vec![body], binders));
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr_for_loop(self, node);
        }
        fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
            if self.kind == Wi2dExprKind::IfLet {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contained(ab.0, ab.1)
                        && !self.kept.iter().any(|(s, e)| *s <= ab.0 && ab.1 <= *e)
                    {
                        if let syn::Expr::Let(let_) = &*node.cond {
                            let binders = self.binders_of(&let_.pat);
                            if !binders.is_empty() {
                                if let Some(then) = crate::matcher_item::span_bytes(
                                    self.original,
                                    self.index,
                                    node.then_branch.span(),
                                ) {
                                    self.kept.push(ab);
                                    self.scopes.push((vec![then], binders));
                                }
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr_if(self, node);
        }
        fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
            if self.kind == Wi2dExprKind::WhileLet {
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self.contained(ab.0, ab.1)
                        && !self.kept.iter().any(|(s, e)| *s <= ab.0 && ab.1 <= *e)
                    {
                        if let syn::Expr::Let(let_) = &*node.cond {
                            let binders = self.binders_of(&let_.pat);
                            if !binders.is_empty() {
                                if let Some(body) = crate::matcher_item::span_bytes(
                                    self.original,
                                    self.index,
                                    node.body.span(),
                                ) {
                                    self.kept.push(ab);
                                    self.scopes.push((vec![body], binders));
                                }
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr_while(self, node);
        }
    }
    let mut grab = ScopeGrab {
        original,
        index,
        old,
        spans: &spans,
        kind,
        kept: Vec::new(),
        scopes: Vec::new(),
    };
    grab.visit_file(parsed);
    let scopes = grab.scopes;
    if scopes.is_empty() {
        return Vec::new();
    }
    interior_reports(
        parsed,
        original,
        index,
        path,
        &scopes,
        old,
        &follow_spans(file_edits, path, edits_before),
    )
}

/// WI-2d: scope-interior orphans for fn-param renames. The scope is the
/// matched `fn`'s body block; the renamed param ident in the signature
/// is excluded. Only the hunk's own file is scanned. Unlike the
/// whole-node Expr hunks, a fn header edit is *inside* its item, so the
/// matched fn is the one *containing* the hunk's edit spans (reversed
/// containment vs [`closure_or_arm_orphans`]).
pub(crate) fn fn_param_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    old: &str,
) -> Vec<crate::OrphanReport> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    let Some(st) = states.get(path) else {
        return Vec::new();
    };
    let Some(parsed) = st.parsed.as_ref() else {
        return Vec::new();
    };
    let original = st.original.as_str();
    let index = &st.index;
    let spans: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    if spans.is_empty() {
        return Vec::new();
    }

    struct FnGrab<'a> {
        original: &'a str,
        index: &'a LineIndex,
        old: &'a str,
        spans: &'a [(usize, usize)],
        scopes: NodeScopes,
    }
    impl<'ast> Visit<'ast> for FnGrab<'_> {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            if let Some(ab) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                // Header edit lives inside the fn; every fn containing an
                // edit claims it (a nested fn names its own param only
                // when its signature also binds `old`, which the shadow
                // discipline then labels).
                let contains = self.spans.iter().any(|(s, e)| ab.0 <= *s && *e <= ab.1);
                if contains {
                    let mut binders: Vec<(usize, usize)> = Vec::new();
                    for arg in &node.sig.inputs {
                        if let syn::FnArg::Typed(t) = arg {
                            for off in pat_old_offsets(&t.pat, self.old, self.original, self.index)
                            {
                                let end = off + self.old.len();
                                if self.original.get(off..end) == Some(self.old) {
                                    binders.push((off, end));
                                }
                            }
                        }
                    }
                    if !binders.is_empty() {
                        if let Some(body) = crate::matcher_item::span_bytes(
                            self.original,
                            self.index,
                            node.block.span(),
                        ) {
                            self.scopes.push((vec![body], binders));
                        }
                    }
                }
            }
            syn::visit::visit_item_fn(self, node);
        }
    }
    let mut grab = FnGrab {
        original,
        index,
        old,
        spans: &spans,
        scopes: Vec::new(),
    };
    grab.visit_file(parsed);
    let scopes = grab.scopes;
    if scopes.is_empty() {
        return Vec::new();
    }
    interior_reports(
        parsed,
        original,
        index,
        path,
        &scopes,
        old,
        &follow_spans(file_edits, path, edits_before),
    )
}

/// WI-3: the hunk's `follow`-labelled spans (completion evidence the
/// interior scan must not report back as orphans).
fn follow_spans(
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
) -> Vec<(usize, usize)> {
    file_edits
        .get(path)
        .map(|v| {
            v[edits_before.min(v.len())..]
                .iter()
                .filter(|e| e.label == Some("follow"))
                .map(|e| (e.start, e.end))
                .collect()
        })
        .unwrap_or_default()
}

/// Opaque lines for macro interiors naming `old`: one entry per ident
/// or inline `{old}` capture line (WI-2b ident-line parity — not the
/// macro start line). Single-line macros are unchanged; multi-line
/// macros attribute to the line holding the ident/literal.
fn macro_opaque_lines(tokens: &proc_macro2::TokenStream, old: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for t in tokens.clone() {
        match t {
            proc_macro2::TokenTree::Group(g) => {
                out.extend(macro_opaque_lines(&g.stream(), old));
            }
            proc_macro2::TokenTree::Ident(id) => {
                if id == old {
                    out.push(id.span().start().line as u32);
                }
            }
            proc_macro2::TokenTree::Literal(lit) if literal_captures(&lit.to_string(), old) => {
                out.push(lit.span().start().line as u32);
            }
            _ => {}
        }
    }
    out
}

/// Ident-only token scan for attribute interiors (`Meta::List` tokens
/// and macros inside `Meta::NameValue` values): one entry per matching
/// ident line. String literals stay silent by construction (`Literal`,
/// never `Ident`) — unlike macros, attributes never host inline
/// `{old}` captures, so the literal arm is intentionally absent.
fn attr_ident_lines(tokens: &proc_macro2::TokenStream, old: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for t in tokens.clone() {
        match t {
            proc_macro2::TokenTree::Group(g) => {
                out.extend(attr_ident_lines(&g.stream(), old));
            }
            proc_macro2::TokenTree::Ident(id) if id == old => {
                out.push(id.span().start().line as u32);
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn states_for(src: &str) -> (HashMap<String, FileState>, LineIndex, syn::File) {
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = LineIndex::new(src);
        let mut states = HashMap::new();
        states.insert(
            "src/main.rs".to_string(),
            FileState {
                original: src.to_string(),
                index: index.clone(),
                parsed: Some(parsed.clone()),
            },
        );
        (states, index, parsed)
    }

    fn edits_for_node(
        src: &str,
        index: &LineIndex,
        span: proc_macro2::Span,
    ) -> HashMap<String, Vec<TextEdit>> {
        let (a, b) = crate::matcher_item::span_bytes(src, index, span).unwrap();
        let mut m = HashMap::new();
        m.insert(
            "src/main.rs".to_string(),
            vec![TextEdit::new(a, b, "X".to_string())],
        );
        m
    }

    fn first_for_span(src: &str) -> proc_macro2::Span {
        struct Find(Vec<proc_macro2::Span>);
        impl<'ast> syn::visit::Visit<'ast> for Find {
            fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
                use syn::spanned::Spanned;
                self.0.push(node.span());
                syn::visit::visit_expr_for_loop(self, node);
            }
        }
        use syn::visit::Visit;
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let mut f = Find(Vec::new());
        f.visit_file(&parsed);
        f.0.into_iter().next().expect("test src has a for loop")
    }

    fn first_if_span(src: &str) -> proc_macro2::Span {
        struct Find(Vec<proc_macro2::Span>);
        impl<'ast> syn::visit::Visit<'ast> for Find {
            fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
                use syn::spanned::Spanned;
                self.0.push(node.span());
                syn::visit::visit_expr_if(self, node);
            }
        }
        use syn::visit::Visit;
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let mut f = Find(Vec::new());
        f.visit_file(&parsed);
        f.0.into_iter().next().expect("test src has an if")
    }

    fn first_while_span(src: &str) -> proc_macro2::Span {
        struct Find(Vec<proc_macro2::Span>);
        impl<'ast> syn::visit::Visit<'ast> for Find {
            fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
                use syn::spanned::Spanned;
                self.0.push(node.span());
                syn::visit::visit_expr_while(self, node);
            }
        }
        use syn::visit::Visit;
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let mut f = Find(Vec::new());
        f.visit_file(&parsed);
        f.0.into_iter().next().expect("test src has a while")
    }

    fn expr_orphans(src: &str, kind: Wi2dExprKind) -> Vec<crate::OrphanReport> {
        let (states, index, _) = states_for(src);
        let span = match kind {
            Wi2dExprKind::For => first_for_span(src),
            Wi2dExprKind::IfLet => first_if_span(src),
            Wi2dExprKind::WhileLet => first_while_span(src),
        };
        let edits = edits_for_node(src, &index, span);
        wi2d_expr_orphans(&states, &edits, "src/main.rs", 0, "total", kind)
    }

    fn fn_orphans(src: &str) -> Vec<crate::OrphanReport> {
        let (states, index, _) = states_for(src);
        // Header edit: the `total` param ident bytes (inside the fn).
        let off = src.find("total").expect("test src names total");
        let mut edits = HashMap::new();
        edits.insert(
            "src/main.rs".to_string(),
            vec![TextEdit::new(off, off + "total".len(), "sum".to_string())],
        );
        let _ = index;
        fn_param_orphans(&states, &edits, "src/main.rs", 0, "total")
    }

    #[test]
    fn for_body_use_is_orphan() {
        let src =
            "fn main() {\n    for total in 0..3 {\n        let x = total + total;\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].line, 3);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn for_empty_body_is_clean() {
        let src = "fn main() {\n    for total in 0..3 {\n        break;\n    }\n}\n";
        assert!(expr_orphans(src, Wi2dExprKind::For).is_empty());
    }

    #[test]
    fn for_iter_use_not_orphan() {
        // `total` in the iterator names the outer scope, not the loop
        // binder — body `break` uses nothing, so no orphans.
        let src =
            "fn main() {\n    let total = vec![1];\n    for total in total {\n        break;\n    }\n}\n";
        assert!(expr_orphans(src, Wi2dExprKind::For).is_empty());
    }

    #[test]
    fn for_inner_let_shadows() {
        let src = "fn main() {\n    for total in 0..3 {\n        let total = 1;\n        let y = total;\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].reason, "shadowed");
    }

    #[test]
    fn for_macro_hidden() {
        let src =
            "fn main() {\n    for total in 0..3 {\n        println!(\"{}\", total);\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].reason, "macro-hidden");
    }

    #[test]
    fn for_multiline_macro_reports_ident_line() {
        // WI-2b ident-line parity: the macro starts line 3 but `total`
        // sits on line 4 — the orphan reports line 4.
        let src = "fn main() {\n    for total in 0..3 {\n        println!(\n            \"{}\", total\n        );\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].reason, "macro-hidden");
        assert_eq!(orphans[0].line, 4);
    }

    #[test]
    fn for_multiline_macro_literal_capture_reports_literal_line() {
        // Inline `{total}` capture inside a multi-line macro: the
        // literal sits on line 4, not the macro start line 3.
        let src = "fn main() {\n    for total in 0..3 {\n        println!(\n            \"{total}\"\n        );\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].reason, "macro-hidden");
        assert_eq!(orphans[0].line, 4);
    }

    #[test]
    fn for_multiline_qself_reports_ident_line() {
        // WI-2b ident-line parity: the `<...>` starts line 3 but `total`
        // sits on line 4 — the orphan reports line 4.
        let src = "fn main() {\n    for total in 0..3 {\n        let x: <T as Tr>::a::\n            total = make();\n        let _ = x;\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "macro-hidden");
        assert_eq!(orphans[0].line, 4);
    }

    #[test]
    fn for_attribute_path_is_macro_hidden() {
        // `#[total]` never resolves to the binder: opaque like a macro
        // interior (let-path + item parity), never an `off-target` use.
        let src =
            "fn main() {\n    for total in 0..3 {\n        #[total]\n        let x = 1;\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "macro-hidden");
        assert_eq!(orphans[0].line, 3, "got: {orphans:?}");
    }

    #[test]
    fn for_attribute_value_is_macro_hidden() {
        // `#[doc = total]` values never resolve either (same law as the
        // `#[total]` path twin).
        let src =
            "fn main() {\n    for total in 0..3 {\n        #[doc = total]\n        let x = 1;\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "macro-hidden");
        assert_eq!(orphans[0].line, 3, "got: {orphans:?}");
    }

    #[test]
    fn for_attribute_list_is_macro_hidden() {
        // List-token idents (`#[foo(total)]`) are token-stream idents,
        // never parsed paths — equally opaque (this form was silent
        // `verified` before the Interior unification).
        let src =
            "fn main() {\n    for total in 0..3 {\n        #[foo(total)]\n        let x = 1;\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::For);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "macro-hidden");
        assert_eq!(orphans[0].line, 3, "got: {orphans:?}");
    }

    #[test]
    fn for_attribute_string_literal_is_silent() {
        // `#[doc = "total"]` is a `Literal`, never an `Ident`: prose
        // mentions of the name stay silent (control).
        let src =
            "fn main() {\n    for total in 0..3 {\n        #[doc = \"total\"]\n        let x = 1;\n    }\n}\n";
        assert!(
            expr_orphans(src, Wi2dExprKind::For).is_empty(),
            "got: {:?}",
            expr_orphans(src, Wi2dExprKind::For)
        );
    }

    #[test]
    fn for_attribute_outside_scope_is_silent() {
        // An attribute on the `for` node itself sits outside the loop-body
        // scope: the binder is not visible there, so nothing dangles.
        let src =
            "fn main() {\n    #[doc = total]\n    for total in 0..3 {\n        break;\n    }\n}\n";
        assert!(
            expr_orphans(src, Wi2dExprKind::For).is_empty(),
            "got: {:?}",
            expr_orphans(src, Wi2dExprKind::For)
        );
    }

    #[test]
    fn iflet_then_use_is_orphan() {
        let src =
            "fn main() {\n    if let total = Some(1) {\n        let x = total + 1;\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::IfLet);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn iflet_else_use_not_orphan() {
        // The binder is not visible in `else`.
        let src = "fn main() {\n    if let total = Some(1) {\n        let x = 1;\n    } else {\n        let y = total;\n    }\n}\n";
        assert!(expr_orphans(src, Wi2dExprKind::IfLet).is_empty());
    }

    #[test]
    fn whilelet_body_use_is_orphan() {
        let src = "fn main() {\n    while let total = Some(1) {\n        let x = total;\n        break;\n    }\n}\n";
        let orphans = expr_orphans(src, Wi2dExprKind::WhileLet);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn fn_param_body_use_is_orphan() {
        let src = "fn foo(total: u32) -> u32 {\n    total + 1\n}\n";
        let orphans = fn_orphans(src);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].line, 2);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn fn_param_outside_use_not_orphan() {
        // A same-name `total` in another fn is a different binding.
        let src = "fn foo(total: u32) -> u32 {\n    1\n}\nfn bar() {\n    let total = 1;\n    let _ = total;\n}\n";
        assert!(fn_orphans(src).is_empty());
    }

    fn apply_report(
        patch_text: &str,
        file_text: &str,
    ) -> Result<(String, crate::ApplyReport), String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        let report = crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok((files["src/lib.rs"].clone(), report))
    }

    const HEAD: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";

    #[test]
    fn for_rename_completes_body_uses() {
        // WI-3: `for`-binder renames complete over the loop body.
        let patch = format!(
            "{HEAD}@@ -2,3 @@ rule=rename_for\n# lang=rust\n# expr E\n- for total in E {{ ... }}\n+ for sum in E {{ ... }}\n"
        );
        let src =
            "fn main() {\n    for total in 0..3 {\n        let x = total + total;\n    }\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("for sum in"), "got: {out}");
        assert!(out.contains("let x = sum + sum;"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(report.rules[0].orphans.is_empty());
    }

    #[test]
    fn iflet_rename_completes_then_uses() {
        // WI-3: `if let`-binder renames complete over the then-branch.
        let patch = format!(
            "{HEAD}@@ -2,3 @@ rule=rename_iflet\n# lang=rust\n# expr E\n- if let total = E {{ ... }}\n+ if let sum = E {{ ... }}\n"
        );
        let src =
            "fn main() {\n    if let total = Some(1) {\n        let x = total + 1;\n    }\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("if let sum ="), "got: {out}");
        assert!(out.contains("let x = sum + 1;"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(report.rules[0].orphans.is_empty());
    }

    #[test]
    fn whilelet_rename_completes_body_uses() {
        // WI-3: `while let`-binder renames complete over the loop body.
        let patch = format!(
            "{HEAD}@@ -2,4 @@ rule=rename_whilelet\n# lang=rust\n# expr E\n- while let total = E {{ ... }}\n+ while let sum = E {{ ... }}\n"
        );
        let src =
            "fn main() {\n    while let total = Some(1) {\n        let x = total;\n        break;\n    }\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("while let sum ="), "got: {out}");
        assert!(out.contains("let x = sum;"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(report.rules[0].orphans.is_empty());
    }

    #[test]
    fn fn_param_rename_completes_body_uses() {
        // WI-3: fn-param renames complete over the fn body.
        let patch = format!(
            "{HEAD}@@ -1,3 @@ rule=rename_param\n- fn foo(total: u32) {{ ... }}\n+ fn foo(sum: u32) {{ ... }}\n"
        );
        let src = "fn foo(total: u32) -> u32 {\n    total + 1\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("fn foo(sum: u32)"), "got: {out}");
        assert!(out.contains("sum + 1"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(report.rules[0].orphans.is_empty());
    }

    #[test]
    fn fn_param_rename_shadowed_use_stays_orphan() {
        let patch = format!(
            "{HEAD}@@ -1,4 @@ rule=rename_param\n- fn foo(total: u32) {{ ... }}\n+ fn foo(sum: u32) {{ ... }}\n"
        );
        let src = "fn foo(total: u32) -> u32 {\n    let total = 1;\n    total + total\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("fn foo(sum: u32)"), "got: {out}");
        assert_eq!(report.rules[0].semantics, crate::RuleSemantics::ReparseOnly);
        assert!(!report.rules[0].orphans.is_empty());
        assert!(
            report.rules[0]
                .orphans
                .iter()
                .any(|o| o.reason == "shadowed"),
            "got: {:?}",
            report.rules[0].orphans
        );
    }

    #[test]
    fn fn_param_rename_clean_stays_instantiated() {
        let patch = format!(
            "{HEAD}@@ -1,3 @@ rule=rename_param\n- fn foo(total: u32) {{ ... }}\n+ fn foo(sum: u32) {{ ... }}\n"
        );
        let src = "fn foo(total: u32) -> u32 {\n    1\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("fn foo(sum: u32)"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(report.rules[0].orphans.is_empty());
    }
}
