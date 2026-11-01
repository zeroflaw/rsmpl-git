//! WI-2a/2b: `let`-binder rename honesty + block completion.
//!
//! WI-2a detects the single-binder rename shape (`let old = E;` → `let
//! new = E;`, same init, same type annotation) and lists leftover
//! same-name uses via the proven [`crate::use_graph_orphan::collect_orphans`]
//! path collector. WI-2b additionally rewrites block-scoped value uses in
//! the same hunk (labelled `follow`); shadowing, nested-fn, and macro
//! interiors stay evidence (orphans + downgrade), never rewritten.

use std::collections::HashMap;

use rsmpl_pattern::{GeneralShape, PatternKind};

use crate::edits::TextEdit;
use crate::lines::{FileState, LineIndex};
use crate::use_graph_orphan::collect_orphans;

/// WI-9a: orphan evidence for definition-only member renames (SPEC
/// 14.8). Mirrors the scalar orphan block: the definition hunk's own
/// ident-span edits plus every loaded `.rs` file feed the member
/// collector. (Moved from `plan.rs` so that file stays under the
/// 2000-line hygiene gate; behaviour-preserving move.)
#[allow(clippy::too_many_arguments)]
pub(crate) fn member_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    kind: crate::use_graph_member::MemberKind,
    container: &str,
    old: &str,
) -> Vec<crate::OrphanReport> {
    let renamed: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> = Vec::new();
    let mut paths: Vec<&String> = states.keys().collect();
    paths.sort();
    for p in paths {
        if let Some(st) = states.get(p) {
            if let Some(parsed) = st.parsed.as_ref() {
                refs.push((p.clone(), parsed, st.original.as_str(), &st.index));
            }
        }
    }
    crate::use_graph_member::collect_member_orphans(&refs, kind, container, old, path, &renamed)
}

/// WI-27: orphan evidence for definition-only assoc renames (SPEC
/// 14.8). Without follow, orphans list every container-proven use site
/// (`Self::`, `S::`, `Tr::`, qself tails) the rename leaves behind.
/// Mirrors `member_orphans`: the definition hunk's own ident-span edits
/// plus every loaded `.rs` file feed the assoc collector. An
/// unparseable header yields no orphans (the definition hunk itself
/// matches nothing, so the run already fails loud there).
/// (Moved from `plan.rs`; behaviour-preserving move.)
#[allow(clippy::too_many_arguments)]
pub(crate) fn assoc_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    impl_header: &str,
    want_trait: bool,
    is_type: bool,
    old: &str,
) -> Vec<crate::OrphanReport> {
    let Some(domain) = crate::use_graph_assoc::assoc_domain(impl_header, want_trait, is_type, old)
    else {
        return Vec::new();
    };
    let Some(filter) = rsmpl_pattern::parse_impl_header(impl_header) else {
        return Vec::new();
    };
    let renamed: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> = Vec::new();
    let mut paths: Vec<&String> = states.keys().collect();
    paths.sort();
    for p in paths {
        if let Some(st) = states.get(p) {
            if let Some(parsed) = st.parsed.as_ref() {
                refs.push((p.clone(), parsed, st.original.as_str(), &st.index));
            }
        }
    }
    crate::use_graph_assoc::collect_assoc_orphans(&refs, &domain, &filter, path, &renamed)
}

/// True when `compiled` is a plain single-binder `let` rename.
pub fn is_let_binder_rename(compiled: &rsmpl_pattern::CompiledPattern) -> bool {
    let_binder_rename(&compiled.kind).is_some()
}

/// WI-2c/2d: true when `compiled` renames exactly one local binder —
/// a `let` binder (WI-2a), a closure parameter, a match-arm pattern
/// (WI-2c), or a `for` / `if let` / `while let` binder (WI-2d) — and
/// nothing else in the pattern changes. The downgrade gate uses this:
/// any leftover use forces `reparse-only` (global `partial`).
pub fn is_local_binder_rename(compiled: &rsmpl_pattern::CompiledPattern) -> bool {
    is_let_binder_rename(compiled)
        || closure_param_rename(&compiled.kind).is_some()
        || match_arm_binder_rename(&compiled.kind).is_some()
        || crate::use_graph_binder::is_wi2d_expr_binder_rename(compiled)
}

