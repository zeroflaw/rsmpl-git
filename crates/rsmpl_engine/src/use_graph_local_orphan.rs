//! WI-2b orphan precision: enclosing-block gate for `let`-binder orphans.
//!
//! Split from `use_graph_local` (D5 hygiene: that file grows toward the
//! 2000-line gate; behaviour-preserving move, no semantic change).
//! `general_orphans` keeps dispatching; this module owns the
//! `let`-binder scan only.

use std::collections::HashMap;

use crate::edits::TextEdit;
use crate::lines::{FileState, LineIndex};
use crate::use_graph_local::{format_capture_lines, orphan_line_text};
use crate::use_graph_orphan::collect_orphans;

/// WI-2a post-binder orphan scan for `let`-binder renames (same file
/// only: locals can never dangle cross-file; uses before the binder
/// statement end name the outer scope, including the binder's own init
/// RHS `let total = total;`).
///
/// WI-2b precision: the scan is additionally gated to the smallest
/// `Block` containing the binder (the same enclosing scope follows
/// rewrite in). Same-name uses in other fns/other blocks name a
/// different scope and are not orphans.
pub(crate) fn let_binder_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    old: &str,
) -> Vec<crate::OrphanReport> {
    let Some(st) = states.get(path) else {
        return Vec::new();
    };
    let Some(parsed) = st.parsed.as_ref() else {
        return Vec::new();
    };
    let original = st.original.as_str();
    let index = &st.index;
    let hunk_edits: &[TextEdit] = file_edits
        .get(path)
        .map(|v| &v[edits_before.min(v.len())..])
        .unwrap_or(&[]);
    if hunk_edits.is_empty() {
        return Vec::new();
    }
    let renamed: Vec<(usize, usize)> = hunk_edits.iter().map(|e| (e.start, e.end)).collect();
    let binder_spans: Vec<(usize, usize)> = hunk_edits
        .iter()
        .filter(|e| e.label != Some("follow"))
        .map(|e| (e.start, e.end))
        .collect();
    let fallback = binder_spans
        .iter()
        .map(|(s, _)| *s)
        .min()
        .unwrap_or(usize::MAX);
    struct LocalEnds<'a> {
        original: &'a str,
        index: &'a LineIndex,
        binders: &'a [(usize, usize)],
        ends: Vec<usize>,
    }
    impl<'ast> syn::visit::Visit<'ast> for LocalEnds<'_> {
        fn visit_local(&mut self, node: &'ast syn::Local) {
            use syn::spanned::Spanned;
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                if self.binders.iter().any(|(s, e)| a <= *s && *e <= b) {
                    self.ends.push(b);
                }
            }
            syn::visit::visit_local(self, node);
        }
    }
    let mut grab = LocalEnds {
        original,
        index,
        binders: &binder_spans,
        ends: Vec::new(),
    };
    {
        use syn::visit::Visit;
        grab.visit_file(parsed);
    }
    let thresh = grab.ends.into_iter().min().unwrap_or(fallback);
    if thresh == usize::MAX {
        return Vec::new();
    }
    let thresh_line = index.line_of(thresh);
    // Enclosing-block gate (mirrors `let_binder_follow_edits`): smallest
    // block containing the binder. `None` keeps the legacy post-binder
    // behaviour (safe downgrade, never a missed orphan).
    struct ScopeGate<'a> {
        original: &'a str,
        index: &'a LineIndex,
        blocks: Vec<(usize, usize)>,
        item_fns: Vec<(usize, usize)>,
        macros: Vec<(usize, usize)>,
    }
    impl<'ast> syn::visit::Visit<'ast> for ScopeGate<'_> {
        fn visit_block(&mut self, node: &'ast syn::Block) {
            use syn::spanned::Spanned;
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                self.blocks.push((a, b));
            }
            syn::visit::visit_block(self, node);
        }
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            use syn::spanned::Spanned;
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                self.item_fns.push((a, b));
            }
            syn::visit::visit_item_fn(self, node);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            use syn::spanned::Spanned;
            if let Some((a, b)) =
                crate::matcher_item::span_bytes(self.original, self.index, node.span())
            {
                self.macros.push((a, b));
            }
        }
    }
    let mut gate = ScopeGate {
        original,
        index,
        blocks: Vec::new(),
        item_fns: Vec::new(),
        macros: Vec::new(),
    };
    {
        use syn::visit::Visit;
        gate.visit_file(parsed);
    }
    let (binder_start, binder_end) = binder_spans
        .iter()
        .fold((usize::MAX, 0usize), |(ms, me), (s, e)| {
            (ms.min(*s), me.max(*e))
        });
    let mut enclosing: Option<(usize, usize)> = None;
    if binder_start != usize::MAX {
        for b in &gate.blocks {
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
    }
    let nested_fns: Vec<(usize, usize)> = match enclosing {
        Some(enc) => gate
            .item_fns
            .iter()
            .copied()
            .filter(|r| enc.0 <= r.0 && r.1 <= enc.1)
            .collect(),
        None => Vec::new(),
    };
    let in_nested = |off: usize| nested_fns.iter().any(|r| r.0 <= off && off < r.1);
    let in_macro = |off: usize| gate.macros.iter().any(|r| r.0 <= off && off < r.1);
    let in_enc = |off: usize| match enclosing {
        Some(enc) => enc.0 <= off && off < enc.1,
        None => true,
    };
    // Byte start of an orphan line; `None` means unmappable (keep the
    // orphan — safe downgrade direction). Macro lines additionally
    // exclude nested-`fn` interiors (a capture inside `fn inner`
    // names the inner scope, never the outer binder).
    let line_in_scope = |line: u32| -> bool {
        match enclosing {
            None => true,
            Some(enc) => match index.line_start(line) {
                Some(ls) => enc.0 <= ls && ls < enc.1 && !in_nested(ls),
                None => true,
            },
        }
    };
    // True when `line` holds at least one post-binder `old` ident inside
    // the enclosing block, outside renamed/nested-fn/macro spans.
    let line_has_claimable = |line: u32| -> bool {
        let Some(ls) = index.line_start(line) else {
            return true;
        };
        let le = index.line_start(line + 1).unwrap_or(original.len());
        let Some(line_text) = original.get(ls..le.min(original.len())) else {
            return true;
        };
        let mut off = 0;
        while let Some(pos) = line_text[off..].find(old) {
            let a = ls + off + pos;
            let b = a + old.len();
            let prev_ok = a == 0 || {
                let c = original[..a].chars().next_back().unwrap();
                !(c.is_alphanumeric() || c == '_')
            };
            let next_ok = original
                .get(b..)
                .map(|r| {
                    r.chars()
                        .next()
                        .map(|c| !(c.is_alphanumeric() || c == '_'))
                        .unwrap_or(true)
                })
                .unwrap_or(true);
            if prev_ok
                && next_ok
                && a >= thresh
                && in_enc(a)
                && !renamed.iter().any(|(s, e)| a < *e && *s < b)
                && !in_nested(a)
                && !in_macro(a)
            {
                return true;
            }
            off += pos + 1;
            if off >= line_text.len() {
                break;
            }
        }
        false
    };
    let refs = vec![(path.to_string(), parsed, original, index)];
    let mut out = collect_orphans(&refs, old, path, &renamed, false);
    // Inline format-string captures (`println!("{total}")`) hide the
    // ident inside a string literal — no `Ident` token for the path
    // collector above. Pre-binder macros never dangle, so only
    // post-binder capture lines inside the enclosing block are kept
    // (safe direction: downgrade).
    for line in format_capture_lines(parsed, old) {
        if line < thresh_line {
            continue;
        }
        if !line_in_scope(line) {
            continue;
        }
        out.push(crate::OrphanReport {
            file: path.to_string(),
            line,
            text: orphan_line_text(original, index, line),
            reason: "macro-hidden".to_string(),
        });
    }
    out.retain(|o| {
        if o.line < thresh_line {
            return false;
        }
        if o.reason == "macro-hidden" {
            return line_in_scope(o.line);
        }
        // `off-target`: keep only when a post-binder in-scope ident
        // remains on the line. This subsumes the legacy
        // `line > thresh_line → keep` rule while dropping other-fn /
        // other-block / nested-fn uses that merely share the name.
        line_has_claimable(o.line)
    });
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}
