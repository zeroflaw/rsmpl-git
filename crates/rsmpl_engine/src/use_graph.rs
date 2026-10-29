//! WI-6a: same-file `# follow=uses` resolver (SPEC 14.8).
//!
//! After a fn/item rename hunk rewrites its definition ident(s), this
//! module collects the same-file *use sites* of the old name — type paths,
//! value/call paths (bare and qualified, incl. turbofish), struct-literal
//! paths, pattern paths, and `use`-tree segments — and returns their ident
//! spans for rewriting under the same rule.
//!
//! Precision-first (SPEC 2.6, D3): anything the syntactic walk cannot prove
//! is a veto, not a guess. A second same-named definition, a local binding,
//! or a macro/attribute mention the resolver cannot attribute makes the
//! whole hunk fail with a follow veto — unless the hunk also carries
//! `# fallback=syntax`, in which case the unprovable sites are skipped and
//! listed for the WI-2 `follow_skipped` binding. Field accesses, method
//! idents, and `qself` paths are never rewritten (different namespaces /
//! declined, WI-3 scope note); they never veto either, except a `qself`
//! path naming `old`, which is opaque.
//!
//! v0.1 scope is the definition's own file (WI-6b owns the workspace
//! index). The hunk's range is ignored for uses: a follow rewrites the
//! whole file, which is the point (no range discovery).

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::lines::LineIndex;
use crate::matcher_item::span_bytes;

/// Successful resolution: byte ranges to rewrite plus skip notes.
#[derive(Debug)]
pub struct FollowOutcome {
    /// Ident byte ranges (`original[a..b] == old`), sorted, deduped.
    pub spans: Vec<(usize, usize)>,
    /// `line N: ...` notes for sites left untouched under fallback.
    pub skipped: Vec<String>,
}

/// A fail-loud veto: the first unprovable site (no `# fallback=syntax`).
#[derive(Debug)]
pub struct FollowVeto {
    /// Human reason naming the site (embeds the 1-based line).
    pub reason: String,
}

