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

/// Byte span in the pre-image file text.
type Span = (usize, usize);

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

/// Outer-scope `new` capture audit (slice 3 remainder): byte offsets of
/// `new`-binding pattern definitions that share a scope with `window`
/// (the target window of a sequence rename-once `old` → `new` hunk).
///
/// A pre-existing `new` binder in the window's own scope collides with
/// the rename either way: an earlier same-level/outer `let new` is
/// shadowed by the renamed definition (silently rebinding its later
/// uses), a later one double-binds the scope. Either case makes rewritten
/// post-window uses resolve under a shadowing twin, so the follow arm
/// declines and the definitions surface as `capture-risk` orphans.
///
/// Scope rule: only definitions whose innermost `Block` contains the whole
/// window count (same-level `let`s and outer-scope `let`s / fn params).
/// Definitions in descendant scopes (sibling inner blocks) never collide
/// with window-level uses and are skipped, as are nested-`fn` and macro
/// interiors (different scopes, like the follow walk). The window range
/// itself is excluded (window-local `new` stays owned by the
/// `sequence_binder_follows` pat guard). `for` / `if let` / `while let` /
/// match-arm / closure-param `new` binders govern sub-scopes the follow
/// walk already shadow-models, so they are known-skipped, not flagged; any
/// other same-scope `new` binder (unmodellable syntax) is flagged as
/// residual insurance.
///
/// Returns `None` when the enclosing block cannot be mapped (fail-safe:
/// callers decline the arm and skip `capture-risk`, never proceed).
pub(crate) fn enclosing_new_binder_defs(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    new: &str,
    window: (usize, usize),
) -> Option<Vec<usize>> {
    use syn::visit::Visit;
    struct Scopes<'a> {
        original: &'a str,
        index: &'a LineIndex,
        new: &'a str,
        blocks: Vec<(usize, usize)>,
        item_fns: Vec<(usize, usize)>,
        macros: Vec<(usize, usize)>,
        let_defs: Vec<usize>,
        sub_defs: Vec<usize>,
        all_pats: Vec<usize>,
        fn_param_defs: Vec<(usize, usize, usize)>,
    }
    impl<'ast> Visit<'ast> for Scopes<'_> {
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
                for arg in &node.sig.inputs {
                    if let syn::FnArg::Typed(t) = arg {
                        for off in crate::use_graph_local::pat_old_offsets(
                            &t.pat,
                            self.new,
                            self.original,
                            self.index,
                        ) {
                            self.fn_param_defs.push((off, a, b));
                        }
                    }
                }
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
            for off in node.inputs.iter().flat_map(|inp| {
                crate::use_graph_local::pat_old_offsets(inp, self.new, self.original, self.index)
            }) {
                self.sub_defs.push(off);
            }
            syn::visit::visit_expr_closure(self, node);
        }
        fn visit_local(&mut self, node: &'ast syn::Local) {
            for off in crate::use_graph_local::pat_old_offsets(
                &node.pat,
                self.new,
                self.original,
                self.index,
            ) {
                self.let_defs.push(off);
            }
            syn::visit::visit_local(self, node);
        }
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            if node.ident == self.new {
                let sp = node.ident.span();
                if let (Some(a), Some(b)) = (
                    self.index.offset(sp.start().line as u32, sp.start().column),
                    self.index.offset(sp.end().line as u32, sp.end().column),
                ) {
                    if self.original.get(a..b) == Some(self.new) {
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
        new,
        blocks: Vec::new(),
        item_fns: Vec::new(),
        macros: Vec::new(),
        let_defs: Vec::new(),
        sub_defs: Vec::new(),
        all_pats: Vec::new(),
        fn_param_defs: Vec::new(),
    };
    scopes.visit_file(parsed);
    // Smallest block containing the whole window is the window's scope.
    let mut enclosing: Option<(usize, usize)> = None;
    for b in &scopes.blocks {
        if b.0 <= window.0 && window.1 <= b.1 {
            let narrower = match enclosing {
                None => true,
                Some(e) => (b.1 - b.0) < (e.1 - e.0),
            };
            if narrower {
                enclosing = Some(*b);
            }
        }
    }
    let enc = enclosing?;
    let nested: Vec<(usize, usize)> = scopes
        .item_fns
        .iter()
        .copied()
        .filter(|r| enc.0 <= r.0 && r.1 <= enc.1)
        .collect();
    let in_window = |o: usize| window.0 <= o && o < window.1;
    let in_nested = |o: usize| nested.iter().any(|r| r.0 <= o && o < r.1);
    let in_macro = |o: usize| scopes.macros.iter().any(|r| r.0 <= o && o < r.1);
    // Innermost block containing `o`; `None` means no block maps the
    // offset (signature-level: only fn params live there, handled
    // separately — other offsets treat as outer scope, flag direction).
    let inner_block = |o: usize| -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        for b in &scopes.blocks {
            if b.0 <= o && o < b.1 {
                let narrower = match best {
                    None => true,
                    Some(e) => (b.1 - b.0) < (e.1 - e.0),
                };
                if narrower {
                    best = Some(*b);
                }
            }
        }
        best
    };
    // Same-scope collision: the def's innermost block contains the whole
    // window scope (same level or outer). Descendant scopes never collide
    // with window-level uses.
    let collides = |o: usize| match inner_block(o) {
        Some(inner) => inner.0 <= enc.0 && enc.1 <= inner.1,
        None => true,
    };
    let mut flagged: Vec<usize> = Vec::new();
    for o in &scopes.let_defs {
        if in_window(*o) || in_nested(*o) || in_macro(*o) {
            continue;
        }
        if collides(*o) {
            flagged.push(*o);
        }
    }
    for (o, fa, fb) in scopes.fn_param_defs.iter() {
        if *fa <= window.0 && window.1 <= *fb {
            flagged.push(*o);
        }
    }
    // Known sub-scope binders (closure params + construct-scoped binders
    // the follow walk already shadow-models): never flagged, but they own
    // their offsets so the residual below does not claim them.
    let mut known_sub: Vec<usize> = scopes.sub_defs;
    known_sub.extend(
        extra_binder_shadow_ranges(parsed, original, index, new)
            .into_iter()
            .map(|(d, _)| d),
    );
    known_sub.sort_unstable();
    known_sub.dedup();
    scopes.all_pats.sort_unstable();
    scopes.all_pats.dedup();
    scopes.let_defs.sort_unstable();
    scopes.let_defs.dedup();
    let fn_defs: Vec<usize> = scopes.fn_param_defs.iter().map(|(o, _, _)| *o).collect();
    for o in &scopes.all_pats {
        if in_window(*o) || in_nested(*o) || in_macro(*o) {
            continue;
        }
        if scopes.let_defs.contains(o) || fn_defs.contains(o) || known_sub.contains(o) {
            continue;
        }
        if collides(*o) {
            flagged.push(*o);
        }
    }
    flagged.sort_unstable();
    flagged.dedup();
    Some(flagged)
}