/// `(old, new)` binder idents when `kind` is a plain `let old = E;` →
/// `let new = E;` rename with identical init and type annotation.
/// Anything else (destructuring, init/type change, `let-else`,
/// attribute delta) yields `None` — today's behaviour is kept.
pub fn let_binder_rename(kind: &PatternKind) -> Option<(String, String)> {
    let PatternKind::General {
        shape: GeneralShape::Stmt,
        minus_sub,
        plus_body,
        placeholders,
        minus_attrs,
        plus_attrs,
        ..
    } = kind
    else {
        return None;
    };
    if *minus_attrs != *plus_attrs {
        return None;
    }
    let Ok(syn::Stmt::Local(minus_local)) = syn::parse_str::<syn::Stmt>(minus_sub) else {
        return None;
    };
    let Ok(syn::Stmt::Local(plus_local)) = syn::parse_str::<syn::Stmt>(plus_body) else {
        return None;
    };
    // Plain `let` only: this syn tree has no `let-else` diverge field;
    // a `let-else` minus side fails to parse as `Stmt::Local` and yields
    // `None` below, keeping today's behaviour.
    let (old_id, old_mut) = single_ident_binder(&minus_local.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus_local.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    // Type annotation must match (both absent or token-equal after
    // placeholder normalization).
    let ph_map: HashMap<&str, &str> = placeholders
        .iter()
        .map(|(name, ph)| (ph.as_str(), name.as_str()))
        .collect();
    if normalize_ty(&minus_local.pat, &ph_map) != normalize_ty(&plus_local.pat, &ph_map) {
        return None;
    }
    // Both sides need the same init expression (token-equal after
    // mapping `__RSMPL_*` placeholders back to their names; both sides
    // normalized so `...` markers compare intact).
    let minus_init = minus_local.init.as_ref().map(|i| init_tokens(&i.expr))?;
    let plus_init = plus_local.init.as_ref().map(|i| init_tokens(&i.expr))?;
    if normalize_placeholders(&minus_init, &ph_map) != normalize_placeholders(&plus_init, &ph_map) {
        return None;
    }
    Some((old_id, new_id))
}

/// WI-2c: `(old, new)` when `kind` is a closure-parameter rename —
/// `|old| Body` → `|new| Body` (or an annotated `|old: T| Body` pair
/// with the same `T`) with placeholder-normalized-equal bodies.
/// Qualifiers (async/static/move/capture/lifetimes/return type) must match;
/// exactly one parameter may differ, and the differing pair must be a
/// single ident with the same `P: T` annotation discipline as `let`
/// (bare `P` never drops/adds an annotation). A full interior
/// old→new rewrite is deliberately NOT detected: nothing can dangle
/// outside the closure, so it needs no evidence and stays today's
/// `instantiated`.
pub fn closure_param_rename(kind: &PatternKind) -> Option<(String, String)> {
    let PatternKind::General {
        shape: GeneralShape::Expr,
        minus_sub,
        plus_body,
        placeholders,
        minus_attrs,
        plus_attrs,
        ..
    } = kind
    else {
        return None;
    };
    if *minus_attrs != *plus_attrs {
        return None;
    }
    let Ok(syn::Expr::Closure(minus)) = syn::parse_str::<syn::Expr>(minus_sub) else {
        return None;
    };
    let Ok(syn::Expr::Closure(plus)) = syn::parse_str::<syn::Expr>(plus_body) else {
        return None;
    };
    // Qualifiers compare strictly (a different qualifier is a
    // different closure). Fields are rendered via locals: `#m.f`
    // in `quote!` would interpolate `m`, not the field.
    if crate::matcher_general::render(&minus.asyncness)
        != crate::matcher_general::render(&plus.asyncness)
    {
        return None;
    }
    if crate::matcher_general::render(&minus.movability)
        != crate::matcher_general::render(&plus.movability)
    {
        return None;
    }
    if crate::matcher_general::render(&minus.capture)
        != crate::matcher_general::render(&plus.capture)
    {
        return None;
    }
    if crate::matcher_general::render(&minus.lifetimes)
        != crate::matcher_general::render(&plus.lifetimes)
    {
        return None;
    }
    if crate::matcher_general::render(&minus.constness)
        != crate::matcher_general::render(&plus.constness)
    {
        return None;
    }
    if crate::matcher_general::render(&minus.output) != crate::matcher_general::render(&plus.output)
    {
        return None;
    }
    if minus.inputs.len() != plus.inputs.len() {
        return None;
    }
    let ph_map: HashMap<&str, &str> = placeholders
        .iter()
        .map(|(name, ph)| (ph.as_str(), name.as_str()))
        .collect();
    let mut diff: Option<(String, String)> = None;
    for (mp, pp) in minus.inputs.iter().zip(plus.inputs.iter()) {
        let mn = normalize_placeholders(&quote::quote!(#mp).to_string(), &ph_map);
        let pn = normalize_placeholders(&quote::quote!(#pp).to_string(), &ph_map);
        if mn == pn {
            continue;
        }
        let (old_id, old_mut) = single_ident_binder(mp)?;
        let (new_id, new_mut) = single_ident_binder(pp)?;
        if old_id == new_id || old_mut != new_mut {
            return None;
        }
        if normalize_ty(mp, &ph_map) != normalize_ty(pp, &ph_map) {
            return None;
        }
        if diff.replace((old_id, new_id)).is_some() {
            return None;
        }
    }
    let (old, new) = diff?;
    let minus_body = crate::matcher_general::render(&*minus.body);
    let plus_raw_body = crate::matcher_general::render(&*plus.body);
    if normalize_placeholders(&minus_body, &ph_map)
        != normalize_placeholders(&plus_raw_body, &ph_map)
    {
        return None;
    }
    Some((old, new))
}

/// WI-2c: `(old, new)` when `kind` is a match-arm binder rename —
/// `match S { old => B, … }` → `match S { new => B, … }` with the same
/// scrutinee, arm count, guard, and body (placeholder-normalized).
/// Only a single-ident arm pattern may change; destructuring,
/// scrutinee/guard/body deltas, and multi-arm diffs veto (today's
/// behaviour is kept).
pub fn match_arm_binder_rename(kind: &PatternKind) -> Option<(String, String)> {
    let PatternKind::General {
        shape: GeneralShape::Expr,
        minus_sub,
        plus_body,
        placeholders,
        minus_attrs,
        plus_attrs,
        ..
    } = kind
    else {
        return None;
    };
    if *minus_attrs != *plus_attrs {
        return None;
    }
    let Ok(syn::Expr::Match(minus)) = syn::parse_str::<syn::Expr>(minus_sub) else {
        return None;
    };
    let Ok(syn::Expr::Match(plus)) = syn::parse_str::<syn::Expr>(plus_body) else {
        return None;
    };
    let ph_map: HashMap<&str, &str> = placeholders
        .iter()
        .map(|(name, ph)| (ph.as_str(), name.as_str()))
        .collect();
    let minus_scrut = crate::matcher_general::render(&*minus.expr);
    let plus_scrut = crate::matcher_general::render(&*plus.expr);
    if normalize_placeholders(&minus_scrut, &ph_map) != normalize_placeholders(&plus_scrut, &ph_map)
    {
        return None;
    }
    if minus.arms.len() != plus.arms.len() {
        return None;
    }
    let mut diff: Option<(String, String)> = None;
    for (ma, pa) in minus.arms.iter().zip(plus.arms.iter()) {
        let mn = normalize_placeholders(&quote::quote!(#ma).to_string(), &ph_map);
        let pn = normalize_placeholders(&quote::quote!(#pa).to_string(), &ph_map);
        if mn == pn {
            continue;
        }
        let (old_id, old_mut) = single_ident_binder(&ma.pat)?;
        let (new_id, new_mut) = single_ident_binder(&pa.pat)?;
        if old_id == new_id || old_mut != new_mut {
            return None;
        }
        match (&ma.guard, &pa.guard) {
            (None, None) => {}
            (Some((_, mg)), Some((_, pg))) => {
                let mgs = crate::matcher_general::render(&**mg);
                let pgs = crate::matcher_general::render(&**pg);
                if normalize_placeholders(&mgs, &ph_map) != normalize_placeholders(&pgs, &ph_map) {
                    return None;
                }
            }
            _ => return None,
        }
        let minus_arm_body = crate::matcher_general::render(&*ma.body);
        let plus_arm_body = crate::matcher_general::render(&*pa.body);
        if normalize_placeholders(&minus_arm_body, &ph_map)
            != normalize_placeholders(&plus_arm_body, &ph_map)
        {
            return None;
        }
        if diff.replace((old_id, new_id)).is_some() {
            return None;
        }
    }
    diff
}

/// Report-only orphans for the general-statement binder-rename shape.
/// Empty for every other shape (never alters existing verdicts).
pub fn general_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Vec<crate::OrphanReport> {
    if let Some((old, _)) = let_binder_rename(&compiled.kind) {
        return let_binder_orphans(states, file_edits, path, edits_before, &old);
    }
    // WI-2c: closure-param / match-arm binder renames. Scope-interior
    // scan only (see `closure_or_arm_orphans`): outside uses can never
    // dangle, so the file-wide collector would over-report.
    if let Some((old, _)) = closure_param_rename(&compiled.kind) {
        return crate::use_graph_scope::closure_or_arm_orphans(
            states,
            file_edits,
            path,
            edits_before,
            &old,
            true,
        );
    }
    if let Some((old, _)) = match_arm_binder_rename(&compiled.kind) {
        return crate::use_graph_scope::closure_or_arm_orphans(
            states,
            file_edits,
            path,
            edits_before,
            &old,
            false,
        );
    }
    // WI-2d: `for` / `if let` / `while let` binders (empty otherwise).
    if crate::use_graph_binder::is_wi2d_expr_binder_rename(compiled) {
        return crate::use_graph_scope::wi2d_orphans_for_compiled(
            states,
            file_edits,
            path,
            edits_before,
            compiled,
        );
    }
    Vec::new()
}

/// WI-2a file-wide orphan scan for `let`-binder renames (block scope:
/// uses after the binder anywhere in the file may dangle).
fn let_binder_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    old: &str,
) -> Vec<crate::OrphanReport> {
    let renamed: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    let mut refs: Vec<(String, &syn::File, &str, &crate::lines::LineIndex)> = Vec::new();
    let mut paths: Vec<&String> = states.keys().collect();
    paths.sort();
    for p in paths {
        if let Some(st) = states.get(p) {
            if let Some(parsed) = st.parsed.as_ref() {
                refs.push((p.clone(), parsed, st.original.as_str(), &st.index));
            }
        }
    }
    let mut out = collect_orphans(&refs, old, path, &renamed, false);
    // Inline format-string captures (`println!("{total}")`) hide the
    // ident inside a string literal — no `Ident` token for the path
    // collector above. Scan macro interiors for `{old}` captures and
    // report them as `macro-hidden` (safe direction: downgrade).
    for (fpath, parsed, original, index) in &refs {
        for line in format_capture_lines(parsed, old) {
            out.push(crate::OrphanReport {
                file: (*fpath).clone(),
                line,
                text: orphan_line_text(original, index, line),
                reason: "macro-hidden".to_string(),
            });
        }
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}

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
                for off in pat_old_offsets(inp, self.old, self.original, self.index) {
                    self.closure_params.push(off);
                }
            }
            syn::visit::visit_expr_closure(self, node);
        }
        fn visit_local(&mut self, node: &'ast syn::Local) {
            for off in pat_old_offsets(&node.pat, self.old, self.original, self.index) {
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

    // Unhandled same-name binders (for-loop pats, match arms, fn params
    // outside nested fns, …) veto completion — safe downgrade instead of
    // a mis-resolved rewrite.
    for o in &scopes.all_pats {
        if *o <= binder_end || !within(enc, *o) || is_outer(*o) || in_excluded(*o) {
            continue;
        }
        if inner_lets.contains(o) || inner_closure_params.contains(o) {
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

/// Lines of macros whose string literals capture `{old}` inline
/// (`println!("{old}")`, `{old:?}`). `collect_orphans` sees only
/// `Ident` tokens; format captures live inside `Literal`s.
fn format_capture_lines(parsed: &syn::File, old: &str) -> Vec<u32> {
    use syn::visit::{self, Visit};
    struct Finder<'a> {
        old: &'a str,
        lines: Vec<u32>,
    }
    impl Finder<'_> {
        fn scan_tokens(&mut self, tokens: &proc_macro2::TokenStream, line: u32) {
            for t in tokens.clone() {
                match t {
                    proc_macro2::TokenTree::Group(g) => self.scan_tokens(&g.stream(), line),
                    proc_macro2::TokenTree::Literal(lit)
                        if literal_captures(&lit.to_string(), self.old) =>
                    {
                        self.lines.push(line);
                    }
                    _ => {}
                }
            }
        }
    }
    impl<'ast> Visit<'ast> for Finder<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            use syn::spanned::Spanned;
            let line = node.span().start().line as u32;
            self.scan_tokens(&node.tokens, line);
            visit::visit_macro(self, node);
        }
    }
    let mut finder = Finder {
        old,
        lines: Vec::new(),
    };
    finder.visit_file(parsed);
    finder.lines.sort_unstable();
    finder.lines.dedup();
    finder.lines
}

/// True when a string-literal token text contains an inline `{old}`
/// capture (next char after `old` is not an ident continue).
pub(crate) fn literal_captures(lit_text: &str, old: &str) -> bool {
    let needle = format!("{{{old}");
    let mut search: &str = lit_text;
    while let Some(pos) = search.find(needle.as_str()) {
        let after = search[pos + needle.len()..].chars().next();
        match after {
            Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
                search = &search[pos + 1..];
            }
            _ => return true,
        }
    }
    false
}

pub(crate) fn orphan_line_text(
    original: &str,
    index: &crate::lines::LineIndex,
    line: u32,
) -> String {
    let Some(start) = index.line_start(line) else {
        return String::new();
    };
    let end = index.line_start(line + 1).unwrap_or(original.len());
    original
        .get(start..end)
        .unwrap_or("")
        .trim_end_matches(['\n', '\r'])
        .trim()
        .to_string()
}
/// Byte starts of every `old` ident bound by `pat` (destructuring
/// walked structurally). Shared by the WI-2b completion scopes and the
/// WI-2c interior orphan scan.
pub(crate) fn pat_old_offsets(
    pat: &syn::Pat,
    old: &str,
    original: &str,
    index: &LineIndex,
) -> Vec<usize> {
    fn walk(pat: &syn::Pat, old: &str, original: &str, index: &LineIndex, out: &mut Vec<usize>) {
        match pat {
            syn::Pat::Ident(pid) => {
                if pid.ident == old {
                    let sp = pid.ident.span();
                    let (Some(a), Some(b)) = (
                        index.offset(sp.start().line as u32, sp.start().column),
                        index.offset(sp.end().line as u32, sp.end().column),
                    ) else {
                        return;
                    };
                    if original.get(a..b) == Some(old) {
                        out.push(a);
                    }
                }
                if let Some((_, sub)) = &pid.subpat {
                    walk(sub, old, original, index, out);
                }
            }
            syn::Pat::Type(pt) => walk(&pt.pat, old, original, index, out),
            syn::Pat::Reference(pr) => walk(&pr.pat, old, original, index, out),
            syn::Pat::Paren(pp) => walk(&pp.pat, old, original, index, out),
            syn::Pat::Slice(ps) => {
                for e in &ps.elems {
                    walk(e, old, original, index, out);
                }
            }
            syn::Pat::Tuple(pt) => {
                for e in &pt.elems {
                    walk(e, old, original, index, out);
                }
            }
            syn::Pat::TupleStruct(pts) => {
                for e in &pts.elems {
                    walk(e, old, original, index, out);
                }
            }
            syn::Pat::Struct(pst) => {
                for f in &pst.fields {
                    walk(&f.pat, old, original, index, out);
                }
            }
            syn::Pat::Or(po) => {
                for c in &po.cases {
                    walk(c, old, original, index, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(pat, old, original, index, &mut out);
    out
}

/// Single `x` binder (no subpattern); returns `(ident, has_mut)`.
pub(crate) fn single_ident_binder(pat: &syn::Pat) -> Option<(String, bool)> {
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
        syn::Pat::Type(pt) => single_ident_binder(&pt.pat),
        _ => None,
    }
}

/// Type annotation tokens of a `let` pattern (`""` when unannotated).
fn normalize_ty(pat: &syn::Pat, ph_map: &HashMap<&str, &str>) -> String {
    let ty = match pat {
        syn::Pat::Type(pt) => Some(&*pt.ty),
        _ => None,
    };
    match ty {
        None => String::new(),
        Some(t) => {
            let s = quote::quote!(#t).to_string();
            normalize_placeholders(&s, ph_map)
        }
    }
}

fn init_tokens(expr: &syn::Expr) -> String {
    quote::quote!(#expr).to_string()
}

pub(crate) use crate::use_graph_normalize::normalize_placeholders;

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_pattern::{CompiledPattern, PatternKind};

    fn general_stmt(minus_sub: &str, plus_body: &str) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Stmt,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders: vec![("E".to_string(), "__RSMPL_E0".to_string())],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }

    fn compiled(kind: PatternKind) -> CompiledPattern {
        CompiledPattern {
            kind,
            table: Default::default(),
            minus_bindings: Default::default(),
            plus_bindings: Default::default(),
            unbound_plus: Vec::new(),
            repeated: Vec::new(),
            root: None,
        }
    }

    #[test]
    fn plain_binder_rename_fires() {
        let kind = general_stmt("let total = __RSMPL_E0;", "let sum = E;");
        assert_eq!(
            let_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
        assert!(is_let_binder_rename(&compiled(kind)));
    }

    #[test]
    fn concrete_init_same_fires() {
        let mut kind = general_stmt("let total = 1;", "let sum = 1;");
        if let PatternKind::General { placeholders, .. } = &mut kind {
            placeholders.clear();
        }
        assert_eq!(
            let_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
    }

    #[test]
    fn init_change_vetoes() {
        let kind = general_stmt("let total = __RSMPL_E0;", "let sum = 2;");
        assert_eq!(let_binder_rename(&kind), None);
    }

    #[test]
    fn same_name_vetoes() {
        let kind = general_stmt("let total = __RSMPL_E0;", "let total = E;");
        assert_eq!(let_binder_rename(&kind), None);
    }

    #[test]
    fn mut_mismatch_vetoes() {
        let kind = general_stmt("let mut total = __RSMPL_E0;", "let sum = E;");
        assert_eq!(let_binder_rename(&kind), None);
    }

    #[test]
    fn destructuring_vetoes() {
        let kind = general_stmt("let (a, b) = __RSMPL_E0;", "let (a, c) = E;");
        assert_eq!(let_binder_rename(&kind), None);
    }

    #[test]
    fn type_change_vetoes() {
        let kind = general_stmt("let total : u32 = __RSMPL_E0;", "let sum : u64 = E;");
        assert_eq!(let_binder_rename(&kind), None);
    }

    #[test]
    fn annotated_same_fires() {
        let kind = general_stmt("let total : u32 = __RSMPL_E0;", "let sum : u32 = E;");
        assert_eq!(
            let_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
    }

    #[test]
    fn inline_format_capture_detected() {
        assert!(literal_captures("\"{total}\"", "total"));
        assert!(literal_captures("\"{total:?}\"", "total"));
        assert!(!literal_captures("\"{totalx}\"", "total"));
        assert!(!literal_captures("\"hello\"", "total"));
    }

    #[test]
    fn format_capture_line_found_in_macro() {
        let src = "fn f() {\n    let total = 1;\n    println!(\"{total}\");\n}\n";
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let lines = format_capture_lines(&parsed, "total");
        assert_eq!(lines, vec![3]);
    }

    #[test]
    fn general_orphans_lists_leftover_uses() {
        use std::collections::HashMap;
        let src = "fn main() {\n    let total = 1;\n    let x = total + total;\n}\n";
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        let binder_off = src.find("let total").unwrap() + 4;
        let mut states = HashMap::new();
        states.insert(
            "src/main.rs".to_string(),
            crate::lines::FileState {
                original: src.to_string(),
                index,
                parsed: Some(parsed),
            },
        );
        let mut file_edits = HashMap::new();
        file_edits.insert(
            "src/main.rs".to_string(),
            vec![crate::edits::TextEdit {
                start: binder_off,
                end: binder_off + "total".len(),
                replacement: "sum".to_string(),
                label: None,
            }],
        );
        let kind = general_stmt("let total = 1;", "let sum = 1;");
        let mut kind = kind;
        if let PatternKind::General { placeholders, .. } = &mut kind {
            placeholders.clear();
        }
        let compiled = compiled(kind);
        let orphans = general_orphans(&states, &file_edits, "src/main.rs", 0, &compiled);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].line, 3);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn general_orphans_empty_when_no_uses() {
        use std::collections::HashMap;
        let src = "fn main() {\n    let total = 1;\n}\n";
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        let binder_off = src.find("let total").unwrap() + 4;
        let mut states = HashMap::new();
        states.insert(
            "src/main.rs".to_string(),
            crate::lines::FileState {
                original: src.to_string(),
                index,
                parsed: Some(parsed),
            },
        );
        let mut file_edits = HashMap::new();
        file_edits.insert(
            "src/main.rs".to_string(),
            vec![crate::edits::TextEdit {
                start: binder_off,
                end: binder_off + "total".len(),
                replacement: "sum".to_string(),
                label: None,
            }],
        );
        let kind = general_stmt("let total = 1;", "let sum = 1;");
        let mut kind = kind;
        if let PatternKind::General { placeholders, .. } = &mut kind {
            placeholders.clear();
        }
        let compiled = compiled(kind);
        let orphans = general_orphans(&states, &file_edits, "src/main.rs", 0, &compiled);
        assert!(orphans.is_empty());
    }

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
    fn follow_aborts_on_for_loop_shadow() {
        let src = "fn main() {\n    let total = 1;\n    for total in 0..3 {\n        let x = total;\n    }\n}\n";
        assert!(follow_edits_for(src).is_empty());
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

    // WI-2c: closure-param / match-arm binder evidence (orphans +
    // downgrade, no completion).

    fn general_expr(minus_sub: &str, plus_body: &str) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Expr,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders: vec![("E".to_string(), "__RSMPL_E0".to_string())],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }

    fn general_expr2(
        minus_sub: &str,
        plus_body: &str,
        placeholders: Vec<(String, String)>,
    ) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Expr,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders,
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }

    #[test]
    fn closure_param_plain_fires() {
        let kind = general_expr("|total| __RSMPL_E0", "|sum| E");
        assert_eq!(
            closure_param_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
        let c = compiled(kind);
        assert!(is_local_binder_rename(&c));
        assert!(!is_let_binder_rename(&c));
    }

    #[test]
    fn closure_param_annotated_fires() {
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum : u32| E");
        assert_eq!(
            closure_param_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
    }

    #[test]
    fn closure_param_annotation_change_vetoes() {
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum : u64| E");
        assert_eq!(closure_param_rename(&kind), None);
        // Bare `P` never drops an annotation.
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum| E");
        assert_eq!(closure_param_rename(&kind), None);
    }

    #[test]
    fn closure_param_body_change_vetoes() {
        let kind = general_expr("|total| __RSMPL_E0", "|sum| E + 1");
        assert_eq!(closure_param_rename(&kind), None);
    }

    #[test]
    fn closure_param_static_vetoes() {
        // `movability` is syn's `static` qualifier; a delta there is a
        // different closure.
        let kind = general_expr("|total| __RSMPL_E0", "static |sum| E");
        assert_eq!(closure_param_rename(&kind), None);
    }

    #[test]
    fn closure_param_move_delta_vetoes() {
        // `capture` is syn's `move` qualifier; a delta there is a
        // different closure (engine `match_closure` parity).
        let kind = general_expr("|total| __RSMPL_E0", "move |sum| E");
        assert_eq!(closure_param_rename(&kind), None);
        let kind = general_expr("move |total| __RSMPL_E0", "|sum| E");
        assert_eq!(closure_param_rename(&kind), None);
    }

    #[test]
    fn closure_param_two_binders_veto() {
        let kind = general_expr("|total, other| __RSMPL_E0", "|sum, changed| E");
        assert_eq!(closure_param_rename(&kind), None);
        let kind = general_expr("|total| __RSMPL_E0", "|sum, x| E");
        assert_eq!(closure_param_rename(&kind), None);
    }

    #[test]
    fn closure_param_non_closure_vetoes() {
        let kind = general_expr("total + __RSMPL_E0", "sum + E");
        assert_eq!(closure_param_rename(&kind), None);
    }

    fn match_placeholders() -> Vec<(String, String)> {
        vec![
            ("E".to_string(), "__RSMPL_E0".to_string()),
            ("B".to_string(), "__RSMPL_E1".to_string()),
        ]
    }

    #[test]
    fn match_arm_plain_fires() {
        let kind = general_expr2(
            "match __RSMPL_E0 { total => __RSMPL_E1, _ => 0 }",
            "match E { sum => B, _ => 0 }",
            match_placeholders(),
        );
        assert_eq!(
            match_arm_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
        let c = compiled(kind);
        assert!(is_local_binder_rename(&c));
        assert!(!is_let_binder_rename(&c));
    }

    #[test]
    fn match_arm_scrutinee_change_vetoes() {
        let kind = general_expr2(
            "match __RSMPL_E0 { total => __RSMPL_E1, _ => 0 }",
            "match E { sum => B, _ => 1 }",
            match_placeholders(),
        );
        assert_eq!(match_arm_binder_rename(&kind), None);
    }

    #[test]
    fn match_arm_count_vetoes() {
        let kind = general_expr2(
            "match __RSMPL_E0 { total => __RSMPL_E1, _ => 0 }",
            "match E { sum => B, _ => 0, _ => 1 }",
            match_placeholders(),
        );
        assert_eq!(match_arm_binder_rename(&kind), None);
    }

    #[test]
    fn match_arm_guard_change_vetoes() {
        let kind = general_expr(
            "match x { total if y => 1, _ => 0 }",
            "match x { sum if z => 1, _ => 0 }",
        );
        assert_eq!(match_arm_binder_rename(&kind), None);
        // Added guard vetoes too.
        let kind = general_expr(
            "match x { total => 1, _ => 0 }",
            "match x { sum if y => 1, _ => 0 }",
        );
        assert_eq!(match_arm_binder_rename(&kind), None);
    }

    #[test]
    fn match_arm_body_change_vetoes() {
        let kind = general_expr2(
            "match __RSMPL_E0 { total => __RSMPL_E1, _ => 0 }",
            "match E { sum => B + 1, _ => 0 }",
            match_placeholders(),
        );
        assert_eq!(match_arm_binder_rename(&kind), None);
    }

    #[test]
    fn match_arm_destructuring_vetoes() {
        let kind = general_expr2(
            "match __RSMPL_E0 { Some (total) => __RSMPL_E1, _ => 0 }",
            "match E { Some (sum) => B, _ => 0 }",
            match_placeholders(),
        );
        assert_eq!(match_arm_binder_rename(&kind), None);
    }

    #[test]
    fn match_arm_two_binders_vetoes() {
        let kind = general_expr2(
            "match __RSMPL_E0 { total => __RSMPL_E1, other => __RSMPL_E1 }",
            "match E { sum => B, changed => B }",
            match_placeholders(),
        );
        assert_eq!(match_arm_binder_rename(&kind), None);
    }

    #[test]
    fn match_arm_non_match_vetoes() {
        let kind = general_expr("|total| __RSMPL_E0", "|sum| E");
        assert_eq!(match_arm_binder_rename(&kind), None);
    }

    fn first_closure_span(src: &str) -> (usize, usize) {
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        struct Find<'a> {
            original: &'a str,
            index: &'a crate::lines::LineIndex,
            out: Vec<(usize, usize)>,
        }
        impl<'ast> syn::visit::Visit<'ast> for Find<'_> {
            fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
                use syn::spanned::Spanned;
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    self.out.push(ab);
                }
                syn::visit::visit_expr_closure(self, node);
            }
        }
        let mut find = Find {
            original: src,
            index: &index,
            out: Vec::new(),
        };
        use syn::visit::Visit;
        find.visit_file(&parsed);
        find.out.into_iter().next().expect("test src has a closure")
    }

    fn first_match_span(src: &str) -> (usize, usize) {
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        struct Find<'a> {
            original: &'a str,
            index: &'a crate::lines::LineIndex,
            out: Vec<(usize, usize)>,
        }
        impl<'ast> syn::visit::Visit<'ast> for Find<'_> {
            fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
                use syn::spanned::Spanned;
                if let Some(ab) =
                    crate::matcher_item::span_bytes(self.original, self.index, node.span())
                {
                    self.out.push(ab);
                }
                syn::visit::visit_expr_match(self, node);
            }
        }
        let mut find = Find {
            original: src,
            index: &index,
            out: Vec::new(),
        };
        use syn::visit::Visit;
        find.visit_file(&parsed);
        find.out.into_iter().next().expect("test src has a match")
    }

    fn wi2c_orphans(
        src: &str,
        kind: PatternKind,
        span: (usize, usize),
    ) -> Vec<crate::OrphanReport> {
        use std::collections::HashMap;
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        let mut states = HashMap::new();
        states.insert(
            "src/main.rs".to_string(),
            crate::lines::FileState {
                original: src.to_string(),
                index,
                parsed: Some(parsed),
            },
        );
        let mut file_edits = HashMap::new();
        file_edits.insert(
            "src/main.rs".to_string(),
            vec![crate::edits::TextEdit {
                start: span.0,
                end: span.1,
                replacement: String::new(),
                label: None,
            }],
        );
        general_orphans(&states, &file_edits, "src/main.rs", 0, &compiled(kind))
    }

    #[test]
    fn closure_orphans_lists_body_uses() {
        let src = "fn main() {\n    let f = |total: u32| total + total;\n    let _ = f;\n}\n";
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum : u32| E");
        let orphans = wi2c_orphans(src, kind, first_closure_span(src));
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].line, 2);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn closure_orphans_empty_when_no_uses() {
        let src = "fn main() {\n    let f = |total: u32| 1;\n    let _ = f;\n}\n";
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum : u32| E");
        assert!(wi2c_orphans(src, kind, first_closure_span(src)).is_empty());
    }

    #[test]
    fn closure_orphans_ignore_outer_scope() {
        // `total` outside the closure can never dangle: the file-wide
        // scan would flag lines 2 and 4, the scoped scan flags nothing.
        let src = "fn main() {\n    let total = 5;\n    let f = |total: u32| 1;\n    let y = total;\n    let _ = (f, y);\n}\n";
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum : u32| E");
        assert!(wi2c_orphans(src, kind, first_closure_span(src)).is_empty());
    }

    #[test]
    fn closure_orphans_inner_let_shadowed() {
        let src =
            "fn main() {\n    let f = |total: u32| { let total = 2; total };\n    let _ = f;\n}\n";
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum : u32| E");
        let orphans = wi2c_orphans(src, kind, first_closure_span(src));
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].reason, "shadowed");
    }

    #[test]
    fn closure_orphans_macro_and_capture_hidden() {
        let src = "fn main() {\n    let f = |total: u32| { println!(\"{total}\"); println!(\"{}\", total); 0 };\n    let _ = f;\n}\n";
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum : u32| E");
        let orphans = wi2c_orphans(src, kind, first_closure_span(src));
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].line, 2);
        assert_eq!(orphans[0].reason, "macro-hidden");
    }

    #[test]
    fn match_arm_orphans_lists_arm_body_use() {
        let src = "fn main() {\n    let v = match Some(1) { total => total + 1, _ => 0 };\n    let _ = v;\n}\n";
        let kind = general_expr2(
            "match __RSMPL_E0 { total => __RSMPL_E1, _ => 0 }",
            "match E { sum => B, _ => 0 }",
            match_placeholders(),
        );
        let orphans = wi2c_orphans(src, kind, first_match_span(src));
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].line, 2);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn match_arm_orphans_ignore_scrutinee_and_other_arms() {
        // Scrutinee `total` names the outer binding; the renamed arm
        // carries no use — the binder line itself is never an orphan.
        let src = "fn main() {\n    let total = 9;\n    let v = match total { total => 1, _ => 0 };\n    let _ = v;\n}\n";
        let kind = general_expr(
            "match __RSMPL_E0 { total => 1, _ => 0 }",
            "match E { sum => 1, _ => 0 }",
        );
        assert!(wi2c_orphans(src, kind, first_match_span(src)).is_empty());
    }

    #[test]
    fn match_arm_orphans_guard_use_listed() {
        let src = "fn main() {\n    let v = match Some(1) { total if total > 0 => 1, _ => 0 };\n    let _ = v;\n}\n";
        let kind = general_expr2(
            "match __RSMPL_E0 { total if __RSMPL_E1 => 1, _ => 0 }",
            "match E { sum if B => 1, _ => 0 }",
            match_placeholders(),
        );
        // Detector check first: guard preserved via hole, binder-only diff.
        assert_eq!(
            match_arm_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
        let orphans = wi2c_orphans(src, kind, first_match_span(src));
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].reason, "off-target");
    }

    fn apply_report(
        patch_text: &str,
        file_text: &str,
    ) -> Result<(String, crate::ApplyReport), String> {
        use std::collections::HashMap;
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

    const WI2C_HEAD: &str =
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";

    #[test]
    fn closure_rename_leftover_downgrades() {
        let patch = format!(
            "{WI2C_HEAD}@@ -2,1 @@ rule=rename_closure\n# lang=rust\n# expr E\n- |total: u32| E\n+ |sum: u32| E\n"
        );
        let src = "fn main() {\n    let f = |total: u32| total + total;\n    let _ = f;\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("|sum: u32| total + total"), "got: {out}");
        assert_eq!(report.rules[0].semantics, crate::RuleSemantics::ReparseOnly);
        assert_eq!(report.rules[0].orphans.len(), 1);
        assert_eq!(report.rules[0].orphans[0].reason, "off-target");
    }

    #[test]
    fn closure_rename_clean_stays_instantiated() {
        let patch = format!(
            "{WI2C_HEAD}@@ -2,1 @@ rule=rename_closure\n# lang=rust\n# expr E\n- |total: u32| E\n+ |sum: u32| E\n"
        );
        let src = "fn main() {\n    let f = |total: u32| 1;\n    let _ = f;\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("|sum: u32| 1"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(report.rules[0].orphans.is_empty());
    }

    #[test]
    fn match_arm_rename_leftover_downgrades() {
        let patch = format!(
            "{WI2C_HEAD}@@ -2,1 @@ rule=rename_arm\n# lang=rust\n# expr E\n# expr B\n- match E {{ total => B, _ => 0 }}\n+ match E {{ sum => B, _ => 0 }}\n"
        );
        let src = "fn main() {\n    let v = match Some(1) { total => total + 1, _ => 0 };\n    let _ = v;\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("sum => total + 1"), "got: {out}");
        assert_eq!(report.rules[0].semantics, crate::RuleSemantics::ReparseOnly);
        assert_eq!(report.rules[0].orphans.len(), 1);
    }
}
