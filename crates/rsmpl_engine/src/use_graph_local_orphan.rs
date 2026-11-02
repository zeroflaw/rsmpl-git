//! WI-2b orphan precision: enclosing-block gate for `let`-binder orphans.
//!
//! Split from `use_graph_local` (D5 hygiene: that file grows toward the
//! 2000-line gate; behaviour-preserving move, no semantic change).
//! `general_orphans` keeps dispatching; this module owns the
//! `let`-binder scan only.

use std::collections::HashMap;

use crate::edits::TextEdit;
use crate::lines::{FileState, LineIndex};
use crate::use_graph_local::{format_capture_spans, orphan_line_text};
use crate::use_graph_orphan::collect_orphans;

/// WI-13: modelled shadow ranges for same-name binders outside the
/// `let` / closure-param family: `(binder_def, construct_scope_end)`
/// for `for` / `if let` / `while let` patterns, match-arm patterns, and
/// fn-param patterns (raw and unfiltered — callers apply their scope
/// filters). Single source for the orphan scan below and the
/// `let`-binder follow walk: both sides must agree on which uses name
/// an inner binder, or completion and evidence disagree and an
/// unshadowed outer use strands as `off-target`.
pub(crate) fn extra_binder_shadow_ranges(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    old: &str,
) -> Vec<(usize, usize)> {
    use syn::visit::Visit;
    struct Grab<'a> {
        original: &'a str,
        index: &'a LineIndex,
        old: &'a str,
        out: Vec<(usize, usize)>,
    }
    impl<'ast> Visit<'ast> for Grab<'_> {
        fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
            use syn::spanned::Spanned;
            let defs = crate::use_graph_local::pat_old_offsets(
                &node.pat,
                self.old,
                self.original,
                self.index,
            );
            if !defs.is_empty() {
                if let Some((_, bend)) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.body.span())
                {
                    for d in defs {
                        self.out.push((d, bend));
                    }
                }
            }
            syn::visit::visit_expr_for_loop(self, node);
        }
        fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
            use syn::spanned::Spanned;
            if let syn::Expr::Let(let_) = &*node.cond {
                let defs = crate::use_graph_local::pat_old_offsets(
                    &let_.pat,
                    self.old,
                    self.original,
                    self.index,
                );
                if !defs.is_empty() {
                    if let Some((_, tend)) = crate::matcher_item::span_bytes(
                        self.original,
                        self.index,
                        node.then_branch.span(),
                    ) {
                        for d in defs {
                            self.out.push((d, tend));
                        }
                    }
                }
            }
            syn::visit::visit_expr_if(self, node);
        }
        fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
            use syn::spanned::Spanned;
            if let syn::Expr::Let(let_) = &*node.cond {
                let defs = crate::use_graph_local::pat_old_offsets(
                    &let_.pat,
                    self.old,
                    self.original,
                    self.index,
                );
                if !defs.is_empty() {
                    if let Some((_, bend)) =
                        crate::matcher_item::span_bytes(self.original, self.index, node.body.span())
                    {
                        for d in defs {
                            self.out.push((d, bend));
                        }
                    }
                }
            }
            syn::visit::visit_expr_while(self, node);
        }
        fn visit_arm(&mut self, node: &'ast syn::Arm) {
            use syn::spanned::Spanned;
            let defs = crate::use_graph_local::pat_old_offsets(
                &node.pat,
                self.old,
                self.original,
                self.index,
            );
            if !defs.is_empty() {
                let mut end: Option<usize> = None;
                if let Some((_, be)) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.body.span())
                {
                    end = Some(be);
                }
                if let Some((_, guard)) = &node.guard {
                    if let Some((_, ge)) =
                        crate::matcher_item::span_bytes(self.original, self.index, guard.span())
                    {
                        end = Some(end.map_or(ge, |e| e.max(ge)));
                    }
                }
                if let Some(e) = end {
                    for d in defs {
                        self.out.push((d, e));
                    }
                }
            }
            syn::visit::visit_arm(self, node);
        }
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            use syn::spanned::Spanned;
            let mut param_defs = Vec::new();
            for arg in &node.sig.inputs {
                if let syn::FnArg::Typed(t) = arg {
                    param_defs.extend(crate::use_graph_local::pat_old_offsets(
                        &t.pat,
                        self.old,
                        self.original,
                        self.index,
                    ));
                }
            }
            if !param_defs.is_empty() {
                if let Some((_, bend)) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.block.span())
                {
                    for d in param_defs {
                        self.out.push((d, bend));
                    }
                }
            }
            syn::visit::visit_item_fn(self, node);
        }
    }
    let mut grab = Grab {
        original,
        index,
        old,
        out: Vec::new(),
    };
    grab.visit_file(parsed);
    grab.out.sort_unstable();
    grab.out.dedup();
    grab.out
}

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
        old: &'a str,
        blocks: Vec<(usize, usize)>,
        item_fns: Vec<(usize, usize)>,
        macros: Vec<(usize, usize)>,
        closures: Vec<(usize, usize)>,
        let_defs: Vec<usize>,
        closure_params: Vec<usize>,
        all_pats: Vec<usize>,
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
        fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
            use syn::spanned::Spanned;
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
    let mut gate = ScopeGate {
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
    // Shadow ranges (mirrors `let_binder_follow_edits`): an inner
    // `let old` governs to its innermost block end, a closure-param
    // `old` to its closure end, and unmodelled binders (`for` / `if
    // let` / `while let` / match-arm / fn-param) to their construct
    // scope (loop body, then-branch, guard+body, fn body). Uses inside
    // these ranges name the inner binder, so leftover orphans there
    // are `shadowed`.
    let within_enc = |r: (usize, usize), off: usize| r.0 <= off && off < r.1;
    let mut inner_lets: Vec<usize> = gate.let_defs.to_vec();
    inner_lets.sort_unstable();
    inner_lets.dedup();
    let mut inner_params: Vec<usize> = gate.closure_params.to_vec();
    inner_params.sort_unstable();
    inner_params.dedup();
    let is_outer_def = |off: usize| binder_spans.iter().any(|(s, e)| *s <= off && off < *e);
    let mut all_pat_defs: Vec<usize> = gate.all_pats.to_vec();
    all_pat_defs.sort_unstable();
    all_pat_defs.dedup();
    let is_pat_def = |off: usize| all_pat_defs.contains(&off);
    let inner_lets: Vec<usize> = inner_lets
        .into_iter()
        .filter(|o| {
            *o > binder_end
                && enclosing.map(|enc| within_enc(enc, *o)).unwrap_or(true)
                && !in_nested(*o)
                && !in_macro(*o)
                && !is_outer_def(*o)
        })
        .collect();
    let inner_params: Vec<usize> = inner_params
        .into_iter()
        .filter(|o| {
            *o > binder_end
                && enclosing.map(|enc| within_enc(enc, *o)).unwrap_or(true)
                && !in_nested(*o)
                && !in_macro(*o)
                && !is_outer_def(*o)
        })
        .collect();
    let mut extra: Vec<(usize, usize)> = extra_binder_shadow_ranges(parsed, original, index, old);
    extra.sort_unstable();
    extra.dedup();
    let extra: Vec<(usize, usize)> = extra
        .into_iter()
        .filter(|(d, _)| {
            *d > binder_end
                && enclosing.map(|enc| within_enc(enc, *d)).unwrap_or(true)
                && !in_nested(*d)
                && !in_macro(*d)
                && !is_outer_def(*d)
        })
        .collect();
    let mut shadows: Vec<(usize, usize)> = Vec::new();
    // Unmodelled binders already carry their construct end; keep the
    // recorded range (never widen to the enclosing end — uses past the
    // construct resume the outer binder and stay `off-target`).
    for (d, e) in &extra {
        if *e > *d {
            shadows.push((*d, *e));
        }
    }
    if let Some(enc) = enclosing {
        for d in &inner_lets {
            let mut best: Option<(usize, usize)> = None;
            for b in &gate.blocks {
                if b.0 <= *d && *d < b.1 && within_enc(enc, b.0) {
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
        for d in &inner_params {
            let mut best: Option<(usize, usize)> = None;
            for c in &gate.closures {
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
    }
    let is_shadowed = |off: usize| shadows.iter().any(|(d, e)| *d < off && off < *e);
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
    // All post-binder in-scope `old` ident offsets on `line`
    // (outside renamed/nested-fn/macro spans). Empty means no
    // claimable leftover on the line.
    let claimable_offsets = |line: u32| -> Vec<usize> {
        let mut found = Vec::new();
        let Some(ls) = index.line_start(line) else {
            return vec![usize::MAX];
        };
        let le = index.line_start(line + 1).unwrap_or(original.len());
        let Some(line_text) = original.get(ls..le.min(original.len())) else {
            return vec![usize::MAX];
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
                && !is_pat_def(a)
            {
                found.push(a);
            }
            off += pos + 1;
            if off >= line_text.len() {
                break;
            }
        }
        found
    };
    // True when `line` holds at least one post-binder `old` ident inside
    // the enclosing block, outside renamed/nested-fn/macro spans.
    let line_has_claimable = |line: u32| -> bool { !claimable_offsets(line).is_empty() };
    let refs = vec![(path.to_string(), parsed, original, index)];
    let mut out = collect_orphans(&refs, old, path, &renamed, false);
    // Inline format-string captures (`println!("{total}")`) hide the
    // ident inside a string literal — no `Ident` token for the path
    // collector above. Pre-binder macros never dangle, so only
    // post-binder captures inside the enclosing block are kept
    // (safe direction: downgrade). Byte-precise: the literal's own
    // offset (not the macro start line) gates block membership, so a
    // multi-line macro straddling the boundary attributes correctly.
    // Unmappable spans (`usize::MAX`) keep the legacy line gate.
    let capture_spans = format_capture_spans(parsed, old, original, index);
    let mut capture_off_by_line: std::collections::HashMap<u32, Vec<usize>> =
        std::collections::HashMap::new();
    for (line, off) in &capture_spans {
        capture_off_by_line.entry(*line).or_default().push(*off);
    }
    for (line, off) in &capture_spans {
        if *off == usize::MAX {
            if *line < thresh_line {
                continue;
            }
            if !line_in_scope(*line) {
                continue;
            }
        } else {
            if *off < thresh {
                continue;
            }
            if !in_enc(*off) || in_nested(*off) {
                continue;
            }
        }
        out.push(crate::OrphanReport {
            file: path.to_string(),
            line: *line,
            text: orphan_line_text(original, index, *line),
            reason: "macro-hidden".to_string(),
        });
    }
    // Byte offsets of `old` idents on `line` that sit inside a macro
    // invocation (opaque Ident case, e.g. `println!("{}", total)` split
    // across lines). Non-empty means the line truly holds a post-binder
    // in-block macro use — not just a macro start line.
    let macro_ident_offsets = |line: u32| -> Vec<usize> {
        let mut found = Vec::new();
        let Some(ls) = index.line_start(line) else {
            return vec![usize::MAX];
        };
        let le = index.line_start(line + 1).unwrap_or(original.len());
        let Some(line_text) = original.get(ls..le.min(original.len())) else {
            return vec![usize::MAX];
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
                && in_macro(a)
                && !is_pat_def(a)
            {
                found.push(a);
            }
            off += pos + 1;
            if off >= line_text.len() {
                break;
            }
        }
        found
    };
    // Macros by start line: `collect_orphans` reports the ident line
    // while the macro invocation may span several lines. Sibling-line
    // scan gates on ident bytes (a multi-line macro naming `old`
    // out-of-block stays dropped, ident-line orphans stay kept).
    let mut macros_by_start: std::collections::HashMap<u32, Vec<(usize, usize)>> =
        std::collections::HashMap::new();
    for r in &gate.macros {
        macros_by_start
            .entry(index.line_of(r.0))
            .or_default()
            .push(*r);
    }
    let macro_invocation_has_claimable = |start_line: u32| -> bool {
        let Some(ranges) = macros_by_start.get(&start_line) else {
            return false;
        };
        for (a, b) in ranges {
            if *b <= *a {
                continue;
            }
            let first = index.line_of(*a);
            let last = index.line_of(b.saturating_sub(1));
            for line in first..=last {
                if !macro_ident_offsets(line).is_empty() {
                    return true;
                }
            }
        }
        false
    };
    out.retain(|o| {
        if o.line < thresh_line {
            return false;
        }
        if o.reason == "macro-hidden" {
            // Format-capture lines carry exact literal offsets: any
            // in-block post-binder literal keeps the line (byte gate
            // already passed at push; re-check here for lines that
            // arrived via `collect_orphans` sharing the same line).
            if let Some(offs) = capture_off_by_line.get(&o.line) {
                if offs
                    .iter()
                    .any(|a| *a == usize::MAX || (*a >= thresh && in_enc(*a) && !in_nested(*a)))
                {
                    return true;
                }
            }
            // Otherwise (opaque Ident case): keep when the reported
            // line or any sibling line of the same macro invocation
            // holds a post-binder in-block macro ident, so a
            // multi-line macro starting in-block but naming `old`
            // out-of-block (or vice versa) attributes by bytes
            // without dropping a true orphan whose ident sits on a
            // later line than the reported start line.
            if !macro_ident_offsets(o.line).is_empty() {
                return true;
            }
            if macro_invocation_has_claimable(o.line) {
                return true;
            }
            // A macro starts on this line but holds no qualifying
            // ident anywhere in its span: the start-line report names
            // an out-of-scope use — drop it.
            if macros_by_start.contains_key(&o.line) {
                return false;
            }
            // No macro starts here (attribute/`qself`/unmappable-span
            // opacity the byte scan cannot see): legacy line gate
            // decides (safe direction, matches prior behaviour).
            return line_in_scope(o.line);
        }
        // `off-target`: keep only when a post-binder in-scope ident
        // remains on the line. This subsumes the legacy
        // `line > thresh_line → keep` rule while dropping other-fn /
        // other-block / nested-fn uses that merely share the name.
        line_has_claimable(o.line)
    });
    // Shadowed relabelling (WI-2b follow-up): leftovers inside an
    // inner `let old` / closure-param `old` shadow range name the
    // inner binder, so they report `shadowed`. A line reports
    // `shadowed` only when every leftover ident on it is shadowed;
    // any unshadowed leftover stays `off-target`.
    for o in out.iter_mut() {
        if o.reason != "off-target" {
            continue;
        }
        let offs = claimable_offsets(o.line);
        if offs.is_empty() || offs.iter().all(|a| *a == usize::MAX) {
            continue;
        }
        if !offs.is_empty() && offs.iter().all(|a| is_shadowed(*a)) {
            o.reason = "shadowed".to_string();
        }
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}
