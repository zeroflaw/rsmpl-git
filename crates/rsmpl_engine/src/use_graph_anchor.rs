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
        // WI-10: shadow set is build-time only (never stored — the
        // scopes already carry the deref'd containers).
        let no_unwrap = collect_no_unwrap(parsed);
        // WI-10 slice 2: same-file Deref targets, likewise build-time
        // only (scopes carry the deref'd containers).
        let deref_targets = collect_deref_targets(parsed);
        let mut idx = Self::default();
        let mut builder = Builder {
            idx: &mut idx,
            no_unwrap: &no_unwrap,
            deref_targets: &deref_targets,
        };
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
    no_unwrap: &'a HashSet<String>,
    deref_targets: &'a HashMap<String, String>,
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
                    if let Some(container) =
                        type_tail_deref(&pat.ty, self.no_unwrap, self.deref_targets)
                    {
                        scope.params.insert(id.ident.to_string(), container);
                    }
                }
            }
        }
        let mut collector = LocalCollector {
            out: &mut scope.bindings,
            no_unwrap: self.no_unwrap,
            deref_targets: self.deref_targets,
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
                    if let Some(container) =
                        type_tail_deref(&pat.ty, self.no_unwrap, self.deref_targets)
                    {
                        scope.params.insert(id.ident.to_string(), container);
                    }
                }
            }
        }
        let mut collector = LocalCollector {
            out: &mut scope.bindings,
            no_unwrap: self.no_unwrap,
            deref_targets: self.deref_targets,
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
    no_unwrap: &'a HashSet<String>,
    deref_targets: &'a HashMap<String, String>,
}

