//! WI-14: type-anchored member follow (SPEC 14.8 anchor classes).
//!
//! Syntactic anchors prove which container a `.old` receiver names,
//! from text the change already contains — fn-signature param types,
//! `let x: C` annotations, `let x = C { … }` struct-literal inits, and
//! `self` inside `impl C`. Order is document order per fn body;
//! a later `let x: B` re-anchors earlier `x: A` uses after it
//! (rebinding-shadow veto, pinned). `-> C` returns never anchor
//! receivers. Variant bare-`Old` needs a `use …::E::*` glob anchor.

use std::collections::{HashMap, HashSet};

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

/// Per-file anchor index for one member-follow resolution.
#[derive(Debug, Default)]
pub(crate) struct AnchorIndex {
    scopes: Vec<FnScope>,
    impls: Vec<ImplScope>,
    glob_containers: HashSet<String>,
}

#[derive(Debug)]
struct FnScope {
    start: u32,
    end: u32,
    params: HashMap<String, String>,
    /// (var, container, bind_line) in document order.
    bindings: Vec<(String, String, u32)>,
}

#[derive(Debug)]
struct ImplScope {
    start: u32,
    end: u32,
    self_name: String,
}

impl AnchorIndex {
    /// Build the index for one parsed file.
    pub(crate) fn for_file(parsed: &syn::File) -> Self {
        let mut idx = Self::default();
        let mut builder = Builder { idx: &mut idx };
        builder.visit_file(parsed);
        idx
    }

    /// Strongest anchor for `var` at 1-based `use_line`, or `None`
    /// when no anchor proves it. Latest binding with
    /// `bind_line < use_line` inside the enclosing fn wins over the
    /// param default.
    pub(crate) fn anchor_of(&self, var: &str, use_line: u32) -> Option<String> {
        for scope in &self.scopes {
            if use_line < scope.start || use_line > scope.end {
                continue;
            }
            let mut current = scope.params.get(var).cloned();
            for (name, container, bind_line) in &scope.bindings {
                if name == var && *bind_line < use_line {
                    current = Some(container.clone());
                }
            }
            // Inside a fn scope the (possibly-None) result is final:
            // fn scopes never nest as siblings in this index.
            return current;
        }
        None
    }

    /// `Self`-type name enclosing `line`, if any.
    pub(crate) fn self_type_at(&self, line: u32) -> Option<&str> {
        for scope in &self.impls {
            if line >= scope.start && line <= scope.end && !scope.self_name.is_empty() {
                return Some(&scope.self_name);
            }
        }
        None
    }

    /// True when the file imports `container::*` (glob anchor for
    /// bare variant spellings).
    pub(crate) fn has_glob_anchor(&self, container: &str) -> bool {
        self.glob_containers.contains(container)
    }
}

struct Builder<'a> {
    idx: &'a mut AnchorIndex,
}

