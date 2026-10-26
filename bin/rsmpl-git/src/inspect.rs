//! `--inspect` helper: list top-level Rust items with line spans.
//!
//! Agent use: run `rsmpl-git --repo X --inspect src/lib.rs` to learn valid
//! `@@ -old,len` ranges without reading the whole file. Spans come from
//! `syn` (1-based lines, inclusive) and cover the attached attribute/doc
//! envelope the same way matching does.

use rsmpl_patch::{ErrorCode, StructuredError};
use syn::spanned::Spanned;

/// One listed item: `kind name start-end` (1-based inclusive lines).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ItemInfo {
    pub kind: String,
    pub name: String,
    pub start: u32,
    pub end: u32,
}

/// Parse `text` and list its items. Failures are structured
/// `RSMPL_PARSE_CONFLICT` errors so CLI exit codes stay machine-readable
/// (and `--format json` can envelope them).
pub fn inspect_source(text: &str) -> Result<Vec<ItemInfo>, StructuredError> {
    let file: syn::File = syn::parse_str(text).map_err(|e| {
        StructuredError::new(
            ErrorCode::ParseConflict,
            format!("target file does not parse as Rust: {e}"),
        )
    })?;
    let mut out = Vec::new();
    collect(&file.items, "", &mut out);
    out.sort_by(|a: &ItemInfo, b: &ItemInfo| {
        (a.start, a.end, &a.kind, &a.name).cmp(&(b.start, b.end, &b.kind, &b.name))
    });
    Ok(out)
}

/// 1-based line span of a syntax node. Line counts cannot realistically
/// exceed `u32`; the engine uses `u32` lines throughout.
#[allow(clippy::cast_possible_truncation)]
fn span_lines<T: Spanned>(node: &T) -> (u32, u32) {
    (
        node.span().start().line as u32,
        node.span().end().line as u32,
    )
}

