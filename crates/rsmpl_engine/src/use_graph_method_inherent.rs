//! WI-6 inherent-method domain (SPEC 14.8).
//!
//! Split from [`crate::use_graph_method`] under the D5 file-size gate:
//! the inherent `impl S {` half of method follow — domain resolution,
//! definition claims, namesake evidence, and the shadow census. The
//! trait half and the shared call walker stay in `use_graph_method`;
//! both halves meet in `MethodDomain`.

use syn::visit::{self, Visit};

use crate::lines::LineIndex;
use crate::matcher_item::span_bytes;
use crate::use_graph_method::{last_ident, method_trait_name, self_ty_last};

/// WI-6: method domain — trait-header (`trait Tr {`) or inherent
/// (`impl S {`) follow. `impl T for S` headers are neither (C17):
/// they name a trait-impl block, not a renameable method owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MethodDomain {
    Trait(String),
    Inherent(String),
}

/// Domain for an `ImplContext` header: trait headers by declared
/// ident, bare inherent headers by self-type last ident.
/// `None` for `impl T for S` headers and garbage.
pub(crate) fn method_domain(impl_header: &str) -> Option<MethodDomain> {
    if rsmpl_pattern::is_trait_header(impl_header) {
        return method_trait_name(impl_header).map(MethodDomain::Trait);
    }
    inherent_self_name(impl_header).map(MethodDomain::Inherent)
}

/// Self-type last ident of a bare inherent header (`impl S {` → `S`).
/// `None` for trait headers, `impl T for S` headers, and garbage.
/// Last-segment discipline (D8): `impl crate::S {` proves `S`.
pub(crate) fn inherent_self_name(impl_header: &str) -> Option<String> {
    let filter = rsmpl_pattern::parse_impl_header(impl_header)?;
    if filter.want_trait || !filter.trait_path.is_empty() {
        return None;
    }
    let name = last_ident(&filter.self_ty);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// WI-6: same-name `fn` definitions in inherent `impl` blocks of the
/// domain self type (`impl S { fn old }` for domain `S`).
/// Returns (path, start, end, line). Other-type inherent impls,
/// trait decls, and trait-impl defs never claim (different items).
pub(crate) fn collect_inherent_impl_claims(
    files: &[(String, &syn::File, &str, &LineIndex)],
    self_name: &str,
    old: &str,
) -> Vec<(String, usize, usize, u32)> {
    let mut out = Vec::new();
    for (path, parsed, original, index) in files {
        struct V<'a> {
            self_name: &'a str,
            old: &'a str,
            original: &'a str,
            index: &'a LineIndex,
            out: Vec<(usize, usize, u32)>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                if node.trait_.is_none()
                    && self_ty_last(&node.self_ty).as_deref() == Some(self.self_name)
                {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                if let Some((a, b)) =
                                    span_bytes(self.original, self.index, m.sig.ident.span())
                                {
                                    if self.original.get(a..b) == Some(self.old) {
                                        let line = m.sig.ident.span().start().line as u32;
                                        self.out.push((a, b, line));
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
            self_name,
            old,
            original,
            index,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for (a, b, line) in v.out {
            out.push((path.clone(), a, b, line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Block-aware other-type inherent lines: same-name `fn` in inherent
/// `impl` blocks of a *different* self type (evidence, never claims).
pub(crate) fn inherent_other_lines(
    files: &[(String, &syn::File, &str, &LineIndex)],
    self_name: &str,
    old: &str,
) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (path, parsed, _, _) in files {
        struct V<'a> {
            self_name: &'a str,
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                if node.trait_.is_none()
                    && self_ty_last(&node.self_ty).as_deref() != Some(self.self_name)
                {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                self.out.push(m.sig.ident.span().start().line as u32);
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut v = V {
            self_name,
            old,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for line in v.out {
            out.push((path.clone(), line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// WI-6: `trait` declarations of same-name `fn old` (any trait):
/// evidence only (`shadowed`) under an inherent rename.
pub(crate) fn collect_trait_decl_fn_lines(
    files: &[(String, &syn::File, &str, &LineIndex)],
    old: &str,
) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (path, parsed, _, _) in files {
        struct V<'a> {
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
                for it in &node.items {
                    if let syn::TraitItem::Fn(m) = it {
                        if m.sig.ident == self.old {
                            self.out.push(m.sig.ident.span().start().line as u32);
                        }
                    }
                }
                visit::visit_item_trait(self, node);
            }
        }
        let mut v = V {
            old,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for line in v.out {
            out.push((path.clone(), line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// WI-6: `impl <Trait> for _` definitions of same-name `fn old`
/// (any trait): evidence only (`shadowed`) under an inherent rename.
pub(crate) fn collect_trait_impl_fn_lines(
    files: &[(String, &syn::File, &str, &LineIndex)],
    old: &str,
) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (path, parsed, _, _) in files {
        struct V<'a> {
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                if node.trait_.is_some() {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                self.out.push(m.sig.ident.span().start().line as u32);
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut v = V {
            old,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for line in v.out {
            out.push((path.clone(), line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// WI-6: true when another item defines same-name `fn old` outside the
/// inherent domain: any `trait` decl, any `impl <T> for _` def, or any
/// inherent `impl` of a different self type.
pub(crate) fn inherent_has_shadow(
    files: &[(String, &syn::File, &str, &LineIndex)],
    self_name: &str,
    old: &str,
) -> bool {
    !collect_trait_decl_fn_lines(files, old).is_empty()
        || !collect_trait_impl_fn_lines(files, old).is_empty()
        || !inherent_other_lines(files, self_name, old).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherent_domain_names() {
        assert_eq!(
            method_domain("trait T {"),
            Some(MethodDomain::Trait("T".to_string()))
        );
        assert_eq!(
            method_domain("impl S {"),
            Some(MethodDomain::Inherent("S".to_string()))
        );
        assert_eq!(method_domain("impl T for S {"), None);
        assert_eq!(method_domain("garbage"), None);
    }

    #[test]
    fn inherent_self_name_rejects_trait_headers() {
        assert_eq!(inherent_self_name("impl S {").as_deref(), Some("S"));
        assert_eq!(inherent_self_name("trait T {"), None);
        assert_eq!(inherent_self_name("impl T for S {"), None);
    }
}
