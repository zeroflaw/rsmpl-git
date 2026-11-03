//! WI-1.4 syn definition collector: every definition ident in the
//! post-image file with exact text and line range. Exact ident equality
//! (never substring) is what removes the `New`/`New2` prefix hazard.
//!
//! Split from `postcheck.rs` (D5 line gate); behavior identical. The
//! `as`-renamed use source (`UseTree::Rename::ident`) is recorded
//! alongside the alias so a rewritten `use crate::new as mid` verifies
//! against the `new` needle.

/// WI-1.4 definition-ident classes the syn collector distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefKind {
    #[allow(dead_code)]
    /// Free functions and impl/trait methods (`fn` idents).
    Fn,
    /// Item names (struct/enum/trait/union/type/const/static/mod) plus
    /// `impl` self-ty and trait-path idents.
    Item,
    /// Struct field idents.
    Field,
    /// Enum variant idents.
    Variant,
    /// `use`-item path-segment idents.
    UseSeg,
}

/// One collected definition ident with its 1-based inclusive line range.
#[derive(Debug, Clone)]
pub(crate) struct DefIdent {
    pub(crate) kind: DefKind,
    pub(crate) name: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
}

pub(crate) fn span_lines<S: syn::spanned::Spanned>(node: &S) -> (usize, usize) {
    let s = node.span();
    let start = s.start().line.max(1);
    let end = s.end().line.max(start);
    (start, end)
}

fn ranges_overlap(def: (usize, usize), span: (u32, u32)) -> bool {
    (def.0 as u32) <= span.1 && span.0 <= (def.1 as u32)
}

/// WI-1.4 syn definition collector: every definition ident in the
/// post-image file with exact text and line range. Exact ident equality
/// (never substring) is what removes the `New`/`New2` prefix hazard.
#[derive(Default)]
struct DefCollector {
    defs: Vec<DefIdent>,
}

impl DefCollector {
    /// Record one ident by its own token lines (not the enclosing
    /// item's): a reported span must overlap the ident itself, so an
    /// attr-deletion span above the item never counts as ident-bearing.
    fn push(&mut self, kind: DefKind, name: &syn::Ident) {
        let (start_line, end_line) = span_lines(name);
        self.defs.push(DefIdent {
            kind,
            name: name.to_string(),
            start_line,
            end_line,
        });
    }

    fn push_use(&mut self, tree: &syn::UseTree) {
        use syn::UseTree;
        match tree {
            UseTree::Path(p) => {
                self.push(DefKind::UseSeg, &p.ident);
                self.push_use(&p.tree);
            }
            UseTree::Name(n) => {
                self.push(DefKind::UseSeg, &n.ident);
            }
            UseTree::Rename(r) => {
                self.push(DefKind::UseSeg, &r.ident);
                self.push(DefKind::UseSeg, &r.rename);
            }
            UseTree::Glob(_) => {}
            UseTree::Group(g) => {
                for t in &g.items {
                    self.push_use(t);
                }
            }
        }
    }

    fn push_path_last(&mut self, path: &syn::Path) {
        if let Some(seg) = path.segments.last() {
            self.push(DefKind::Item, &seg.ident);
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for DefCollector {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.push(DefKind::Fn, &node.sig.ident);
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.push(DefKind::Fn, &node.sig.ident);
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.push(DefKind::Fn, &node.sig.ident);
        syn::visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.push(DefKind::Item, &node.ident);
        for field in &node.fields {
            if let Some(ident) = field.ident.as_ref() {
                self.push(DefKind::Field, ident);
            }
        }
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.push(DefKind::Item, &node.ident);
        for variant in &node.variants {
            self.push(DefKind::Variant, &variant.ident);
        }
        syn::visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_union(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_type(self, node);
    }

    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_static(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_mod(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        // Self-ty ident (`impl New {`, `impl ... for New {`).
        if let syn::Type::Path(tp) = node.self_ty.as_ref() {
            self.push_path_last(&tp.path);
        }
        // Trait-side last segment (`impl Labeled for S` renames the
        // trait path's last segment alongside the self-ty).
        if let Some((_, path, _)) = node.trait_.as_ref() {
            self.push_path_last(path);
        }
        syn::visit::visit_item_impl(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        self.push_use(&node.tree);
        syn::visit::visit_item_use(self, node);
    }

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        // WI-1.4: `macro_rules! name` definitions carry the name in
        // `ItemMacro::ident` (the token stream holds only the rules), so
        // the text posture's `new_m`-on-the-line check needs this syn
        // counterpart.
        if let Some(ident) = node.ident.as_ref() {
            self.push(DefKind::Item, ident);
        }
        syn::visit::visit_item_macro(self, node);
    }

    fn visit_trait_item_type(&mut self, node: &'ast syn::TraitItemType) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_trait_item_type(self, node);
    }

    fn visit_trait_item_const(&mut self, node: &'ast syn::TraitItemConst) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_trait_item_const(self, node);
    }

    fn visit_impl_item_type(&mut self, node: &'ast syn::ImplItemType) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_impl_item_type(self, node);
    }

    fn visit_impl_item_const(&mut self, node: &'ast syn::ImplItemConst) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_impl_item_const(self, node);
    }
}

pub(crate) fn collect_def_idents(parsed: &syn::File) -> Vec<DefIdent> {
    let mut collector = DefCollector::default();
    use syn::visit::Visit;
    collector.visit_file(parsed);
    collector.defs
}

/// WI-1.4 syn span count: reported spans overlapping a collected
/// definition ident exactly equal to `needle`. `spans` are post-image
/// coordinates; empty spans (byte-identical self-rewrite probes, which
/// emit no edits) count whole-file ident-equality matches instead —
/// exact idents, never a substring, so the `New`/`New2` prefix hazard
/// stays closed on the fallback path too.
pub(crate) fn syn_spans_count(
    defs: &[DefIdent],
    kind: DefKind,
    spans: &[(u32, u32)],
    needle: &str,
) -> usize {
    if needle.is_empty() {
        return 0;
    }
    if spans.is_empty() {
        return defs
            .iter()
            .filter(|d| d.kind == kind && d.name == needle)
            .count();
    }
    spans
        .iter()
        .filter(|sp| {
            defs.iter().any(|d| {
                d.kind == kind
                    && d.name == needle
                    && ranges_overlap((d.start_line, d.end_line), **sp)
            })
        })
        .count()
}
