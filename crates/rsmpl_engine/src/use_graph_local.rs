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
    // mapping `__RSMPL_*` placeholders back to their names).
    let minus_init = minus_local.init.as_ref().map(|i| init_tokens(&i.expr))?;
    let plus_init = plus_local.init.as_ref().map(|i| init_tokens(&i.expr))?;
    if normalize_placeholders(&minus_init, &ph_map) != plus_init {
        return None;
    }
    Some((old_id, new_id))
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
    let Some((old, _)) = let_binder_rename(&compiled.kind) else {
        return Vec::new();
    };
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
    let mut out = collect_orphans(&refs, &old, path, &renamed, false);
    // Inline format-string captures (`println!("{total}")`) hide the
    // ident inside a string literal — no `Ident` token for the path
    // collector above. Scan macro interiors for `{old}` captures and
    // report them as `macro-hidden` (safe direction: downgrade).
    for (fpath, parsed, original, index) in &refs {
        for line in format_capture_lines(parsed, old.as_str()) {
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

    fn pat_old_offsets(pat: &syn::Pat, old: &str, original: &str, index: &LineIndex) -> Vec<usize> {
        fn walk(
            pat: &syn::Pat,
            old: &str,
            original: &str,
            index: &LineIndex,
            out: &mut Vec<usize>,
        ) {
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
        enc: (usize, usize),
        binder_end: usize,
        item_fns: &'b [(usize, usize)],
        macros: &'b [(usize, usize)],
        shadows: &'b [(usize, usize)],
        out: Vec<(usize, usize)>,
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
                    if self.original.get(a..b) == Some(self.old)
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
        // Macro interiors stay evidence (opaque, never rewritten).
        fn visit_macro(&mut self, _node: &'ast syn::Macro) {}
    }

    let mut uses = Uses {
        original,
        index,
        old,
        enc,
        binder_end,
        item_fns: &nested_fns,
        macros: &scopes.macros,
        shadows: &shadows,
        out: Vec::new(),
    };
    uses.visit_file(parsed);
    uses.out.sort_unstable();
    uses.out.dedup();
    uses.out
        .into_iter()
        .map(|(a, b)| TextEdit::new(a, b, new.to_string()).with_label("follow"))
        .collect()
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
fn literal_captures(lit_text: &str, old: &str) -> bool {
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

fn orphan_line_text(original: &str, index: &crate::lines::LineIndex, line: u32) -> String {
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
/// Single `x` binder (no subpattern); returns `(ident, has_mut)`.
fn single_ident_binder(pat: &syn::Pat) -> Option<(String, bool)> {
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

fn normalize_placeholders(s: &str, ph_map: &HashMap<&str, &str>) -> String {
    let mut out = s.to_string();
    for (ph, name) in ph_map {
        out = out.replace(ph, name);
    }
    out
}

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
}
