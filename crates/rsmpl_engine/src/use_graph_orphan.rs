//! WI-7c: report-only orphan index for definition-only renames (SPEC 14.8).
//!
//! Sibling of `use_graph` (D5: new code lands in new modules). A minimal
//! syntactic visitor collects every same-name use ident plus every opaque
//! macro/attribute/`qself` line — never vetoing, never filtering on
//! ambiguity. The renamed definition's own spans are excluded by the
//! caller-supplied byte ranges. Sorted by `(file, line)`, deduped per
//! line. Unparseable files are excluded (the follow tripwire owns them).
//!
//! Mirror note: the use/opaque rules intentionally match `use_graph`'s
//! `Collector` (every-segment paths, impl self-ty/trait sides, `use`
//! trees, `qself`/macro/attribute opacity). A follow matcher change that
//! adds a use shape needs a mirror here, pinned by `orphan_e2e.rs`.

use syn::visit::{self, Visit};

use crate::lines::LineIndex;
use crate::matcher_item::span_bytes;

/// Report-only orphan collection (never fails loud: unmappable spans are
/// skipped, never guessed).
/// `macro_paths` (WI-7b mirror of `use_graph`): also list macro-invocation
/// paths (`Old!…`) as orphans when the rename is a `macro_rules!` rename.
pub fn collect_orphans(
    files: &[(String, &syn::File, &str, &LineIndex)],
    old: &str,
    renamed_file: &str,
    renamed: &[(usize, usize)],
    macro_paths: bool,
) -> Vec<crate::OrphanReport> {
    let renamed_hit_in = |path: &str, a: usize, b: usize| {
        path == renamed_file && renamed.iter().any(|(s, e)| a < *e && *s < b)
    };
    let mut out: Vec<crate::OrphanReport> = Vec::new();
    for (path, parsed, original, index) in files {
        let mut collector = Collector {
            old,
            macro_paths,
            uses: Vec::new(),
            opaque: Vec::new(),
        };
        collector.visit_file(parsed);
        // WI-2 reasons: provable uses name `old` but sit outside any
        // rewritten hunk (`off-target`); macro/attribute/`qself`
        // interiors are un-attributable (`macro-hidden`).
        let mut lines: Vec<(u32, &'static str)> = Vec::new();
        for span in &collector.uses {
            let line = span.start().line as u32;
            let Some((a, b)) = span_bytes(original, index, *span) else {
                continue;
            };
            if original.get(a..b) != Some(old) {
                continue;
            }
            if renamed_hit_in(path, a, b) {
                continue;
            }
            lines.push((line, "off-target"));
        }
        for (line, _) in &collector.opaque {
            lines.push((*line, "macro-hidden"));
        }
        lines.sort_unstable();
        lines.dedup();
        for (line, reason) in lines {
            out.push(crate::OrphanReport {
                file: (*path).clone(),
                line,
                text: source_line_text(original, index, line),
                reason: reason.to_string(),
            });
        }
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}

/// One trimmed source line (never fails: OOB lines yield empty text).
fn source_line_text(original: &str, index: &LineIndex, line: u32) -> String {
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

struct Collector<'a> {
    old: &'a str,
    macro_paths: bool,
    uses: Vec<proc_macro2::Span>,
    opaque: Vec<(u32, String)>,
}

impl Collector<'_> {
    fn push_path_use(&mut self, path: &syn::Path) {
        let mut hits_old = false;
        for seg in &path.segments {
            if seg.ident == self.old {
                hits_old = true;
                break;
            }
        }
        if !hits_old {
            return;
        }
        for seg in &path.segments {
            if seg.ident == self.old {
                self.uses.push(seg.ident.span());
            }
        }
    }

    fn push_qself_path(&mut self, path: &syn::Path) -> bool {
        let mut hit = false;
        for seg in &path.segments {
            if seg.ident == self.old {
                self.opaque.push((
                    seg.ident.span().start().line as u32,
                    "a qself path".to_string(),
                ));
                hit = true;
            }
        }
        hit
    }
}

fn macro_ident_lines(tokens: &proc_macro2::TokenStream, old: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for t in tokens.clone() {
        match t {
            proc_macro2::TokenTree::Ident(id) if id == old => {
                out.push(id.span().start().line as u32);
            }
            proc_macro2::TokenTree::Group(g) => {
                out.extend(macro_ident_lines(&g.stream(), old));
            }
            _ => {}
        }
    }
    out
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        // Self-ty and trait-side segments name the item itself.
        if let syn::Type::Path(tp) = &*node.self_ty {
            if tp.qself.is_none() {
                if let Some(last) = tp.path.segments.last() {
                    if last.ident == self.old {
                        self.uses.push(last.ident.span());
                    }
                }
            }
        }
        if let Some((_, path, _)) = &node.trait_ {
            if let Some(last) = path.segments.last() {
                if last.ident == self.old {
                    self.uses.push(last.ident.span());
                }
            }
        }
        visit::visit_item_impl(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        fn walk(tree: &syn::UseTree, old: &str, uses: &mut Vec<proc_macro2::Span>) {
            match tree {
                syn::UseTree::Path(p) => {
                    if p.ident == old {
                        uses.push(p.ident.span());
                    }
                    walk(&p.tree, old, uses);
                }
                syn::UseTree::Name(n) => {
                    if n.ident == old {
                        uses.push(n.ident.span());
                    }
                }
                syn::UseTree::Rename(r) => {
                    if r.ident == old {
                        uses.push(r.ident.span());
                    }
                }
                syn::UseTree::Group(g) => {
                    for member in &g.items {
                        walk(member, old, uses);
                    }
                }
                // Glob trees name nothing directly.
                syn::UseTree::Glob(_) => {}
            }
        }
        walk(&node.tree, self.old, &mut self.uses);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        self.push_path_use(node);
        visit::visit_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        if node.qself.is_some() && self.push_qself_path(&node.path) {
            return;
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if node.qself.is_some() && self.push_qself_path(&node.path) {
            return;
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if self.macro_paths {
            self.push_path_use(&node.path);
        }
        for line in macro_ident_lines(&node.tokens, self.old) {
            self.opaque.push((line, "a macro invocation".to_string()));
        }
    }

    fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
        // One entry per matching ident line (WI-2b ident-line parity —
        // not the attribute start line). Single-line attributes unchanged.
        for seg in node.path().segments.iter() {
            if seg.ident == self.old {
                self.opaque.push((
                    seg.ident.span().start().line as u32,
                    "an attribute".to_string(),
                ));
            }
        }
        // Value idents (`#[doc = old]`, `#[foo(bar = old)]`) are
        // equally opaque: they never resolve to the renamed item.
        // `Meta::NameValue` values are real exprs, so their paths are
        // collected structurally; `Meta::List` tokens are never parsed,
        // so they are scanned as token trees. String literals
        // (`#[doc = "old"]`, `/// old` doc comments) are `Literal`,
        // never `Ident`, and stay out by construction.
        match &node.meta {
            syn::Meta::NameValue(nv) => {
                struct ValuePaths<'o> {
                    old: &'o str,
                    opaque: Vec<u32>,
                }
                impl<'ast> Visit<'ast> for ValuePaths<'_> {
                    fn visit_path(&mut self, node: &'ast syn::Path) {
                        for seg in &node.segments {
                            if seg.ident == self.old {
                                self.opaque.push(seg.ident.span().start().line as u32);
                            }
                        }
                        visit::visit_path(self, node);
                    }
                    fn visit_macro(&mut self, node: &'ast syn::Macro) {
                        for line in macro_ident_lines(&node.tokens, self.old) {
                            self.opaque.push(line);
                        }
                    }
                }
                let mut values = ValuePaths {
                    old: self.old,
                    opaque: Vec::new(),
                };
                values.visit_expr(&nv.value);
                for line in values.opaque {
                    self.opaque.push((line, "an attribute".to_string()));
                }
            }
            syn::Meta::List(list) => {
                for line in macro_ident_lines(&list.tokens, self.old) {
                    self.opaque.push((line, "an attribute".to_string()));
                }
            }
            syn::Meta::Path(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orphans_for(src: &str, old: &str, renamed_site: &str) -> Vec<crate::OrphanReport> {
        let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
        let index = LineIndex::new(src);
        let off = src.find(renamed_site).expect("renamed site present");
        let a = off + renamed_site.find(old).expect("old inside site");
        collect_orphans(
            &[("src/a.rs".to_string(), &parsed, src, &index)],
            old,
            "src/a.rs",
            &[(a, a + old.len())],
            false,
        )
    }

    #[test]
    fn lists_use_lines_but_not_the_definition() {
        let out = orphans_for(
            "pub struct Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
            "Old",
            "struct Old",
        );
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].file.as_str(), out[0].line), ("src/a.rs", 2));
        assert!(out[0].text.contains("Old"));
    }

    #[test]
    fn opaque_macro_line_is_an_orphan() {
        let out = orphans_for(
            "pub struct Old;\nfn f() {\n    m!(Old);\n    x\n}\n",
            "Old",
            "struct Old",
        );
        assert!(out.iter().any(|o| o.line == 3), "got: {out:?}");
    }

    #[test]
    fn multiline_attribute_reports_ident_line() {
        // `Old` sits on line 3; the attribute starts on line 2.
        let out = orphans_for(
            "pub struct Old;\n#[a::\nOld]\nfn f() {}\n",
            "Old",
            "struct Old",
        );
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert_eq!(out[0].line, 3, "got: {out:?}");
        assert_eq!(out[0].reason, "macro-hidden");
    }

    #[test]
    fn attribute_value_ident_is_a_macro_hidden_orphan() {
        // `#[doc = Old]` never resolves to the item: one opaque orphan
        // at the attr line, never a use.
        let out = orphans_for(
            "pub struct Old;\n#[doc = Old]\nfn f() {}\n",
            "Old",
            "struct Old",
        );
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert_eq!(out[0].line, 2, "got: {out:?}");
        assert_eq!(out[0].reason, "macro-hidden");
    }

    #[test]
    fn attribute_list_form_ident_is_a_macro_hidden_orphan() {
        // List tokens are never parsed, so they are scanned as token
        // trees: `#[foo(Old)]` surfaces at its own line.
        let out = orphans_for(
            "pub struct Old;\n#[foo(Old)]\nfn f() {}\n",
            "Old",
            "struct Old",
        );
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert_eq!(out[0].line, 2, "got: {out:?}");
        assert_eq!(out[0].reason, "macro-hidden");
    }

    #[test]
    fn attribute_string_literal_is_not_an_orphan() {
        // `#[doc = "Old"]` and `/// Old` are literals, never idents:
        // prose mentions of the name stay silent.
        let out = orphans_for(
            "pub struct Old;\n#[doc = \"Old\"]\n/// Old\nfn f() {}\n",
            "Old",
            "struct Old",
        );
        assert!(out.is_empty(), "got: {out:?}");
    }

    #[test]
    fn multiline_qself_reports_ident_line() {
        // `Old` sits on line 3; the `<...>` starts on line 2.
        let out = orphans_for(
            "pub struct Old;\nfn f(x: <T as Tr>::a::\n    Old) {}\n",
            "Old",
            "struct Old",
        );
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert_eq!(out[0].line, 3, "got: {out:?}");
        assert_eq!(out[0].reason, "macro-hidden");
    }
}