/// Sequence-window orphan cover (residue slice 3 + hole precision):
/// map the pattern-side rename/substitution positions onto one applied
/// target window.
///
/// Returns the def span plus extra exclusion spans (authored
/// substitution inits, whose directly-spelled old uses the plus side
/// rewrote verbatim), or `None` when the window cannot be mapped
/// exactly (fail-safe: the caller keeps today's whole-window cover).
/// Exclusion is per occurrence via a parallel minus/target structural
/// walk: hole-bound subtrees (minus side a `__RSMPL_*` placeholder)
/// contribute zero spans, so old idents smuggled through `E1`
/// (shadow/macro interiors) surface with existing reasons instead of
/// staying silent. Unmappable shapes fall back to the whole-statement
/// span (today's behaviour, no regression).
pub(crate) fn sequence_window_cover(
    kind: &rsmpl_pattern::PatternKind,
    window: (usize, usize),
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    old: &str,
) -> Option<(Span, Vec<Span>)> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;
    let rsmpl_pattern::PatternKind::General {
        shape, minus_sub, ..
    } = kind
    else {
        return None;
    };
    if *shape != rsmpl_pattern::GeneralShape::Sequence {
        return None;
    }
    let (idx, subs, _, _) = rsmpl_pattern::binder::sequence_binder_positions(kind)?;
    let minus_block = syn::parse_str::<syn::Block>(&format!("{{ {minus_sub} }}")).ok()?;
    let k = minus_block.stmts.len();
    if !(2..=4).contains(&k) || subs.iter().any(|j| *j >= k) || idx >= k {
        return None;
    }
    struct Blocks<'ast> {
        blocks: Vec<&'ast syn::Block>,
    }
    impl<'ast> Visit<'ast> for Blocks<'ast> {
        fn visit_block(&mut self, node: &'ast syn::Block) {
            self.blocks.push(node);
            syn::visit::visit_block(self, node);
        }
    }
    let mut grab = Blocks { blocks: Vec::new() };
    grab.visit_file(parsed);
    // Collect every K-run of plain `let` siblings whose byte span is
    // exactly the window edit span (positional correspondence with the
    // apply zip). At most one run can match; anything else bails.
    let mut runs: Vec<(Vec<Span>, Vec<syn::Local>)> = Vec::new();
    for block in &grab.blocks {
        if block.stmts.len() < k {
            continue;
        }
        for start in 0..=(block.stmts.len() - k) {
            let run = &block.stmts[start..start + k];
            let mut spans: Vec<(usize, usize)> = Vec::with_capacity(k);
            let mut locals: Vec<syn::Local> = Vec::with_capacity(k);
            let mut ok = true;
            for stmt in run {
                let syn::Stmt::Local(l) = stmt else {
                    ok = false;
                    break;
                };
                let Some(se) = crate::matcher_item::span_bytes(original, index, stmt.span()) else {
                    ok = false;
                    break;
                };
                spans.push(se);
                locals.push(l.clone());
            }
            if !ok {
                continue;
            }
            let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
                continue;
            };
            if (first.0, last.1) == window {
                runs.push((spans, locals));
            }
        }
    }
    if runs.len() != 1 {
        return None;
    }
    let (spans, run) = runs.pop().unwrap();
    // The target window must bind `old` exactly once, at the pattern
    // rename index.
    let mut hit: Option<usize> = None;
    for (i, local) in run.iter().enumerate() {
        let mut offs = crate::use_graph_local::pat_old_offsets(&local.pat, old, original, index);
        offs.sort_unstable();
        offs.dedup();
        if offs.is_empty() {
            continue;
        }
        if offs.len() != 1 || hit.is_some() {
            return None;
        }
        hit = Some(i);
    }
    if hit? != idx {
        return None;
    }
    let def = spans[idx];
    let mut extra: Vec<(usize, usize)> = Vec::new();
    for j in subs {
        match substitution_excluded_spans(
            &minus_local_at(minus_sub, j),
            &run[j],
            original,
            index,
            old,
        ) {
            Some(v) if !v.is_empty() => extra.extend(v),
            Some(_) => {}
            None => extra.push(spans[j]),
        }
    }
    Some((def, extra))
}