impl<'ast> Visit<'ast> for LocalCollector<'_> {
    fn visit_local(&mut self, node: &'ast syn::Local) {
        let bind_line = node.span().start().line as u32;
        match &node.pat {
            // `let x: C = …` — ascription lives in `Pat::Type` in syn 2.
            syn::Pat::Type(t) => {
                if let syn::Pat::Ident(id) = t.pat.as_ref() {
                    if let Some(container) =
                        type_tail_deref(&t.ty, self.no_unwrap, self.deref_targets)
                    {
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

/// WI-10 slice 1: one-step deref unwrap over [`type_tail`].
/// WI-10 slice 2: same-file `impl Deref for C` targets.
///
/// `Box<T>` / `Rc<T>` / `Arc<T>` (exactly one generic type arg)
/// anchors to the inner tail `T`; `&`/`&mut` peel first via the
/// shared tail so `&Box<S>` composes. A plain `C` with a same-file
/// `impl Deref for C { type Target = T; }` anchors to the outer
/// tail of `T`. Single step only: `Box<Box<S>>` unwraps to `Box`
/// (off-target against `S`); `Box<C>` unwraps to `C` (wrapper wins,
/// never chains to `T`); `&C` peels then derefs to `T`.
/// Anything else (incl. `Vec<S>`) keeps the outer-type behavior.
///
/// Resolution, not spelling (D8): a single-segment wrapper unwraps
/// unless this file shadows the name (`no_unwrap`); a multi-segment
/// path unwraps only on a `std`/`alloc`/`core` first segment
/// (`std::boxed::Box<S>` yes, `foo::Box<S>` no).
fn type_tail_deref(
    ty: &syn::Type,
    no_unwrap: &HashSet<String>,
    deref_targets: &HashMap<String, String>,
) -> Option<String> {
    match ty {
        syn::Type::Path(tp) => {
            if tp.qself.is_some() {
                return None;
            }
            let last = tp.path.segments.last()?;
            let name = last.ident.to_string();
            if is_wrapper(&name) && path_allows_unwrap(&tp.path, &name, no_unwrap) {
                if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
                    let inners: Vec<&syn::Type> = args
                        .args
                        .iter()
                        .filter_map(|a| match a {
                            syn::GenericArgument::Type(t) => Some(t),
                            _ => None,
                        })
                        .collect();
                    if inners.len() == 1 {
                        // Inner is one deref step: peel refs, take the
                        // plain-path tail. Non-path inners (`dyn Trait`)
                        // fall through to the outer name (off-target).
                        if let Some(inner) = plain_tail(inners[0]) {
                            return Some(inner);
                        }
                    }
                }
            }
            // Same-file Deref target: one step, no chaining through
            // wrappers (the wrapper arm above already returned).
            if tp.path.segments.len() == 1 {
                if let Some(target) = deref_targets.get(&name) {
                    return Some(target.clone());
                }
            }
            Some(name)
        }
        syn::Type::Reference(r) => type_tail_deref(&r.elem, no_unwrap, deref_targets),
        _ => None,
    }
}

/// Inner tail of one deref step: refs peeled, plain path only.
fn plain_tail(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(tp) => {
            if tp.qself.is_some() {
                return None;
            }
            tp.path.segments.last().map(|s| s.ident.to_string())
        }
        syn::Type::Reference(r) => plain_tail(&r.elem),
        _ => None,
    }
}

fn is_wrapper(name: &str) -> bool {
    matches!(name, "Box" | "Rc" | "Arc")
}

fn path_allows_unwrap(path: &syn::Path, name: &str, no_unwrap: &HashSet<String>) -> bool {
    if path.segments.len() == 1 {
        return !no_unwrap.contains(name);
    }
    matches!(
        path.segments
            .first()
            .map(|s| s.ident.to_string())
            .as_deref(),
        Some("std" | "alloc" | "core")
    )
}

/// WI-10 slice 2: same-file `impl Deref for C` targets (`C -> T`
/// outer tail of `type Target = T;`). `Target = Self` is a no-op and
/// is never stored. Only the trait's last ident must read `Deref`;
/// a user type merely *named* `Deref` in another module is a named
/// gap (workspace resolution in slice 3 disambiguates by path).
fn collect_deref_targets(parsed: &syn::File) -> HashMap<String, String> {
    struct V {
        out: HashMap<String, String>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
            let names_deref = match &node.trait_ {
                Some((_, p, _)) => p.segments.last().is_some_and(|s| s.ident == "Deref"),
                None => false,
            };
            if names_deref {
                if let Some(self_ty) = self_ty_last(&node.self_ty) {
                    for it in &node.items {
                        if let syn::ImplItem::Type(t) = it {
                            if t.ident == "Target" {
                                if let Some(target) = type_tail(&t.ty) {
                                    if target != "Self" && !target.is_empty() {
                                        self.out.insert(self_ty.clone(), target);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            visit::visit_item_impl(self, node);
        }
    }
    let mut v = V {
        out: HashMap::new(),
    };
    v.visit_file(parsed);
    v.out
}

/// WI-10: names in the closed wrapper set that this file shadows, so
/// the deref unwrap must not fire. Local `struct`/`enum`/`union`/`type`
/// definitions plus `use` imports of the bare name from a non-std
/// prefix (`use foo::Box` shadows; `use std::boxed::Box` does not).
fn collect_no_unwrap(parsed: &syn::File) -> HashSet<String> {
    struct V {
        out: HashSet<String>,
    }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
            if is_wrapper(&node.ident.to_string()) {
                self.out.insert(node.ident.to_string());
            }
            visit::visit_item_struct(self, node);
        }
        fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
            if is_wrapper(&node.ident.to_string()) {
                self.out.insert(node.ident.to_string());
            }
            visit::visit_item_enum(self, node);
        }
        fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
            if is_wrapper(&node.ident.to_string()) {
                self.out.insert(node.ident.to_string());
            }
            visit::visit_item_union(self, node);
        }
        fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
            if is_wrapper(&node.ident.to_string()) {
                self.out.insert(node.ident.to_string());
            }
            visit::visit_item_type(self, node);
        }
        fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
            let mut pairs: Vec<(Vec<String>, String)> = Vec::new();
            flatten_use_names(&node.tree, &mut Vec::new(), &mut pairs);
            for (full, local) in pairs {
                if is_wrapper(&local)
                    && !full
                        .first()
                        .is_some_and(|f| f == "std" || f == "alloc" || f == "core")
                {
                    self.out.insert(local);
                }
            }
        }
    }
    let mut v = V {
        out: HashSet::new(),
    };
    v.visit_file(parsed);
    v.out
}

/// (full path segments, local name) for one `use` tree.
fn flatten_use_names(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut Vec<(Vec<String>, String)>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            flatten_use_names(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            out.push((full, n.ident.to_string()));
        }
        syn::UseTree::Rename(r) => {
            let mut full = prefix.clone();
            full.push(r.ident.to_string());
            out.push((full, r.rename.to_string()));
        }
        syn::UseTree::Glob(_) => {}
        syn::UseTree::Group(g) => {
            for t in &g.items {
                flatten_use_names(t, prefix, out);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(src: &str, var: &str, line: u32) -> Option<String> {
        let parsed: syn::File = syn::parse_str(src).expect("fixture parses");
        AnchorIndex::for_file(&parsed).anchor_of(var, line)
    }

    #[test]
    fn box_param_anchors_to_inner() {
        let src = "struct S;\nfn f(b: Box<S>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 3).as_deref(), Some("S"));
    }

    #[test]
    fn rc_arc_let_anchors_to_inner() {
        let src = "struct S;\nfn f() {\n    let x: Rc<S> = y;\n    x;\n}\n";
        assert_eq!(anchor(src, "x", 4).as_deref(), Some("S"));
        let src = "struct S;\nfn f() {\n    let x: Arc<S> = y;\n    x;\n}\n";
        assert_eq!(anchor(src, "x", 4).as_deref(), Some("S"));
    }

    #[test]
    fn vec_stays_outer() {
        let src = "struct S;\nfn f(b: Vec<S>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 3).as_deref(), Some("Vec"));
    }

    #[test]
    fn local_struct_box_shadows() {
        let src = "struct Box;\nstruct S;\nfn f(b: Box<S>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 4).as_deref(), Some("Box"));
    }

    #[test]
    fn foreign_path_box_stays_outer() {
        let src = "struct S;\nfn f(b: foo::Box<S>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 3).as_deref(), Some("Box"));
    }

    #[test]
    fn std_path_box_unwraps() {
        let src = "struct S;\nfn f(b: std::boxed::Box<S>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 3).as_deref(), Some("S"));
    }

    #[test]
    fn double_deref_stays_outer_by_one_step() {
        // Single unwrap only: Box<Box<S>> -> Box (off-target vs S).
        let src = "struct S;\nfn f(b: Box<Box<S>>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 3).as_deref(), Some("Box"));
    }

    #[test]
    fn ref_of_box_composes() {
        let src = "struct S;\nfn f(b: &Box<S>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 3).as_deref(), Some("S"));
    }

    #[test]
    fn shadow_import_blocks_unwrap() {
        let src = "use foo::Box;\nstruct S;\nfn f(b: Box<S>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 4).as_deref(), Some("Box"));
    }

    #[test]
    fn std_import_keeps_unwrap() {
        let src = "use std::boxed::Box;\nstruct S;\nfn f(b: Box<S>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 4).as_deref(), Some("S"));
    }

    #[test]
    fn deref_target_param_anchors_to_target() {
        let src = "struct C;\nstruct S;\nimpl Deref for C {\n    type Target = S;\n}\nfn f(c: C) {\n    c;\n}\n";
        assert_eq!(anchor(src, "c", 7).as_deref(), Some("S"));
    }

    #[test]
    fn wrapper_wins_over_deref_target_no_chain() {
        // `Box<C>` unwraps to `C`, never chains to `T`.
        let src = "struct C;\nstruct S;\nimpl Deref for C {\n    type Target = S;\n}\nfn f(b: Box<C>) {\n    b;\n}\n";
        assert_eq!(anchor(src, "b", 6).as_deref(), Some("C"));
    }

    #[test]
    fn ref_of_deref_target_composes() {
        let src = "struct C;\nstruct S;\nimpl Deref for C {\n    type Target = S;\n}\nfn f(c: &C) {\n    c;\n}\n";
        assert_eq!(anchor(src, "c", 6).as_deref(), Some("S"));
    }

    #[test]
    fn vec_of_deref_target_stays_outer() {
        let src = "struct C;\nstruct S;\nimpl Deref for C {\n    type Target = S;\n}\nfn f(v: Vec<C>) {\n    v;\n}\n";
        assert_eq!(anchor(src, "v", 6).as_deref(), Some("Vec"));
    }

    #[test]
    fn deref_target_self_is_noop() {
        let src =
            "struct C;\nimpl Deref for C {\n    type Target = Self;\n}\nfn f(c: C) {\n    c;\n}\n";
        assert_eq!(anchor(src, "c", 6).as_deref(), Some("C"));
    }
}
