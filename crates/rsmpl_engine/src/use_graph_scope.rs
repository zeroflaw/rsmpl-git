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
        fn contained(&self, a: usize, b: usize) -> bool {
            self.spans.iter().any(|(s, e)| *s <= a && b <= *e)
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

    struct Interior<'a> {
        original: &'a str,
        index: &'a LineIndex,
        old: &'a str,
        scopes: &'a NodeScopes,
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
                        if self.original.get(a..b) == Some(self.old)
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
            if node.qself.is_some() && node.path.segments.iter().any(|s| s.ident == self.old) {
                let line = node.span().start().line as u32;
                if let Some((a, b)) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self
                        .scopes
                        .iter()
                        .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e))
                    {
                        self.opaque_lines.push(line);
                    }
                }
                return;
            }
            syn::visit::visit_type_path(self, node);
        }
        fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
            if node.qself.is_some() && node.path.segments.iter().any(|s| s.ident == self.old) {
                let line = node.span().start().line as u32;
                if let Some((a, b)) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    if self
                        .scopes
                        .iter()
                        .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e))
                    {
                        self.opaque_lines.push(line);
                    }
                }
                return;
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
            let line = node.span().start().line as u32;
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                if self
                    .scopes
                    .iter()
                    .any(|(rs, _)| rs.iter().any(|(s, e)| *s <= a && b <= *e))
                    && macro_mentions_old(&node.tokens, self.old)
                {
                    self.opaque_lines.push(line);
                }
            }
            // Macro interiors stay opaque (never descended into).
        }
    }

    let mut interior = Interior {
        original,
        index,
        old,
        scopes: &scopes,
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

/// True when macro tokens mention `old` as an ident or an inline
/// `{old}` format capture (the WI-2a capture rule: a trailing ident
/// continue disqualifies the capture).
fn macro_mentions_old(tokens: &proc_macro2::TokenStream, old: &str) -> bool {
    for t in tokens.clone() {
        match t {
            proc_macro2::TokenTree::Group(g) => {
                if macro_mentions_old(&g.stream(), old) {
                    return true;
                }
            }
            proc_macro2::TokenTree::Ident(id) => {
                if id == old {
                    return true;
                }
            }
            proc_macro2::TokenTree::Literal(lit) if literal_captures(&lit.to_string(), old) => {
                return true;
            }
            _ => {}
        }
    }
    false
}