/// Pattern-side minus `Local` at position `j` (re-parsed from
/// `minus_sub`; the caller already validated K and the index).
fn minus_local_at(minus_sub: &str, j: usize) -> syn::Local {
    let minus_block =
        syn::parse_str::<syn::Block>(&format!("{{ {minus_sub} }}")).expect("checked minus block");
    match minus_block.stmts[j].clone() {
        syn::Stmt::Local(l) => l,
        _ => unreachable!("checked plain lets"),
    }
}

/// Per-occurrence exclusion for one authored substitution position:
/// byte spans of directly-spelled `old` uses the minus side proves
/// swapped. Hole-bound subtrees contribute nothing (their olds stay
/// visible); structural mismatch yields `None` (whole-statement
/// fallback).
fn substitution_excluded_spans(
    minus_local: &syn::Local,
    target_local: &syn::Local,
    original: &str,
    index: &LineIndex,
    old: &str,
) -> Option<Vec<Span>> {
    if render_tt(&minus_local.pat) != render_tt(&target_local.pat) {
        return None;
    }
    let m_init = minus_local.init.as_ref().map(|i| &*i.expr)?;
    let t_init = target_local.init.as_ref().map(|i| &*i.expr)?;
    collect_expr(m_init, t_init, original, index, old)
}

fn is_placeholder_ident(s: &str) -> bool {
    s.starts_with("__RSMPL_")
}

fn is_placeholder_expr(e: &syn::Expr) -> bool {
    if let syn::Expr::Path(p) = e {
        if p.qself.is_none()
            && p.path.segments.len() == 1
            && matches!(p.path.segments[0].arguments, syn::PathArguments::None)
        {
            return is_placeholder_ident(&p.path.segments[0].ident.to_string());
        }
    }
    false
}

fn is_old_ident_expr(e: &syn::Expr, old: &str) -> bool {
    if let syn::Expr::Path(p) = e {
        if p.qself.is_none()
            && p.path.leading_colon.is_none()
            && p.path.segments.len() == 1
            && matches!(p.path.segments[0].arguments, syn::PathArguments::None)
        {
            return p.path.segments[0].ident == old;
        }
    }
    false
}