impl<'ast> Visit<'ast> for Builder<'_> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let start = node.span().start().line as u32;
        let end = node.span().end().line as u32;
        let mut scope = FnScope {
            start,
            end,
            params: HashMap::new(),
            bindings: Vec::new(),
        };
        for arg in &node.sig.inputs {
            if let syn::FnArg::Typed(pat) = arg {
                if let syn::Pat::Ident(id) = pat.pat.as_ref() {
                    if let Some(container) = type_tail(&pat.ty) {
                        scope.params.insert(id.ident.to_string(), container);
                    }
                }
            }
        }
        let mut collector = LocalCollector {
            out: &mut scope.bindings,
        };
        collector.visit_block(&node.block);
        self.idx.scopes.push(scope);
        // Do not recurse into the fn body again (locals already
        // collected in document order); nested items still visited.
        for stmt in &node.block.stmts {
            if let syn::Stmt::Item(item) = stmt {
                self.visit_item(item);
            }
        }
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let start = node.span().start().line as u32;
        let end = node.span().end().line as u32;
        let mut scope = FnScope {
            start,
            end,
            params: HashMap::new(),
            bindings: Vec::new(),
        };
        for arg in &node.sig.inputs {
            if let syn::FnArg::Typed(pat) = arg {
                if let syn::Pat::Ident(id) = pat.pat.as_ref() {
                    if let Some(container) = type_tail(&pat.ty) {
                        scope.params.insert(id.ident.to_string(), container);
                    }
                }
            }
        }
        let mut collector = LocalCollector {
            out: &mut scope.bindings,
        };
        collector.visit_block(&node.block);
        self.idx.scopes.push(scope);
        for stmt in &node.block.stmts {
            if let syn::Stmt::Item(item) = stmt {
                self.visit_item(item);
            }
        }
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if let Some(name) = self_ty_last(&node.self_ty) {
            let start = node.span().start().line as u32;
            let end = node.span().end().line as u32;
            self.idx.impls.push(ImplScope {
                start,
                end,
                self_name: name,
            });
        }
        visit::visit_item_impl(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        let mut prefix = Vec::new();
        walk_glob(&node.tree, &mut prefix, &mut self.idx.glob_containers);
    }
}

/// Collect `let` anchors in document order (flow-ordered, rebinding
/// overwrites at lookup): `let x: C = …` and `let x = C { … }`.
struct LocalCollector<'a> {
    out: &'a mut Vec<(String, String, u32)>,
}

impl<'ast> Visit<'ast> for LocalCollector<'_> {
    fn visit_local(&mut self, node: &'ast syn::Local) {
        let bind_line = node.span().start().line as u32;
        match &node.pat {
            // `let x: C = …` — ascription lives in `Pat::Type` in syn 2.
            syn::Pat::Type(t) => {
                if let syn::Pat::Ident(id) = t.pat.as_ref() {
                    if let Some(container) = type_tail(&t.ty) {
                        self.out.push((id.ident.to_string(), container, bind_line));
                    }
                }
            }
            syn::Pat::Ident(id) => {
                let var = id.ident.to_string();
                if let Some(init) = node.init.as_ref() {
                    if let Some(container) = expr_struct_name(&init.expr) {
                        self.out.push((var, container, bind_line));
                    }
                }
            }
            _ => {}
        }
        visit::visit_local(self, node);
    }
}

fn walk_glob(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut HashSet<String>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            walk_glob(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Group(g) => {
            for member in &g.items {
                walk_glob(member, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => {
            if let Some(last) = prefix.last() {
                out.insert(last.clone());
            }
        }
        _ => {}
    }
}

/// Last type-path segment (`A` in `A`, `&A`, `&mut A`, `Vec<A>` outer
/// is `Vec` — params name the outer type, which is what receivers
/// carry; `m::A` yields `A`).
fn type_tail(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(tp) => {
            if tp.qself.is_some() {
                return None;
            }
            tp.path.segments.last().map(|s| s.ident.to_string())
        }
        syn::Type::Reference(r) => type_tail(&r.elem),
        _ => None,
    }
}

fn self_ty_last(ty: &syn::Type) -> Option<String> {
    type_tail(ty)
}

/// Container name when `expr` is a struct literal `S { … }`.
fn expr_struct_name(expr: &syn::Expr) -> Option<String> {
    if let syn::Expr::Struct(s) = expr {
        return s.path.segments.last().map(|seg| seg.ident.to_string());
    }
    None
}

/// Root ident of a field base (`a` in `a.old` / `a.b.old`,
/// `self` in `self.old`). `None` for calls, literals, and other
/// unanchored bases (`make_a().old` never claims).
pub(crate) fn field_root(base: &syn::Expr) -> Option<String> {
    match base {
        syn::Expr::Path(p) => {
            if p.qself.is_some() || p.path.segments.len() != 1 {
                return None;
            }
            p.path.segments.first().map(|s| s.ident.to_string())
        }
        syn::Expr::Field(f) => field_root(&f.base),
        _ => None,
    }
}
