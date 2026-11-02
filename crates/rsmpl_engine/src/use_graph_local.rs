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

use rsmpl_pattern::PatternKind;

use crate::edits::TextEdit;
use crate::lines::{FileState, LineIndex};

/// True when `compiled` is a plain single-binder `let` rename.
pub fn is_let_binder_rename(compiled: &rsmpl_pattern::CompiledPattern) -> bool {
    let_binder_rename(&compiled.kind).is_some()
}

/// WI-3: true when `compiled` renames exactly one local binder (any
/// spelling — `let`, closure-param, match-arm, `for` / `if let` /
/// `while let`, let-wrapped closure, via the single pattern identity).
/// The downgrade gate uses this: any leftover use forces `reparse-only`
/// (global `partial`).
pub fn is_local_binder_rename(compiled: &rsmpl_pattern::CompiledPattern) -> bool {
    rsmpl_pattern::binder::binder_rename(&compiled.kind).is_some()
}

/// `(old, new)` binder idents when `kind` is a plain `let old = E;` →
/// `let new = E;` rename — or the single-leaf destructuring twin
/// (`let (a, old) = E;` → `let (a, new) = E;`, struct/shorthand/slice
/// wrapper spellings, exactly one leaf differs) — with identical init
/// and type annotation. Anything else (multi-leaf, init/type change,
/// `let-else`, attribute delta) yields `None` — today's behaviour is
/// kept.
///
/// WI-3: delegates to the single pattern identity
/// ([`rsmpl_pattern::binder::binder_rename`]); the scope filter keeps
/// the `Let`-only contract.
pub fn let_binder_rename(kind: &PatternKind) -> Option<(String, String)> {
    match rsmpl_pattern::binder::binder_rename(kind) {
        Some(b) if b.scope == rsmpl_pattern::binder::BinderScope::Let => Some((b.old, b.new)),
        _ => None,
    }
}

/// WI-2c: `(old, new)` when `kind` is a closure-parameter rename.
/// WI-3: delegates to the single pattern identity; the scope filter
/// keeps the `Closure`-only contract (see the `BinderScope::Closure`
/// pins in `rsmpl_pattern::binder`).
pub fn closure_param_rename(kind: &PatternKind) -> Option<(String, String)> {
    match rsmpl_pattern::binder::binder_rename(kind) {
        Some(b) if b.scope == rsmpl_pattern::binder::BinderScope::Closure => Some((b.old, b.new)),
        _ => None,
    }
}