fn render_tt<T: quote::ToTokens>(n: &T) -> String {
    quote::quote!(#n).to_string()
}

/// Ident-bounded `old` spans inside `[start, end)`.
fn old_spans_in_range(start: usize, end: usize, original: &str, old: &str) -> Vec<Span> {
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let mut out = Vec::new();
    if old.is_empty() || end <= start || end > original.len() {
        return out;
    }
    let hay = &original.as_bytes()[start..end];
    let needle = old.as_bytes();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            let prev_ok = i == 0 || !is_ident_byte(hay[i - 1]);
            let next_ok = i + needle.len() == hay.len() || !is_ident_byte(hay[i + needle.len()]);
            if prev_ok && next_ok {
                out.push((start + i, start + i + needle.len()));
            }
        }
        i += 1;
    }
    out
}

fn target_expr_span(e: &syn::Expr, original: &str, index: &LineIndex) -> Option<Span> {
    use syn::spanned::Spanned;
    crate::matcher_item::span_bytes(original, index, e.span())
}

/// Parallel minus/target walk over a substitution init. `Some` carries
/// the directly-spelled swapped spans (hole subtrees empty); `None`
/// asks the caller to fall back wholesale.
fn collect_expr(
    m: &syn::Expr,
    t: &syn::Expr,
    original: &str,
    index: &LineIndex,
    old: &str,
) -> Option<Vec<Span>> {
    if is_placeholder_expr(m) {
        return Some(Vec::new());
    }
    if is_old_ident_expr(m, old) {
        if is_old_ident_expr(t, old) {
            return target_expr_span(t, original, index).map(|s| vec![s]);
        }
        return None;
    }
    match (m, t) {
        (syn::Expr::Path(a), syn::Expr::Path(b)) => {
            if render_tt(a) == render_tt(b) {
                Some(Vec::new())
            } else {
                None
            }
        }
        (syn::Expr::Lit(a), syn::Expr::Lit(b)) => {
            if render_tt(a) == render_tt(b) {
                Some(Vec::new())
            } else {
                None
            }
        }
        (syn::Expr::Binary(a), syn::Expr::Binary(b)) => {
            if render_tt(&a.op) != render_tt(&b.op) {
                return None;
            }
            let mut out = collect_expr(&a.left, &b.left, original, index, old)?;
            out.extend(collect_expr(&a.right, &b.right, original, index, old)?);
            Some(out)
        }
        (syn::Expr::Unary(a), syn::Expr::Unary(b)) => {
            if render_tt(&a.op) != render_tt(&b.op) {
                return None;
            }
            collect_expr(&a.expr, &b.expr, original, index, old)
        }
        (syn::Expr::Paren(a), syn::Expr::Paren(b)) => {
            collect_expr(&a.expr, &b.expr, original, index, old)
        }
        (syn::Expr::Group(a), syn::Expr::Group(b)) => {
            collect_expr(&a.expr, &b.expr, original, index, old)
        }
        (syn::Expr::Reference(a), syn::Expr::Reference(b)) => {
            if render_tt(&a.mutability) != render_tt(&b.mutability) {
                return None;
            }
            collect_expr(&a.expr, &b.expr, original, index, old)
        }
        (syn::Expr::Cast(a), syn::Expr::Cast(b)) => {
            if render_tt(&a.ty) != render_tt(&b.ty) {
                return None;
            }
            collect_expr(&a.expr, &b.expr, original, index, old)
        }
        (syn::Expr::Assign(a), syn::Expr::Assign(b)) => {
            let mut out = collect_expr(&a.left, &b.left, original, index, old)?;
            out.extend(collect_expr(&a.right, &b.right, original, index, old)?);
            Some(out)
        }
        (syn::Expr::Call(a), syn::Expr::Call(b)) => {
            if a.args.len() != b.args.len() {
                return None;
            }
            let mut out = collect_expr(&a.func, &b.func, original, index, old)?;
            for (x, y) in a.args.iter().zip(b.args.iter()) {
                out.extend(collect_expr(x, y, original, index, old)?);
            }
            Some(out)
        }
        (syn::Expr::MethodCall(a), syn::Expr::MethodCall(b)) => {
            if a.method != b.method || a.args.len() != b.args.len() {
                return None;
            }
            let mut out = collect_expr(&a.receiver, &b.receiver, original, index, old)?;
            for (x, y) in a.args.iter().zip(b.args.iter()) {
                out.extend(collect_expr(x, y, original, index, old)?);
            }
            Some(out)
        }
        (syn::Expr::Field(a), syn::Expr::Field(b)) => {
            if render_tt(&a.member) != render_tt(&b.member) {
                return None;
            }
            collect_expr(&a.base, &b.base, original, index, old)
        }
        (syn::Expr::Index(a), syn::Expr::Index(b)) => {
            let mut out = collect_expr(&a.expr, &b.expr, original, index, old)?;
            out.extend(collect_expr(&a.index, &b.index, original, index, old)?);
            Some(out)
        }
        (syn::Expr::Array(a), syn::Expr::Array(b)) => {
            if a.elems.len() != b.elems.len() {
                return None;
            }
            let mut out = Vec::new();
            for (x, y) in a.elems.iter().zip(b.elems.iter()) {
                out.extend(collect_expr(x, y, original, index, old)?);
            }
            Some(out)
        }
        (syn::Expr::Tuple(a), syn::Expr::Tuple(b)) => {
            if a.elems.len() != b.elems.len() {
                return None;
            }
            let mut out = Vec::new();
            for (x, y) in a.elems.iter().zip(b.elems.iter()) {
                out.extend(collect_expr(x, y, original, index, old)?);
            }
            Some(out)
        }
        (syn::Expr::Struct(a), syn::Expr::Struct(b)) => {
            if render_tt(&a.path) != render_tt(&b.path)
                || a.fields.len() != b.fields.len()
                || render_tt(&a.rest) != render_tt(&b.rest)
            {
                return None;
            }
            let mut out = Vec::new();
            for (x, y) in a.fields.iter().zip(b.fields.iter()) {
                if render_tt(&x.member) != render_tt(&y.member) {
                    return None;
                }
                out.extend(collect_expr(&x.expr, &y.expr, original, index, old)?);
            }
            Some(out)
        }
        (syn::Expr::Block(a), syn::Expr::Block(b)) => {
            if render_tt(&a.label) != render_tt(&b.label)
                || a.block.stmts.len() != b.block.stmts.len()
            {
                return None;
            }
            let mut out = Vec::new();
            for (x, y) in a.block.stmts.iter().zip(b.block.stmts.iter()) {
                out.extend(collect_stmt(x, y, original, index, old)?);
            }
            Some(out)
        }
        (syn::Expr::Macro(a), syn::Expr::Macro(b)) => {
            if render_tt(&a.mac.path) != render_tt(&b.mac.path) {
                return None;
            }
            let m_toks = render_tt(&a.mac.tokens);
            if m_toks.contains("__RSMPL_") {
                return Some(Vec::new());
            }
            if render_tt(&a.mac.tokens) != render_tt(&b.mac.tokens) {
                return None;
            }
            use syn::spanned::Spanned;
            if let Some((ts, te)) = crate::matcher_item::span_bytes(original, index, t.span()) {
                return Some(old_spans_in_range(ts, te, original, old));
            }
            None
        }
        _ => {
            if render_tt(m) == render_tt(t) {
                use syn::spanned::Spanned;
                if let Some((s, e)) = crate::matcher_item::span_bytes(original, index, t.span()) {
                    return Some(old_spans_in_range(s, e, original, old));
                }
                return Some(Vec::new());
            }
            None
        }
    }
}

