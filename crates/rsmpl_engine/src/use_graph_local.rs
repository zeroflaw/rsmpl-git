//! WI-2a: `let`-binder rename honesty — orphan evidence + downgrade.
//!
//! First targeted slice of IMPROVE.md WI-2 (no auto-rewrite yet). Detects
//! the single-binder rename shape (`let old = E;` → `let new = E;`, same
//! init, same type annotation) and lists leftover same-name uses via the
//! proven [`crate::use_graph_orphan::collect_orphans`] path collector.
//! File-wide by design (safe direction: over-report, never silent
//! `verified`). Block-scoped rewriting is WI-2b.

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
}