/// WI-2c: `(old, new)` when `kind` is a match-arm binder rename.
/// WI-3: delegates to the single pattern identity; the scope filter
/// keeps the `MatchArm`-only contract.
pub fn match_arm_binder_rename(kind: &PatternKind) -> Option<(String, String)> {
    match rsmpl_pattern::binder::binder_rename(kind) {
        Some(b) if b.scope == rsmpl_pattern::binder::BinderScope::MatchArm => Some((b.old, b.new)),
        _ => None,
    }
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
            compiled,
        );
    }
    // WI-3: let-wrapped closure binder (F3 headline). The outer `let`
    // name is unchanged, so the let scan stays silent; the init closure
    // body can still dangle. The whole-statement edit contains the
    // closure node, so the closure-interior scan claims it (WI-3
    // reversed containment).
    if let Some(b) = rsmpl_pattern::binder::binder_rename(&compiled.kind) {
        if b.scope == rsmpl_pattern::binder::BinderScope::LetClosure {
            return crate::use_graph_scope::closure_or_arm_orphans(
                states,
                file_edits,
                path,
                edits_before,
                &b.old,
                true,
            );
        }
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

/// Byte-precise spans of inline `{old}` captures: `(line, byte_offset)`
/// of the capturing literal. The line is the literal's own line (not
/// the macro start line), so multi-line macros attribute to the line
/// holding the capture; the offset lets callers gate on the enclosing
/// block byte-precisely instead of by line start. Unmappable spans fall
/// back to the macro start line with `usize::MAX` offset (callers keep
/// them — safe downgrade direction). `original`/`index` map spans to
/// bytes; pass `("", empty_index)` for line-only callers.
pub(crate) fn format_capture_spans(
    parsed: &syn::File,
    old: &str,
    original: &str,
    index: &crate::lines::LineIndex,
) -> Vec<(u32, usize)> {
    use syn::visit::{self, Visit};
    struct Finder<'a> {
        old: &'a str,
        original: &'a str,
        index: &'a crate::lines::LineIndex,
        out: Vec<(u32, usize)>,
    }
    impl Finder<'_> {
        fn scan_tokens(&mut self, tokens: &proc_macro2::TokenStream, fallback_line: u32) {
            for t in tokens.clone() {
                match t {
                    proc_macro2::TokenTree::Group(g) => {
                        self.scan_tokens(&g.stream(), fallback_line)
                    }
                    proc_macro2::TokenTree::Literal(lit)
                        if literal_captures(&lit.to_string(), self.old) =>
                    {
                        let sp = lit.span();
                        match (
                            self.index.offset(sp.start().line as u32, sp.start().column),
                            self.index.offset(sp.end().line as u32, sp.end().column),
                        ) {
                            (Some(a), Some(b))
                                if self.original.get(a..b).is_some()
                                    || self.original.is_empty() =>
                            {
                                self.out.push((sp.start().line as u32, a));
                            }
                            _ => {
                                self.out.push((fallback_line, usize::MAX));
                            }
                        }
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
        original,
        index,
        out: Vec::new(),
    };
    finder.visit_file(parsed);
    finder.out.sort_unstable();
    finder.out.dedup();
    finder.out
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

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_pattern::{CompiledPattern, GeneralShape, PatternKind};

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
    fn destructuring_single_leaf_fires() {
        // Residue slice 2: single-leaf destructuring completes like a
        // plain rename.
        let kind = general_stmt("let (a, b) = __RSMPL_E0;", "let (a, c) = E;");
        assert_eq!(
            let_binder_rename(&kind),
            Some(("b".to_string(), "c".to_string()))
        );
    }

    #[test]
    fn destructuring_multi_leaf_vetoes() {
        let kind = general_stmt("let (a, b) = __RSMPL_E0;", "let (c, d) = E;");
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
        let index = crate::lines::LineIndex::new(src);
        let spans = format_capture_spans(&parsed, "total", src, &index);
        assert_eq!(spans.len(), 1, "got: {spans:?}");
        assert_eq!(spans[0].0, 3, "got: {spans:?}");
    }

    #[test]
    fn format_capture_spans_reports_literal_line_and_offset() {
        // Multi-line macro: the macro starts on line 3 but the
        // capturing literal lives on line 4 — attribution must point
        // at the literal (byte-precise), not the macro start.
        let src = "fn f() {\n    let total = 1;\n    println!(\n        \"{total}\"\n    );\n}\n";
        let parsed: syn::File = syn::parse_str(src).unwrap();
        let index = crate::lines::LineIndex::new(src);
        let spans = format_capture_spans(&parsed, "total", src, &index);
        assert_eq!(spans.len(), 1, "got: {spans:?}");
        assert_eq!(spans[0].0, 4, "got: {spans:?}");
        let lit_off = src.find("\"{total}\"").unwrap();
        assert_eq!(spans[0].1, lit_off, "got: {spans:?}");
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
    fn general_orphans_shadowed_for_loop_body_use() {
        // An inner `for total` shadows the outer `let total` inside the
        // loop body: the body use stays a `shadowed` orphan.
        let src = "fn main() {\n    let total = 1;\n    for total in 0..3 {\n        let x = total;\n    }\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "shadowed");
    }

    #[test]
    fn general_orphans_unshadowed_after_for_loop_stays_off_target() {
        // The `for total` binder governs only the loop body: the outer
        // use after the loop still names the outer binder.
        let src = "fn main() {\n    let total = 1;\n    for total in 0..3 {\n        break;\n    }\n    let c = total;\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 6);
        assert_eq!(orphans[0].reason, "off-target");
    }

    #[test]
    fn general_orphans_shadowed_match_arm_use() {
        // An inner match-arm `total` shadows the outer `let total`
        // inside the arm body.
        let src = "fn main() {\n    let total = 1;\n    match 1 {\n        total => total,\n        _ => 0,\n    }\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "shadowed");
    }

    #[test]
    fn general_orphans_shadowed_if_let_then_use() {
        // An inner `if let total` shadows the outer `let total` inside
        // the then-branch: the then use stays a `shadowed` orphan.
        let src = "fn main() {\n    let total = 1;\n    if let total = Some(1) {\n        let x = total;\n    }\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "shadowed");
    }

    #[test]
    fn general_orphans_unshadowed_after_if_let_stays_off_target() {
        // The `if let total` binder governs only the then-branch: the
        // outer use after the whole `if`/`else` still names the outer
        // binder (`off-target`).
        let src = "fn main() {\n    let total = 1;\n    if let total = Some(1) {\n        let x = total;\n    } else {\n        let y = 0;\n    }\n    let c = total;\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 2, "got: {orphans:?}");
        assert!(
            orphans
                .iter()
                .any(|o| o.line == 4 && o.reason == "shadowed"),
            "got: {orphans:?}"
        );
        assert!(
            orphans
                .iter()
                .any(|o| o.line == 8 && o.reason == "off-target"),
            "got: {orphans:?}"
        );
    }

    #[test]
    fn general_orphans_shadowed_while_let_body_use() {
        // An inner `while let total` shadows the outer `let total`
        // inside the loop body: the body use stays a `shadowed` orphan.
        let src = "fn main() {\n    let total = 1;\n    while let total = Some(1) {\n        let x = total;\n        break;\n    }\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "shadowed");
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

    #[test]
    fn general_orphans_multiline_macro_capture_inside_block_kept() {
        // Multi-line macro fully inside the block: the capture reports
        // the literal line (4), not the macro start line (3).
        let src =
            "fn main() {\n    let total = 1;\n    println!(\n        \"{total}\"\n    );\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 4, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "macro-hidden");
    }

    #[test]
    fn general_orphans_multiline_macro_capture_outside_block_dropped() {
        // Binder lives in `foo`; the multi-line capture lives in `bar`.
        // Byte attribution drops it (no widening past the block).
        let src = "fn foo() {\n    let total = 1;\n}\nfn bar() {\n    println!(\n        \"{total}\"\n    );\n}\n";
        let orphans = orphans_for_src(src);
        assert!(orphans.is_empty(), "got: {orphans:?}");
    }

    #[test]
    fn general_orphans_multiline_opaque_ident_at_ident_line() {
        // Multi-line `println!("{}", total)`: the `old` ident lives on
        // line 5 so `collect_orphans` reports the ident line (5), not
        // the macro start line (3). Sibling-line scan gates on ident
        // bytes (a reported-line-only gate dropped this true orphan
        // mid-turn — unsafe direction).
        let src = "fn main() {\n    let total = 1;\n    println!(\n        \"{}\",\n        total\n    );\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 5, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "macro-hidden");
    }

    #[test]
    fn general_orphans_prebinder_capture_on_binder_line_dropped() {
        // Same-line pre-binder capture (`println!` before `let total`)
        // shares the binder's line: line gates keep it, byte gates drop
        // it (the literal sits before the binder).
        let src = "fn main() {\n    println!(\"{total}\"); let total = 1;\n    println!(\"{total}\");\n}\n";
        let orphans = orphans_for_src(src);
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
        assert_eq!(orphans[0].line, 3, "got: {orphans:?}");
        assert_eq!(orphans[0].reason, "macro-hidden");
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
    fn closure_orphans_attribute_value_names_the_line() {
        // Attribute values never rewrite (opaque like macros) and never
        // resolve to the binder: the leftover is a `macro-hidden`
        // orphan, uniform with the let path and the item/fn/assoc/member
        // parity — the verdict caps instead of verifying.
        let src =
            "fn main() {\n    let f = |total: u32| { #[doc = total] 0 };\n    let _ = f;\n}\n";
        let kind = general_expr("|total : u32| __RSMPL_E0", "|sum : u32| E");
        let orphans = wi2c_orphans(src, kind, first_closure_span(src));
        assert_eq!(orphans.len(), 1, "got: {orphans:?}");
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
    fn closure_rename_completes_body_uses() {
        // WI-3: closure-param renames complete over the body scope —
        // the binder plus unshadowed uses rewrite in-hunk (`follow`),
        // orphans stay empty, `verified`-ward (global `partial` only
        // when leftovers block it).
        let patch = format!(
            "{WI2C_HEAD}@@ -2,1 @@ rule=rename_closure\n# lang=rust\n# expr E\n- |total: u32| E\n+ |sum: u32| E\n"
        );
        let src = "fn main() {\n    let f = |total: u32| total + total;\n    let _ = f;\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("|sum: u32| sum + sum"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(report.rules[0].orphans.is_empty());
    }

    #[test]
    fn closure_rename_shadowed_use_stays_orphan() {
        // A use past an inner same-name binder is `shadowed` evidence,
        // never rewritten; leftovers keep the run non-`verified`.
        let patch = format!(
            "{WI2C_HEAD}@@ -2,1 @@ rule=rename_closure\n# lang=rust\n# expr E\n- |total: u32| E\n+ |sum: u32| E\n"
        );
        let src =
            "fn main() {\n    let f = |total: u32| { let total = 1; total };\n    let _ = f;\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("|sum: u32|"), "got: {out}");
        assert!(out.contains("let total = 1; total"), "got: {out}");
        assert_eq!(report.rules[0].semantics, crate::RuleSemantics::ReparseOnly);
        assert_eq!(report.rules[0].orphans.len(), 1);
        assert_eq!(report.rules[0].orphans[0].reason, "shadowed");
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
    fn match_arm_rename_completes_body_uses() {
        // WI-3: match-arm renames complete over the arm guard+body.
        let patch = format!(
            "{WI2C_HEAD}@@ -2,1 @@ rule=rename_arm\n# lang=rust\n# expr E\n# expr B\n- match E {{ total => B, _ => 0 }}\n+ match E {{ sum => B, _ => 0 }}\n"
        );
        let src = "fn main() {\n    let v = match Some(1) { total => total + 1, _ => 0 };\n    let _ = v;\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("sum => sum + 1"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(report.rules[0].orphans.is_empty());
    }

    #[test]
    fn match_arm_rename_shadowed_use_stays_orphan() {
        let patch = format!(
            "{WI2C_HEAD}@@ -2,1 @@ rule=rename_arm\n# lang=rust\n# expr E\n# expr B\n- match E {{ total => B, _ => 0 }}\n+ match E {{ sum => B, _ => 0 }}\n"
        );
        let src = "fn main() {\n    let v = match Some(1) { total => { let total = 1; total }, _ => 0 };\n    let _ = v;\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("sum =>"), "got: {out}");
        assert!(out.contains("let total = 1; total"), "got: {out}");
        assert_eq!(report.rules[0].semantics, crate::RuleSemantics::ReparseOnly);
        assert_eq!(report.rules[0].orphans.len(), 1);
        assert_eq!(report.rules[0].orphans[0].reason, "shadowed");
    }
}