fn collect_stmt(
    m: &syn::Stmt,
    t: &syn::Stmt,
    original: &str,
    index: &LineIndex,
    old: &str,
) -> Option<Vec<Span>> {
    match (m, t) {
        (syn::Stmt::Local(a), syn::Stmt::Local(b)) => {
            if render_tt(&a.pat) != render_tt(&b.pat) {
                return None;
            }
            match (&a.init, &b.init) {
                (Some(x), Some(y)) => collect_expr(&x.expr, &y.expr, original, index, old),
                (None, None) => Some(Vec::new()),
                _ => None,
            }
        }
        (syn::Stmt::Expr(a, _), syn::Stmt::Expr(b, _)) => collect_expr(a, b, original, index, old),
        _ => {
            if render_tt(m) == render_tt(t) {
                Some(Vec::new())
            } else {
                None
            }
        }
    }
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
///
/// Residue slice 3: for sequence binder windows the cover narrows from
/// the whole-window edit to the def span plus follow spans (plus
/// authored-substitution exclusions), so in-window leftovers surface
/// through the existing claimable/shadow/macro machinery instead of
/// hiding inside the window edit. Unmappable windows keep the legacy
/// whole-window cover (fail-safe).
pub(crate) fn let_binder_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    old: &str,
    compiled: &rsmpl_pattern::CompiledPattern,
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
    // Sequence narrowing (slice 3): map each non-follow window edit
    // onto its def + substitution spans. All windows must map exactly;
    // otherwise keep the legacy whole-window cover (fail-safe).
    let mut narrowed: Option<(Vec<Span>, Vec<Span>)> = None;
    // Slice 3 remainder: per-window outer `new` definitions for the
    // `capture-risk` audit below (sequence rename-once only, plus the
    // Stmt single-`let` parity: plain + single-leaf destructuring; the
    // binder's `new` name comes from the compiled pattern, not the
    // orphan `old`).
    let mut capture_windows: Vec<(usize, usize)> = Vec::new();
    let mut capture_new: Option<String> = None;
    if let rsmpl_pattern::PatternKind::General {
        shape: rsmpl_pattern::GeneralShape::Sequence,
        ..
    } = &compiled.kind
    {
        let windows: Vec<(usize, usize)> = hunk_edits
            .iter()
            .filter(|e| e.label != Some("follow"))
            .map(|e| (e.start, e.end))
            .collect();
        if !windows.is_empty() {
            let mut defs: Vec<(usize, usize)> = Vec::new();
            let mut extras: Vec<(usize, usize)> = Vec::new();
            let mut ok = true;
            for w in windows.iter().copied() {
                match sequence_window_cover(&compiled.kind, w, parsed, original, index, old) {
                    Some((d, e)) => {
                        defs.push(d);
                        extras.extend(e);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                narrowed = Some((defs, extras));
                if let Some(b) = rsmpl_pattern::binder::binder_rename(&compiled.kind) {
                    if b.scope == rsmpl_pattern::binder::BinderScope::Let {
                        capture_windows = windows;
                        capture_new = Some(b.new);
                    }
                }
            }
        }
    }
    // Stmt single-`let` parity: the rename statement spans are already
    // the def cover (no narrowing needed); audit each for a pre-existing
    // `new` binder in its own scope, exactly like the sequence windows.
    if capture_windows.is_empty()
        && crate::use_graph_local::let_binder_rename(&compiled.kind).is_some()
    {
        let windows: Vec<(usize, usize)> = hunk_edits
            .iter()
            .filter(|e| e.label != Some("follow"))
            .map(|e| (e.start, e.end))
            .collect();
        if !windows.is_empty() {
            if let Some(b) = rsmpl_pattern::binder::binder_rename(&compiled.kind) {
                if b.scope == rsmpl_pattern::binder::BinderScope::Let {
                    capture_windows = windows;
                    capture_new = Some(b.new);
                }
            }
        }
    }
    let follow_spans: Vec<(usize, usize)> = hunk_edits
        .iter()
        .filter(|e| e.label == Some("follow"))
        .map(|e| (e.start, e.end))
        .collect();
    let (renamed, binder_spans): (Vec<Span>, Vec<Span>) = match narrowed {
        Some((defs, extras)) => {
            let mut r = defs.clone();
            r.extend(follow_spans);
            r.extend(extras);
            (r, defs)
        }
        None => {
            let renamed: Vec<(usize, usize)> =
                hunk_edits.iter().map(|e| (e.start, e.end)).collect();
            let binder_spans: Vec<(usize, usize)> = hunk_edits
                .iter()
                .filter(|e| e.label != Some("follow"))
                .map(|e| (e.start, e.end))
                .collect();
            (renamed, binder_spans)
        }
    };
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
    // Slice 3 remainder: outer-scope `new` capture audit. A pre-existing
    // `new` binder in the window's own scope collides with the rename, so
    // each such definition surfaces as a `capture-risk` orphan (pushed
    // after the retain gates above — outer definitions before the binder
    // must not be dropped by the post-binder line gate; they are already
    // scope-gated by the helper). Non-empty output caps at `partial` via
    // the existing orphans-nonempty downgrade; no postcheck plumbing.
    if let Some(new) = capture_new {
        for w in &capture_windows {
            if let Some(defs) = enclosing_new_binder_defs(parsed, original, index, &new, *w) {
                for d in defs {
                    let line = index.line_of(d);
                    out.push(crate::OrphanReport {
                        file: path.to_string(),
                        line,
                        text: crate::use_graph_local::orphan_line_text(original, index, line),
                        reason: "capture-risk".to_string(),
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}

#[cfg(test)]
mod tests {
    use crate::lines::LineIndex;

    fn excluded(minus_init: &str, target_init: &str, old: &str) -> Option<Vec<(usize, usize)>> {
        let m: syn::Local = syn::parse_str::<syn::Stmt>(&format!("let b = {minus_init};"))
            .ok()
            .and_then(|s| match s {
                syn::Stmt::Local(l) => Some(l),
                _ => None,
            })?;
        let t: syn::Local = syn::parse_str::<syn::Stmt>(&format!("let b = {target_init};"))
            .ok()
            .and_then(|s| match s {
                syn::Stmt::Local(l) => Some(l),
                _ => None,
            })?;
        let original = format!("fn f() {{\n    let b = {target_init};\n}}\n");
        let index = LineIndex::new(&original);
        super::substitution_excluded_spans(&m, &t, &original, &index, old)
    }

    #[test]
    fn hole_block_surfaces_zero_spans() {
        // `E1` binds the whole block: nothing excluded, the inner
        // shadowed use stays visible to the orphan scan.
        let out = excluded(
            "f (old , __RSMPL_E1)",
            "f (old , { let old = 1 ; old })",
            "old",
        )
        .expect("mappable");
        assert_eq!(out.len(), 1, "got: {out:?}");
    }

    #[test]
    fn hole_macro_surfaces_zero_for_hole_subtree() {
        let out =
            excluded("f (old , __RSMPL_E1)", "f (old , foo ! (old))", "old").expect("mappable");
        assert_eq!(out.len(), 1, "got: {out:?}");
    }

    #[test]
    fn concrete_call_excludes_direct_old() {
        let out = excluded("f (old)", "f (old)", "old").expect("mappable");
        assert_eq!(out.len(), 1, "got: {out:?}");
    }

    #[test]
    fn mismatch_falls_back_none() {
        assert!(excluded("f (old)", "g (old)", "old").is_none());
    }

    fn outer_new_lines(src: &str, new: &str, wstart_line: u32, wend_line: u32) -> Vec<u32> {
        // `capture-risk` helper unit probe: the window is the 1-based
        // closed line range `[wstart_line, wend_line]` (whole lines —
        // unit-level stand-in for the statement byte span).
        let parsed: syn::File = syn::parse_str(src).expect("test src parses");
        let index = LineIndex::new(src);
        let ws = index.line_start(wstart_line).expect("window start maps");
        let we = index.line_start(wend_line + 1).unwrap_or(src.len());
        let out = super::enclosing_new_binder_defs(&parsed, src, &index, new, (ws, we))
            .expect("window maps to an enclosing block");
        let mut lines: Vec<u32> = out.iter().map(|o| index.line_of(*o)).collect();
        lines.sort_unstable();
        lines
    }

    #[test]
    fn outer_let_before_window_flagged() {
        let src = "fn f() {\n    let new = 0;\n    let old = g(1);\n    let b = h();\n    let c = old;\n}\n";
        assert_eq!(outer_new_lines(src, "new", 3, 4), vec![2]);
    }

    #[test]
    fn outer_let_after_window_flagged() {
        let src = "fn f() {\n    let old = g(1);\n    let b = h();\n    let c = old;\n    let new = 9;\n}\n";
        assert_eq!(outer_new_lines(src, "new", 2, 3), vec![5]);
    }

    #[test]
    fn no_outer_new_is_empty() {
        let src = "fn f() {\n    let old = g(1);\n    let b = h();\n    let c = old;\n}\n";
        assert!(outer_new_lines(src, "new", 2, 3).is_empty());
    }

    #[test]
    fn sibling_block_new_not_flagged() {
        // A `new` binder in a descendant scope never collides with
        // window-level uses.
        let src = "fn f() {\n    let old = g(1);\n    let b = h();\n    { let new = 1; }\n    let c = old;\n}\n";
        assert!(outer_new_lines(src, "new", 2, 3).is_empty());
    }

    #[test]
    fn outer_block_new_flagged_for_nested_window() {
        // The window sits in an `if` body; the outer-block `let new`
        // shares the use scope.
        let src = "fn f(c: bool) {\n    let new = 0;\n    if c {\n        let old = g(1);\n        let b = h();\n    }\n}\n";
        assert_eq!(outer_new_lines(src, "new", 4, 5), vec![2]);
    }

    #[test]
    fn fn_param_new_flagged() {
        let src = "fn f(new: u32) {\n    let old = g(1);\n    let b = h();\n}\n";
        assert_eq!(outer_new_lines(src, "new", 2, 3), vec![1]);
    }

    #[test]
    fn other_fn_param_new_not_flagged() {
        // A `new` param on a different function is another scope.
        let src = "fn f() {\n    let old = g(1);\n    let b = h();\n}\nfn g(new: u32) {}\n";
        assert!(outer_new_lines(src, "new", 2, 3).is_empty());
    }

    #[test]
    fn for_loop_new_not_flagged() {
        // Construct-scoped `new` binders are already shadow-modelled by
        // the follow walk; they never collide at window level.
        let src = "fn f() {\n    let old = g(1);\n    let b = h();\n    for new in 0..3 { let x = new; }\n}\n";
        assert!(outer_new_lines(src, "new", 2, 3).is_empty());
    }

    #[test]
    fn closure_param_new_not_flagged() {
        let src =
            "fn f() {\n    let old = g(1);\n    let b = h();\n    let g = |new: u32| new;\n}\n";
        assert!(outer_new_lines(src, "new", 2, 3).is_empty());
    }

    #[test]
    fn window_local_new_excluded() {
        // `new` bound inside the window itself stays owned by the
        // `sequence_binder_follows` pat guard, never by this helper.
        let src = "fn f() {\n    let old = g(1);\n    let new = h();\n}\n";
        assert!(outer_new_lines(src, "new", 2, 3).is_empty());
    }

    #[test]
    fn nested_fn_let_new_not_flagged() {
        let src =
            "fn f() {\n    let old = g(1);\n    let b = h();\n    fn inner() { let new = 1; }\n}\n";
        assert!(outer_new_lines(src, "new", 2, 3).is_empty());
    }

    fn stmt_let_compiled() -> rsmpl_pattern::CompiledPattern {
        // Single-`let` rename `let old = E;` -> `let new = E;`
        // (same shape the Stmt follow arm completes).
        rsmpl_pattern::CompiledPattern {
            kind: rsmpl_pattern::PatternKind::General {
                shape: rsmpl_pattern::GeneralShape::Stmt,
                minus_sub: "let old = __RSMPL_E0;".to_string(),
                plus_raw: "let new = E;".to_string(),
                plus_body: "let new = E;".to_string(),
                placeholders: vec![("E".to_string(), "__RSMPL_E0".to_string())],
                literals: Vec::new(),
                minus_attrs: Vec::new(),
                plus_attrs: Vec::new(),
                had_semi: false,
            },
            table: Default::default(),
            minus_bindings: Default::default(),
            plus_bindings: Default::default(),
            unbound_plus: Vec::new(),
            repeated: Vec::new(),
            root: None,
        }
    }

    fn stmt_let_orphans(src: &str) -> Vec<crate::OrphanReport> {
        // One non-`follow` rename edit (the declined-follow posture: the
        // definition applies, the follow arm stays home).
        let parsed: syn::File = syn::parse_str(src).expect("test src parses");
        let index = LineIndex::new(src);
        let stmt_start = src.find("let old").expect("binder stmt present");
        let stmt_end = src[stmt_start..].find(';').expect("stmt ends") + stmt_start + 1;
        let mut states = std::collections::HashMap::new();
        states.insert(
            "src/main.rs".to_string(),
            crate::lines::FileState {
                original: src.to_string(),
                index,
                parsed: Some(parsed),
            },
        );
        let mut file_edits = std::collections::HashMap::new();
        file_edits.insert(
            "src/main.rs".to_string(),
            vec![crate::edits::TextEdit {
                start: stmt_start,
                end: stmt_end,
                replacement: "let new = 1;".to_string(),
                label: None,
            }],
        );
        let compiled = stmt_let_compiled();
        super::let_binder_orphans(&states, &file_edits, "src/main.rs", 0, "old", &compiled)
    }

    #[test]
    fn stmt_single_let_outer_new_reports_capture_risk() {
        // Stmt parity with the sequence guard: the outer `let new`
        // surfaces as `capture-risk` while the stranded post-binder use
        // stays `off-target`.
        let src = "fn main() {\n    let new = 0;\n    let old = 1;\n    let c = old;\n}\n";
        let orphans = stmt_let_orphans(src);
        assert_eq!(orphans.len(), 2, "got: {orphans:?}");
        assert!(
            orphans
                .iter()
                .any(|o| o.line == 2 && o.reason == "capture-risk" && o.text == "let new = 0;"),
            "got: {orphans:?}"
        );
        assert!(
            orphans
                .iter()
                .any(|o| o.line == 4 && o.reason == "off-target"),
            "got: {orphans:?}"
        );
    }

    #[test]
    fn stmt_single_let_no_outer_new_has_no_capture_risk() {
        // Without a pre-existing `new` the orphan side stays exactly as
        // before (one stranded use, no `capture-risk`).
        let src = "fn main() {\n    let old = 1;\n    let c = old;\n}\n";
        let orphans = stmt_let_orphans(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 3, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "off-target", "got: {orphans:?}");
    }
}