/// Resolve same-file use sites of `old` for a `old` → `new` rename.
///
/// `renamed` carries the byte ranges the definition hunk already rewrote
/// (its ident-span edits): a same-named definition overlapping one of them
/// is the renamed definition itself, any other is shadowing. `fallback`
/// mirrors `# fallback=syntax` (skip + report instead of veto).
pub fn resolve(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    old: &str,
    renamed: &[(usize, usize)],
    fallback: bool,
) -> Result<FollowOutcome, FollowVeto> {
    let mut collector = Collector {
        old,
        defs: Vec::new(),
        uses: Vec::new(),
        opaque: Vec::new(),
    };
    collector.visit_file(parsed);

    // Byte-map definitions (fail loud on unmappable spans, never guess).
    let mut def_ranges: Vec<(usize, usize, u32, &'static str)> = Vec::new();
    for (span, kind) in &collector.defs {
        let line = span.start().line as u32;
        match span_bytes(original, index, *span) {
            Some((a, b)) => def_ranges.push((a, b, line, kind)),
            None => {
                return Err(FollowVeto {
                    reason: format!(
                        "definition of `{old}` ({kind}) at line {line} maps outside the file"
                    ),
                });
            }
        }
    }
    let renamed_hit = |a: usize, b: usize| renamed.iter().any(|(s, e)| a < *e && *s < b);
    let shadow: Vec<(u32, &'static str)> = def_ranges
        .iter()
        .filter(|(a, b, _, _)| !renamed_hit(*a, *b))
        .map(|(_, _, line, kind)| (*line, *kind))
        .collect();

    // Map + verify use spans (text must be exactly `old`, never guessed).
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for span in &collector.uses {
        let line = span.start().line as u32;
        let (a, b) = span_bytes(original, index, *span).ok_or_else(|| FollowVeto {
            reason: format!("use of `{old}` at line {line} maps outside the file"),
        })?;
        if original.get(a..b) != Some(old) {
            return Err(FollowVeto {
                reason: format!("use of `{old}` at line {line} is not the expected text"),
            });
        }
        // The renamed definition's own header spans (e.g. `impl Old`
        // self-ty, already rewritten by the def edit) are not uses.
        if renamed_hit(a, b) {
            continue;
        }
        spans.push((a, b));
    }
    spans.sort_unstable();
    spans.dedup();

    let mut skipped: Vec<String> = Vec::new();
    if !shadow.is_empty() {
        // Every remaining use is ambiguous (which definition does it name?),
        // so under fallback all use rewriting is skipped, not just the
        // shadowed definition. Without fallback the hunk vetoes.
        if !fallback {
            let (line, kind) = shadow[0];
            return Err(FollowVeto {
                reason: format!(
                    "shadowed definition of `{old}` ({kind}) at line {line}: cannot tell which definition each use names"
                ),
            });
        }
        for (line, kind) in &shadow {
            skipped.push(format!(
                "line {line}: shadowed definition of `{old}` ({kind}); {} use sites left untouched",
                spans.len()
            ));
        }
        spans.clear();
    }
    if !collector.opaque.is_empty() {
        if !fallback {
            let (line, ctx) = &collector.opaque[0];
            return Err(FollowVeto {
                reason: format!(
                    "use of `{old}` inside {ctx} at line {line}: macro/attribute interiors are opaque"
                ),
            });
        }
        for (line, ctx) in &collector.opaque {
            skipped.push(format!(
                "line {line}: use of `{old}` inside {ctx} left untouched (opaque)"
            ));
        }
    }
    Ok(FollowOutcome { spans, skipped })
}

/// Whole-ident test on token text, recursing through groups: `old` must
/// appear as a full ident, not a substring (`older`, `my_old` do not
/// count). A missed nested group would be a silent stale site, so the
/// walk is total over the token tree.
fn tokens_mention(tokens: &proc_macro2::TokenStream, old: &str) -> bool {
    tokens.clone().into_iter().any(|t| match t {
        proc_macro2::TokenTree::Ident(id) => id == old,
        proc_macro2::TokenTree::Group(g) => tokens_mention(&g.stream(), old),
        _ => false,
    })
}

struct Collector<'a> {
    old: &'a str,
    /// (span, kind label) definition idents named `old`.
    defs: Vec<(proc_macro2::Span, &'static str)>,
    /// Use-site ident spans named `old`.
    uses: Vec<proc_macro2::Span>,
    /// (line, context) opaque mentions.
    opaque: Vec<(u32, String)>,
}

impl Collector<'_> {
    fn is_old(&self, ident: &syn::Ident) -> bool {
        ident == self.old
    }

    fn push_def(&mut self, ident: &syn::Ident, kind: &'static str) {
        if self.is_old(ident) {
            self.defs.push((ident.span(), kind));
        }
    }

    /// Collect every segment spelling `old` (`Old`, `m::Old`, `Old::new`,
    /// `a::Old::assoc`). Any segment naming a *different* item implies a
    /// same-named definition, which the census turns into a veto — so
    /// rewriting every `old` segment is sound, never a guess. Callers at
    /// `Type::Path` / `Expr::Path` level split off `qself` first (a
    /// qualified `Old` is opaque, WI-3 scope note); a bare `Path` node
    /// never carries qualification itself. Turbofish lives on the
    /// segment, outside the ident span, so it is preserved verbatim.
    fn push_path_use(&mut self, path: &syn::Path) {
        for seg in &path.segments {
            if seg.ident == self.old {
                self.uses.push(seg.ident.span());
            }
        }
    }

    /// `qself` split: any `old` segment under qualification is opaque
    /// (declined); anything else recurses normally so uses nested inside
    /// the `qself` type still resolve. Returns true when the site was
    /// consumed as opaque.
    fn push_qself_path(&mut self, path: &syn::Path) -> bool {
        if path.segments.iter().any(|s| s.ident == self.old) {
            self.opaque
                .push((path.span().start().line as u32, "a qself path".to_string()));
            return true;
        }
        false
    }

    /// Walk a `use` tree collecting segments spelling `old` (WI-6a: any
    /// position — `use m::Old`, `use Old as X`, `use m::{Old, Y}`).
    fn walk_use_tree(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => {
                if p.ident == self.old {
                    self.uses.push(p.ident.span());
                }
                self.walk_use_tree(&p.tree);
            }
            syn::UseTree::Name(n) => {
                if n.ident == self.old {
                    self.uses.push(n.ident.span());
                }
            }
            syn::UseTree::Rename(r) => {
                if r.ident == self.old {
                    self.uses.push(r.ident.span());
                }
            }
            syn::UseTree::Glob(_) => {}
            syn::UseTree::Group(g) => {
                for member in &g.items {
                    self.walk_use_tree(member);
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.push_def(&node.sig.ident, "fn");
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.push_def(&node.sig.ident, "fn");
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.push_def(&node.sig.ident, "fn");
        visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.push_def(&node.ident, "struct");
        visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.push_def(&node.ident, "enum");
        visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.push_def(&node.ident, "trait");
        visit::visit_item_trait(self, node);
    }

    fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
        self.push_def(&node.ident, "union");
        visit::visit_item_union(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.push_def(&node.ident, "type");
        visit::visit_item_type(self, node);
    }

    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.push_def(&node.ident, "const");
        visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.push_def(&node.ident, "static");
        visit::visit_item_static(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.push_def(&node.ident, "mod");
        visit::visit_item_mod(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        // Self-ty and trait-side last segments name the renamed item
        // itself (Rust coherence: `impl Old` implements `Old`), so they
        // are use sites to rewrite, never shadowing definitions. The
        // renamed-range filter drops the ones the def hunk already
        // rewrote; the rest follow here (an `impl Old` left stale would
        // break the build the hunk just fixed).
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

    fn visit_variant(&mut self, node: &'ast syn::Variant) {
        // `E::Old` would otherwise misread as a use of a renamed `Old`.
        self.push_def(&node.ident, "variant");
        visit::visit_variant(self, node);
    }

    fn visit_impl_item_const(&mut self, node: &'ast syn::ImplItemConst) {
        self.push_def(&node.ident, "assoc const");
        visit::visit_impl_item_const(self, node);
    }

    fn visit_impl_item_type(&mut self, node: &'ast syn::ImplItemType) {
        self.push_def(&node.ident, "assoc type");
        visit::visit_impl_item_type(self, node);
    }

    fn visit_trait_item_const(&mut self, node: &'ast syn::TraitItemConst) {
        self.push_def(&node.ident, "assoc const");
        visit::visit_trait_item_const(self, node);
    }

    fn visit_trait_item_type(&mut self, node: &'ast syn::TraitItemType) {
        self.push_def(&node.ident, "assoc type");
        visit::visit_trait_item_type(self, node);
    }

    fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
        // A local binding named `old` shadows value-position uses.
        self.push_def(&node.ident, "local");
        visit::visit_pat_ident(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        self.walk_use_tree(&node.tree);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        // Covers type paths, value/call paths (bare, qualified, and
        // mid-path segments — `Old::new`, `a::Old::assoc`), struct-literal
        // and pattern paths. Method idents and field members are never
        // `Path`s, so they are untouched by construction. `qself` splits
        // happen at the `Type::Path` / `Expr::Path` parents below.
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
        // Token interiors are opaque: no recursion, mention recorded.
        if tokens_mention(&node.tokens, self.old) {
            self.opaque.push((
                node.span().start().line as u32,
                "a macro invocation".to_string(),
            ));
        }
    }

    fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
        // Attribute paths (`#[derive(Old)]`) live outside the item/value
        // namespaces the resolver proves; a mention is opaque, and there
        // is nothing to recurse into that the resolver would claim.
        if node.path().segments.iter().any(|s| s.ident == self.old) {
            self.opaque
                .push((node.span().start().line as u32, "an attribute".to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_for_test(
        src: &str,
        old: &str,
        renamed_site: &str,
        fallback: bool,
    ) -> Result<FollowOutcome, FollowVeto> {
        let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
        let index = LineIndex::new(src);
        // The renamed definition: byte range of `renamed_site`'s `old`.
        let off = src.find(renamed_site).expect("renamed site must exist");
        let a = off + renamed_site.find(old).expect("old inside site");
        resolve(&parsed, src, &index, old, &[(a, a + old.len())], fallback)
    }

    const STRUCT_SRC: &str = "use m::Old;\nstruct Old {\n    field: i32,\n}\nimpl Old {\n    fn make() -> Old {\n        Old { field: 0 }\n    }\n}\nfn f(x: Old) -> Old {\n    let y: Old = Old::make();\n    let z = Old;\n    z\n}\n";

    #[test]
    fn struct_follow_collects_type_literal_use_and_value_sites() {
        let out = resolve_for_test(STRUCT_SRC, "Old", "struct Old", false).unwrap();
        assert!(out.skipped.is_empty());
        // use-tree + impl self-ty (filtered: it is the renamed def range…
        // no — the renamed range is the struct header, so the self-ty IS a
        // use here) + return type + literal + param + return + ascription +
        // leading `Old::make` + value = 9 use spans.
        assert_eq!(out.spans.len(), 9, "spans: {:?}", out.spans);
    }

    #[test]
    fn struct_follow_skips_nothing_when_clean() {
        let out = resolve_for_test(STRUCT_SRC, "Old", "struct Old", true).unwrap();
        assert!(out.skipped.is_empty());
        assert_eq!(out.spans.len(), 9);
    }

    #[test]
    fn middle_segment_module_paths_rewrite() {
        // `crate::Old::make` names `Old` mid-path; every-segment rule
        // rewrites it.
        let src = "mod m;\nstruct Old;\nfn f() {\n    let x = crate::Old::make();\n    x\n}\n";
        let out = resolve_for_test(src, "Old", "struct Old", false).unwrap();
        assert!(out.skipped.is_empty());
        assert_eq!(out.spans.len(), 1, "spans: {:?}", out.spans);
    }

    #[test]
    fn enum_follow_rewrites_leading_variant_qualifier() {
        // Renaming `E` must reach `E::V`: the item names the first segment.
        let src = "enum E {\n    V,\n}\nfn f(x: E) -> E {\n    let y = E::V;\n    y\n}\n";
        let out = resolve_for_test(src, "E", "enum E", false).unwrap();
        assert!(out.skipped.is_empty());
        assert_eq!(out.spans.len(), 3, "spans: {:?}", out.spans);
    }

    #[test]
    fn nested_macro_groups_are_opaque() {
        let src = "struct Old;\nfn f() {\n    m!(vec![Old]);\n    let x = 1;\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("opaque"), "got: {}", err.reason);
    }

    #[test]
    fn fn_follow_rewrites_calls_but_not_methods_or_fields() {
        let src = "fn old(x: i32) -> i32 {\n    x + 1\n}\nfn g(s: S) -> i32 {\n    let a = old(1);\n    let b = m::old(2);\n    let c = s.old();\n    let d = s.old;\n    a + b\n}\n";
        let out = resolve_for_test(src, "old", "fn old", false).unwrap();
        assert!(out.skipped.is_empty());
        // `old(1)` + `m::old(2)` only: method call + field stay.
        assert_eq!(out.spans.len(), 2, "spans: {:?}", out.spans);
    }

    #[test]
    fn shadowed_definition_vetoes_without_fallback() {
        let src = "mod a {\n    pub struct Old;\n}\nmod b {\n    pub struct Old;\n}\n";
        let err = resolve_for_test(src, "Old", "pub struct Old", false).unwrap_err();
        assert!(
            err.reason.contains("shadowed definition"),
            "got: {}",
            err.reason
        );
    }

    #[test]
    fn shadowed_definition_skips_all_uses_with_fallback() {
        let src = "mod a {\n    pub struct Old;\n}\nmod b {\n    pub struct Old;\n    pub fn f(x: Old) -> Old {\n        x\n    }\n}\n";
        let out = resolve_for_test(src, "Old", "pub struct Old", true).unwrap();
        assert!(out.spans.is_empty(), "all uses ambiguous: {:?}", out.spans);
        assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
        assert!(out.skipped[0].contains("use sites left untouched"));
    }

    #[test]
    fn macro_mention_vetoes_without_fallback_and_skips_with_it() {
        let src = "struct Old;\nfn f() {\n    m!(Old);\n    let x: Old = make();\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("opaque"), "got: {}", err.reason);
        let out = resolve_for_test(src, "Old", "struct Old", true).unwrap();
        // Clean sites still rewrite; only the macro site is skipped.
        assert_eq!(out.spans.len(), 1, "spans: {:?}", out.spans);
        assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
    }

    #[test]
    fn qself_mention_is_opaque() {
        let src =
            "struct Old;\nfn f<T: Tr>() where T: Tr {\n    let x = <T as Tr>::Old;\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("qself"), "got: {}", err.reason);
    }

    #[test]
    fn variant_with_same_name_shadows() {
        let src = "struct Old;\nenum E {\n    Old,\n}\nfn f(x: E) -> E {\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("shadowed"), "got: {}", err.reason);
    }

    #[test]
    fn local_binding_named_old_shadows_fn_rename() {
        let src =
            "fn old() -> i32 {\n    1\n}\nfn g() -> i32 {\n    let old = 2;\n    old + old()\n}\n";
        let err = resolve_for_test(src, "old", "fn old", false).unwrap_err();
        assert!(err.reason.contains("shadowed"), "got: {}", err.reason);
    }
}
