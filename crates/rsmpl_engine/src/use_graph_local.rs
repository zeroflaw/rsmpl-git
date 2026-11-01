//! WI-2a/2b: `let`-binder rename honesty + block completion.
//!
//! WI-2a detects the single-binder rename shape (`let old = E;` → `let
//! new = E;`, same init, same type annotation); the leftover-use orphan
//! scan lives in [`crate::use_graph_local_orphan`] (enclosing-block
//! gated). WI-2b additionally rewrites block-scoped value uses in
//! the same hunk (labelled `follow`); shadowing, nested-fn, and macro
//! interiors stay evidence (orphans + downgrade), never rewritten.
//!
//! Member/assoc orphan dispatch lives with its collector:
//! [`crate::use_graph_member::member_orphans`],
//! [`crate::use_graph_assoc::assoc_orphans`].

use std::collections::HashMap;

use rsmpl_pattern::{GeneralShape, PatternKind};

use crate::edits::TextEdit;
use crate::lines::{FileState, LineIndex};

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
        return crate::use_graph_local_orphan::let_binder_orphans(
            states,
            file_edits,
            path,
            edits_before,
            &old,
        );
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

/// WI-2b follow completion lives in [`crate::use_graph_local_follow`]
/// (D5 split; re-exported here so existing paths keep working).
pub use crate::use_graph_local_follow::let_binder_follow_edits;

/// Lines of macros whose string literals capture `{old}` inline
/// (`println!("{old}")`, `{old:?}`). `collect_orphans` sees only
/// `Ident` tokens; format captures live inside `Literal`s.
pub(crate) fn format_capture_lines(parsed: &syn::File, old: &str) -> Vec<u32> {
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

    #[test]
    fn general_orphans_excludes_pre_binder_use() {
        use std::collections::HashMap;
        let src = "fn main() {\n    let y = total;\n    let total = 1;\n    let z = total;\n}\n";
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
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 4);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn general_orphans_excludes_binder_init_rhs() {
        use std::collections::HashMap;
        let src = "fn main() {\n    let total = total;\n}\n";
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
        assert!(orphans.is_empty(), "got: {orphans:?}");
    }

    #[test]
    fn general_orphans_ignores_other_files() {
        use std::collections::HashMap;
        let src = "fn main() {\n    let total = 1;\n    let z = total;\n}\n";
        let other = "fn other() {\n    let y = total;\n}\n";
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let other_parsed: syn::File = syn::parse_str(other).unwrap();
        let binder_off = src.find("let total").unwrap() + 4;
        let mut states = HashMap::new();
        states.insert(
            "src/main.rs".to_string(),
            crate::lines::FileState {
                original: src.to_string(),
                index: crate::lines::LineIndex::new(src),
                parsed: Some(parsed),
            },
        );
        states.insert(
            "src/other.rs".to_string(),
            crate::lines::FileState {
                original: other.to_string(),
                index: crate::lines::LineIndex::new(other),
                parsed: Some(other_parsed),
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
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].file, "src/main.rs");
        assert_eq!(orphans[0].line, 3);
    }

    fn orphans_for_src(src: &str) -> Vec<crate::OrphanReport> {
        use std::collections::HashMap;
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
        let mut kind = general_stmt("let total = 1;", "let sum = 1;");
        if let PatternKind::General { placeholders, .. } = &mut kind {
            placeholders.clear();
        }
        let compiled = compiled(kind);
        general_orphans(&states, &file_edits, "src/main.rs", 0, &compiled)
    }

    #[test]
    fn general_orphans_ignores_other_fn_same_file() {
        // `bar`'s `total` names a different scope — not an orphan.
        let src = "fn foo() {\n    let total = 1;\n}\nfn bar() {\n    let y = total;\n}\n";
        let orphans = orphans_for_src(src);
        assert!(orphans.is_empty(), "got: {orphans:?}");
    }

    #[test]
    fn general_orphans_keeps_in_scope_use_beside_other_fn() {
        // In-scope leftover stays even when another fn shares the name.
        let src = "fn foo() {\n    let total = 1;\n    let x = total;\n}\nfn bar() {\n    let y = total;\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 3);
    }

    #[test]
    fn general_orphans_ignores_nested_fn_use() {
        // Nested fns cannot capture outer locals — unrelated scope.
        let src = "fn main() {\n    let total = 1;\n    fn inner() -> u32 { total }\n    let g = total;\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 4);
    }

    #[test]
    fn general_orphans_keeps_shadowed_inner_let_use() {
        // Shadowed uses stay orphans (no over-narrowing), labelled
        // `shadowed` (WI-2b reason follow-up).
        let src = "fn main() {\n    let total = 1;\n    {\n        let total = 2;\n        let b = total;\n    }\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 5);
        assert_eq!(orphans[0].reason, "shadowed");
    }

    #[test]
    fn general_orphans_shadowed_closure_param_use() {
        // An inner closure-param `total` shadows the outer `let total`:
        // the closure-body use stays a `shadowed` orphan.
        let src =
            "fn main() {\n    let total = 1;\n    let f = |total: u32| total + 1;\n    let _ = f;\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "shadowed");
    }

    #[test]
    fn general_orphans_unshadowed_after_inner_block_stays_off_target() {
        // The inner `let total` governs only its block: the outer use
        // after the block still names the outer binder (`off-target`).
        let src = "fn main() {\n    let total = 1;\n    {\n        let total = 2;\n    }\n    let c = total;\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 6);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn general_orphans_macro_scope_precision() {
        // Macro in the enclosing block stays; macro in another fn drops.
        let same = "fn main() {\n    let total = 1;\n    println!(\"{}\", total);\n}\n";
        let orphans = orphans_for_src(same);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 3);
        let other =
            "fn foo() {\n    let total = 1;\n}\nfn bar() {\n    println!(\"{}\", total);\n}\n";
        let orphans = orphans_for_src(other);
        assert!(orphans.is_empty(), "got: {orphans:?}");
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
