//! `--inspect` helper: list top-level Rust items with line spans.
//!
//! Agent use: run `rsmpl-git --repo X --inspect src/lib.rs` to learn valid
//! `@@ -old,len` ranges without reading the whole file, or
//! `--inspect src/` (`collect_rs_files`) to scan a whole subtree in one
//! call. Spans come from `syn` (1-based lines, inclusive) and cover the
//! attached attribute/doc envelope the same way matching does.

use rsmpl_patch::{ErrorCode, StructuredError};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

/// One listed item: `kind name start-end` (1-based inclusive lines).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ItemInfo {
    pub kind: String,
    pub name: String,
    pub start: u32,
    pub end: u32,
}

/// One candidate call site: `call` (free function), `method` (`recv.m(...)`)
/// or `macro` (`path!(...)` as an expression or `path!(...);` statement).
/// Names render the callee path (`a::b::foo`), the method ident, or the macro
/// path; spans are 1-based inclusive lines for `@@` range authoring.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CallInfo {
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

/// Parse `text` and list candidate call/method/macro sites with line spans.
///
/// Agent use: run `rsmpl-git --repo X --inspect src/lib.rs --calls` to learn
/// tight `@@ -old,len` ranges for call rewrites without reading the file.
/// Only path-callee calls are listed (the engine's call matcher rewrites
/// path callees only); non-path callees such as `(f())()` are skipped.
pub fn inspect_calls(text: &str) -> Result<Vec<CallInfo>, StructuredError> {
    let file: syn::File = syn::parse_str(text).map_err(|e| {
        StructuredError::new(
            ErrorCode::ParseConflict,
            format!("target file does not parse as Rust: {e}"),
        )
    })?;
    let mut collector = CallCollector { out: Vec::new() };
    collector.visit_file(&file);
    let mut out = collector.out;
    out.sort_by(|a: &CallInfo, b: &CallInfo| {
        (a.start, a.end, &a.kind, &a.name).cmp(&(b.start, b.end, &b.kind, &b.name))
    });
    Ok(out)
}

/// `syn::visit` preorder walk collecting call-like sites. `Item::Macro`
/// (top-level `macro_rules!` definitions) is intentionally not collected:
/// it already appears in `items`; here we list expression/statement macro
/// *invocations* inside code.
struct CallCollector {
    out: Vec<CallInfo>,
}

impl CallCollector {
    fn push(&mut self, kind: &str, name: String, span: proc_macro2::Span) {
        // See `span_lines`: lines cannot realistically exceed `u32`.
        #[allow(clippy::cast_possible_truncation)]
        let (start, end) = (span.start().line as u32, span.end().line as u32);
        self.out.push(CallInfo {
            kind: kind.to_string(),
            name,
            start,
            end,
        });
    }
}

fn path_name(path: &syn::Path) -> String {
    let mut name = String::new();
    if path.leading_colon.is_some() {
        name.push_str("::");
    }
    name.push_str(
        &path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
    );
    name
}

impl<'ast> Visit<'ast> for CallCollector {
    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        match node {
            syn::Expr::Call(call) => {
                if let syn::Expr::Path(p) = &*call.func {
                    self.push("call", path_name(&p.path), node.span());
                }
                visit::visit_expr(self, node);
            }
            syn::Expr::MethodCall(m) => {
                self.push("method", m.method.to_string(), node.span());
                visit::visit_expr(self, node);
            }
            syn::Expr::Macro(m) => {
                self.push("macro", path_name(&m.mac.path), node.span());
                visit::visit_expr(self, node);
            }
            _ => visit::visit_expr(self, node),
        }
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        self.push("macro", path_name(&node.mac.path), node.span());
        visit::visit_stmt_macro(self, node);
    }
}

/// Collect absolute paths of all `.rs` files under `root`, recursively.
///
/// Agent use: backs `--inspect <dir>` so one call scans a whole subtree.
/// Skips dot-directories (e.g. `.git/`), `target/`, and non-`.rs` files;
/// does not follow symlinked directories. Output is sorted
/// lexicographically by display string for determinism.
pub fn collect_rs_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    collect_rs_into(root, &mut out);
    out.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    out
}