#[allow(clippy::too_many_lines)]
fn collect(items: &[syn::Item], prefix: &str, out: &mut Vec<ItemInfo>) {
    for item in items {
        match item {
            syn::Item::Fn(f) => {
                let (s, e) = span_lines(f);
                out.push(ItemInfo {
                    kind: "fn".to_string(),
                    name: format!("{prefix}{}", f.sig.ident),
                    start: s,
                    end: e,
                });
            }
            syn::Item::Struct(s) => {
                let (st, e) = span_lines(s);
                out.push(ItemInfo {
                    kind: "struct".to_string(),
                    name: format!("{prefix}{}", s.ident),
                    start: st,
                    end: e,
                });
            }
            syn::Item::Enum(e) => {
                let (s, en) = span_lines(e);
                out.push(ItemInfo {
                    kind: "enum".to_string(),
                    name: format!("{prefix}{}", e.ident),
                    start: s,
                    end: en,
                });
            }
            syn::Item::Trait(t) => {
                let (s, e) = span_lines(t);
                out.push(ItemInfo {
                    kind: "trait".to_string(),
                    name: format!("{prefix}{}", t.ident),
                    start: s,
                    end: e,
                });
            }
            syn::Item::Impl(imp) => {
                let (s, e) = span_lines(imp);
                out.push(ItemInfo {
                    kind: "impl".to_string(),
                    name: format!("{prefix}{}", impl_name(imp)),
                    start: s,
                    end: e,
                });
                let self_ty = {
                    let ty = &*imp.self_ty;
                    render(&quote::quote!(#ty))
                };
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        let (ms, me) = span_lines(m);
                        out.push(ItemInfo {
                            kind: "method".to_string(),
                            name: format!("{prefix}{self_ty}::{}", m.sig.ident),
                            start: ms,
                            end: me,
                        });
                    }
                }
            }
            syn::Item::Mod(m) => {
                let (s, e) = span_lines(m);
                out.push(ItemInfo {
                    kind: "mod".to_string(),
                    name: format!("{prefix}{}", m.ident),
                    start: s,
                    end: e,
                });
                if let Some((_, inner)) = &m.content {
                    collect(inner, &format!("{prefix}{}::", m.ident), out);
                }
            }
            syn::Item::Use(u) => {
                let (s, e) = span_lines(u);
                let tree = &u.tree;
                out.push(ItemInfo {
                    kind: "use".to_string(),
                    name: format!("{prefix}{}", render(&quote::quote!(#tree))),
                    start: s,
                    end: e,
                });
            }
            syn::Item::Const(c) => {
                let (s, e) = span_lines(c);
                out.push(ItemInfo {
                    kind: "const".to_string(),
                    name: format!("{prefix}{}", c.ident),
                    start: s,
                    end: e,
                });
            }
            syn::Item::Static(s) => {
                let (st, e) = span_lines(s);
                out.push(ItemInfo {
                    kind: "static".to_string(),
                    name: format!("{prefix}{}", s.ident),
                    start: st,
                    end: e,
                });
            }
            syn::Item::Type(t) => {
                let (s, e) = span_lines(t);
                out.push(ItemInfo {
                    kind: "type".to_string(),
                    name: format!("{prefix}{}", t.ident),
                    start: s,
                    end: e,
                });
            }
            syn::Item::Macro(m) => {
                let (s, e) = span_lines(m);
                let last = m.mac.path.segments.last();
                out.push(ItemInfo {
                    kind: "macro".to_string(),
                    name: format!("{prefix}{}", last.map_or_default(|s| s.ident.to_string())),
                    start: s,
                    end: e,
                });
            }
            syn::Item::Union(u) => {
                let (s, e) = span_lines(u);
                out.push(ItemInfo {
                    kind: "union".to_string(),
                    name: format!("{prefix}{}", u.ident),
                    start: s,
                    end: e,
                });
            }
            _ => {
                let (s, e) = span_lines(item);
                let kind = other_kind(item);
                let name = if prefix.is_empty() {
                    kind.clone()
                } else {
                    format!("{prefix}{kind}")
                };
                out.push(ItemInfo {
                    kind,
                    name,
                    start: s,
                    end: e,
                });
            }
        }
    }
}

/// Kind label for item variants without a dedicated `collect` arm.
/// Every arm returns a non-empty kind so JSON consumers never see `""`.
fn other_kind(item: &syn::Item) -> String {
    match item {
        syn::Item::ExternCrate(_) => "extern_crate".to_string(),
        syn::Item::ForeignMod(_) => "foreign_mod".to_string(),
        syn::Item::TraitAlias(_) => "trait_alias".to_string(),
        _ => "item".to_string(),
    }
}

fn impl_name(imp: &syn::ItemImpl) -> String {
    let ty = &*imp.self_ty;
    let self_ty = render(&quote::quote!(#ty));
    match &imp.trait_ {
        Some((_, path, _)) => {
            let p = path;
            format!("{} for {}", render(&quote::quote!(#p)), self_ty)
        }
        None => self_ty,
    }
}

fn render(tokens: &proc_macro2::TokenStream) -> String {
    tokens
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_fns_and_impl_methods() {
        let items =
            inspect_source("fn foo() {}\nstruct S;\nimpl S {\n    fn m(&self) {}\n}\n").unwrap();
        let kinds: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.kind.as_str(), i.name.as_str()))
            .collect();
        assert!(kinds.contains(&("fn", "foo")));
        assert!(kinds.contains(&("struct", "S")));
        assert!(kinds.contains(&("impl", "S")));
        assert!(kinds.contains(&("method", "S::m")));
        let foo = items.iter().find(|i| i.name == "foo").unwrap();
        assert_eq!((foo.start, foo.end), (1, 1));
    }

    #[test]
    fn parse_failure_mentions_conflict_code() {
        let err = inspect_source("fn broken( {}").unwrap_err();
        assert_eq!(err.code, ErrorCode::ParseConflict);
        assert!(err.render().contains("RSMPL_PARSE_CONFLICT"));
    }
}