fn collect_rs_into(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            if name.starts_with('.') || name == "target" {
                continue;
            }
            collect_rs_into(&path, out);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
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

    #[test]
    fn lists_calls_methods_and_macros() {
        let calls = inspect_calls(
            "fn foo() {}\nfn a() {\n    foo(1);\n    obj.method(x);\n    assert_eq!(a, b);\n}\n",
        )
        .unwrap();
        let kinds: Vec<(&str, &str)> = calls
            .iter()
            .map(|c| (c.kind.as_str(), c.name.as_str()))
            .collect();
        assert!(kinds.contains(&("call", "foo")), "got: {kinds:?}");
        assert!(kinds.contains(&("method", "method")), "got: {kinds:?}");
        assert!(kinds.contains(&("macro", "assert_eq")), "got: {kinds:?}");
        let foo = calls.iter().find(|c| c.kind == "call").unwrap();
        assert_eq!((foo.start, foo.end), (3, 3));
        // Sorted by start line.
        let starts: Vec<u32> = calls.iter().map(|c| c.start).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted);
    }

    #[test]
    fn call_paths_render_fully_and_multiline_spans() {
        let calls = inspect_calls("fn a() {\n    crate::b::foo(\n        1,\n    );\n}\n").unwrap();
        assert_eq!(calls.len(), 1, "got: {calls:?}");
        assert_eq!(calls[0].kind, "call");
        assert_eq!(calls[0].name, "crate::b::foo");
        assert_eq!((calls[0].start, calls[0].end), (2, 4));
    }

    #[test]
    fn skips_non_path_callees_and_reports_parse_failure() {
        // `(|| 1)()` calls a closure value, not a path callee: the engine's
        // call matcher only rewrites path callees, so nothing is listed.
        let calls = inspect_calls("fn a() {\n    (|| 1)();\n}\n").unwrap();
        assert!(calls.is_empty(), "got: {calls:?}");
        let err = inspect_calls("fn broken( {}").unwrap_err();
        assert_eq!(err.code, ErrorCode::ParseConflict);
    }

    /// Closed kind set pinned in SPEC D.3. Every kind `collect`/`other_kind`
    /// can emit must appear in the D.3 kinds sentence so agents matching on
    /// an exotic item (e.g. `extern crate foo;`) find it documented.
    const DOCUMENTED_KINDS: [&str; 17] = [
        "fn",
        "struct",
        "enum",
        "trait",
        "impl",
        "type",
        "const",
        "static",
        "mod",
        "use",
        "macro",
        "method",
        "union",
        "item",
        "extern_crate",
        "foreign_mod",
        "trait_alias",
    ];

    #[test]
    fn exotic_item_kinds_match_spec_closed_set() {
        let items = inspect_source(
            "extern crate foo;\nextern \"C\" {\n    fn ffi();\n}\ntrait Alias = Send;\nfn foo() {}\n",
        )
        .unwrap();
        let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
        assert!(
            kinds.contains(&"extern_crate"),
            "missing extern_crate, got: {kinds:?}"
        );
        assert!(
            kinds.contains(&"foreign_mod"),
            "missing foreign_mod, got: {kinds:?}"
        );
        assert!(
            kinds.contains(&"trait_alias"),
            "missing trait_alias, got: {kinds:?}"
        );
        for k in &kinds {
            assert!(
                DOCUMENTED_KINDS.contains(k),
                "undocumented kind {k}, got: {kinds:?}"
            );
        }
        // Fallback arm never emits an empty kind.
        let verbatim = syn::Item::Verbatim(proc_macro2::TokenStream::new());
        assert_eq!(other_kind(&verbatim), "item");
        // Anti-rot: every kind in the closed set is named in SPEC.md.
        let spec = include_str!("../../../SPEC.md");
        for k in DOCUMENTED_KINDS {
            assert!(spec.contains(k), "SPEC.md no longer documents kind {k}");
        }
    }

    #[test]
    fn collect_rs_files_skips_noise_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("src/nested/b.rs"), "fn b() {}\n").unwrap();
        std::fs::write(root.join("src/notes.txt"), "not rust\n").unwrap();
        std::fs::write(root.join(".git/hidden.rs"), "fn h() {}\n").unwrap();
        std::fs::write(root.join("target/debug/t.rs"), "fn t() {}\n").unwrap();
        let files = collect_rs_files(root);
        let names: Vec<String> = files
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(
            names,
            vec!["src/lib.rs", "src/nested/b.rs"],
            "got: {names:?}"
        );
    }
}
