//! Non-function item renames (SPEC 14.8, T12).
//!
//! Rename-only rewrite for `struct`, `enum`, `trait`, `union`, `type`,
//! `const`, `static`, `mod` and `impl` self-ty: one bare
//! `<kind> <Old>` / `<kind> <New>` line pair replaces a single name-ident
//! span per match. Visibility, generics, bodies, attributes and formatting
//! are preserved byte-for-byte, exactly like the fn rename fast-path.
//! `impl` renames the self-type ident only; trait-side paths in
//! `impl Trait for Type` headers are T19 path renames, not matched here.

use syn::spanned::Spanned;

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, ItemKind, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// One rename candidate with original byte range + line span.
struct ItemCandidate {
    name: String,
    ident: proc_macro2::Span,
    attrs: Vec<syn::Attribute>,
    start_line: u32,
    end_line: u32,
    anchor_line: u32,
    attr_start_line: u32,
}

fn push_candidate(
    out: &mut Vec<ItemCandidate>,
    name: String,
    ident: proc_macro2::Span,
    attrs: Vec<syn::Attribute>,
    item_span: proc_macro2::Span,
) {
    out.push(ItemCandidate {
        name,
        attr_start_line: item_span.start().line as u32,
        start_line: item_span.start().line as u32,
        end_line: item_span.end().line as u32,
        anchor_line: ident.start().line as u32,
        ident,
        attrs,
    });
}

/// Last-segment ident of a plain path type (`Old`, `Old<T>`, `crate::Old`).
/// Anything else (references, tuples, `dyn`, ...) is not a rename target.
fn self_ty_ident(ty: &syn::Type) -> Option<&syn::Ident> {
    if let syn::Type::Path(tp) = ty {
        if tp.qself.is_none() {
            return tp.path.segments.last().map(|s| &s.ident);
        }
    }
    None
}

/// Collect rename candidates of `want` kind from file + inline mods.
fn collect(items: &[syn::Item], want: ItemKind, out: &mut Vec<ItemCandidate>) {
    for it in items {
        match it {
            syn::Item::Struct(s) if want == ItemKind::Struct => push_candidate(
                out,
                s.ident.to_string(),
                s.ident.span(),
                s.attrs.clone(),
                s.span(),
            ),
            syn::Item::Enum(e) if want == ItemKind::Enum => push_candidate(
                out,
                e.ident.to_string(),
                e.ident.span(),
                e.attrs.clone(),
                e.span(),
            ),
            syn::Item::Trait(t) if want == ItemKind::Trait => push_candidate(
                out,
                t.ident.to_string(),
                t.ident.span(),
                t.attrs.clone(),
                t.span(),
            ),
            syn::Item::Union(u) if want == ItemKind::Union => push_candidate(
                out,
                u.ident.to_string(),
                u.ident.span(),
                u.attrs.clone(),
                u.span(),
            ),
            syn::Item::Type(t) if want == ItemKind::Type => push_candidate(
                out,
                t.ident.to_string(),
                t.ident.span(),
                t.attrs.clone(),
                t.span(),
            ),
            syn::Item::Const(c) if want == ItemKind::Const => push_candidate(
                out,
                c.ident.to_string(),
                c.ident.span(),
                c.attrs.clone(),
                c.span(),
            ),
            syn::Item::Static(s) if want == ItemKind::Static => push_candidate(
                out,
                s.ident.to_string(),
                s.ident.span(),
                s.attrs.clone(),
                s.span(),
            ),
            syn::Item::Mod(m) if want == ItemKind::Mod => {
                push_candidate(
                    out,
                    m.ident.to_string(),
                    m.ident.span(),
                    m.attrs.clone(),
                    m.span(),
                );
                // Recurse: `mod outer { struct Inner; }` nests items.
                if let Some((_, inner)) = &m.content {
                    collect(inner, want, out);
                }
            }
            syn::Item::Mod(m) => {
                // Even when renaming another kind, nested items are visible.
                if let Some((_, inner)) = &m.content {
                    collect(inner, want, out);
                }
            }
            syn::Item::Impl(imp) if want == ItemKind::Impl => {
                if let Some(ident) = self_ty_ident(&imp.self_ty) {
                    push_candidate(
                        out,
                        ident.to_string(),
                        ident.span(),
                        imp.attrs.clone(),
                        imp.span(),
                    );
                }
            }
            _ => {}
        }
    }
}

/// Scope filtering (SPEC 17.3). `anchor` uses the ident line as the first
/// concrete non-ellipsis token; other modes use the attr-envelope span.
fn in_scope(c: &ItemCandidate, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.start_line >= old_start && c.end_line <= end.max(old_start),
        Scope::Anchor => c.anchor_line >= old_start && c.anchor_line <= end.max(old_start),
        Scope::Overlap => c.start_line <= end.max(old_start) && c.end_line >= old_start,
        Scope::Exact => old_len > 0 && c.start_line == old_start && c.end_line == end,
    }
}

fn check_attrs(
    attrs: &[syn::Attribute],
    policy: AttrsPolicy,
    pattern_attrs: &[String],
    rule: &str,
    file: &str,
) -> Result<(), StructuredError> {
    let has = !attrs.is_empty();
    match policy {
        AttrsPolicy::Preserve => Ok(()),
        AttrsPolicy::Absent => {
            if has {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "item has attributes but `# attrs=absent` requires none",
                )
                .with_rule(rule)
                .with_file(file));
            }
            Ok(())
        }
        AttrsPolicy::Explicit => {
            let want: Vec<String> = pattern_attrs
                .iter()
                .map(|a| a.split_whitespace().collect::<String>())
                .collect();
            let have: Vec<String> = attrs
                .iter()
                .map(|a| quote::quote!(#a).to_string().split_whitespace().collect())
                .collect();
            if want != have {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "attached attributes do not match explicitly written pattern attributes",
                )
                .with_rule(rule)
                .with_file(file)
                .with_suggestion("use `# attrs=preserve` (default) to ignore attributes"));
            }
            Ok(())
        }
        AttrsPolicy::Strip => Ok(()),
    }
}

/// Byte range of `span` in the original text, or `None` outside the file.
fn span_bytes(
    original: &str,
    index: &LineIndex,
    span: proc_macro2::Span,
) -> Option<(usize, usize)> {
    let s = span.start();
    let e = span.end();
    let a = index.offset(s.line as u32, s.column)?;
    let b = index.offset(e.line as u32, e.column)?;
    if a <= b && b <= original.len() {
        Some((a, b))
    } else {
        None
    }
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
}

/// Apply an item-rename hunk. Returns the number of matches rewritten.
pub fn apply_item_rename_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (kind, minus_name, plus_name, minus_attrs) = match &compiled.kind {
        PatternKind::ItemRename {
            kind,
            minus_name,
            plus_name,
            minus_attrs,
            ..
        } => (
            *kind,
            minus_name.clone(),
            plus_name.clone(),
            minus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected item rename pattern".to_string(),
            ));
        }
    };
    // A `+` side that writes attributes the `-` side omitted is ambiguous
    // (same rule as General stmts): reject instead of guessing.
    if let PatternKind::ItemRename { plus_attrs, .. } = &compiled.kind {
        if !plus_attrs.is_empty() && minus_attrs.is_empty() {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "plus side writes attributes the minus side omitted (ambiguous merge); write the attributes on both sides or neither",
            )
            .with_rule(hunk.rule_name()));
        }
    }
    let rule = hunk.rule_name();
    let file = "";
    let mut candidates = Vec::new();
    collect(&parsed.items, kind, &mut candidates);
    // Deterministic source order for `allow=many`.
    candidates.sort_by_key(|c| (c.start_line, c.anchor_line));

    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    for c in &candidates {
        if c.name != minus_name {
            continue;
        }
        if !in_scope(c, hunk.scope(), old_start, old_len) {
            continue;
        }
        check_attrs(&c.attrs, hunk.attrs_policy(), &minus_attrs, &rule, file)?;
        let (a, b) = span_bytes(original, index, c.ident)
            .ok_or_else(|| range_err(&rule, c.ident.start().line as u32))?;
        if original.get(a..b) != Some(minus_name.as_str()) {
            return Err(StructuredError::new(
                ErrorCode::MatchConflict,
                "item name span mismatch (file changed during matching?)",
            )
            .with_rule(&rule));
        }
        edits.push(TextEdit::new(a, b, plus_name.clone()));
        // `# attrs=strip`: delete attribute lines above the item.
        if hunk.attrs_policy() == AttrsPolicy::Strip && !c.attrs.is_empty() {
            let start = index
                .line_start(c.attr_start_line)
                .ok_or_else(|| range_err(&rule, c.attr_start_line))?;
            let end = index
                .line_start(c.anchor_line)
                .ok_or_else(|| range_err(&rule, c.anchor_line))?;
            if end > start {
                edits.push(TextEdit::new(start, end, String::new()));
            }
        }
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break; // count validation reports the error
        }
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files["src/lib.rs"].clone())
    }

    fn rename_patch(minus: &str, plus: &str, range: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=rename\n# lang=rust\n- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn renames_struct_preserving_body_vis_generics_and_attrs() {
        let patch = rename_patch("struct Old", "struct New", "@@ -1,4 +1,4 @@");
        let src = "#[derive(Debug)]\npub struct Old<T> {\n    x: T,\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("pub struct New<T>"), "got: {out}");
        assert!(out.contains("#[derive(Debug)]"));
        assert!(out.contains("x: T,"));
        assert!(!out.contains("Old"));
    }

    #[test]
    fn renames_each_supported_kind() {
        let cases = [
            (
                "enum Old",
                "enum New",
                "enum Old {\n    A,\n}\n",
                "enum New {",
                "@@ -1,3 +1,3 @@",
            ),
            (
                "trait Old",
                "trait New",
                "trait Old {\n    fn f(&self);\n}\n",
                "trait New {",
                "@@ -1,3 +1,3 @@",
            ),
            (
                "type Old",
                "type New",
                "type Old = Vec<u8>;\n",
                "type New = Vec<u8>;",
                "@@ -1,1 +1,1 @@",
            ),
            (
                "const Old",
                "const New",
                "const Old: usize = 10;\n",
                "const New: usize = 10;",
                "@@ -1,1 +1,1 @@",
            ),
            (
                "static Old",
                "static New",
                "static Old: u8 = 1;\n",
                "static New: u8 = 1;",
                "@@ -1,1 +1,1 @@",
            ),
            (
                "static mut Old",
                "static mut New",
                "static mut Old: u8 = 1;\n",
                "static mut New: u8 = 1;",
                "@@ -1,1 +1,1 @@",
            ),
            (
                "union Old",
                "union New",
                "union Old {\n    a: u8,\n}\n",
                "union New {",
                "@@ -1,3 +1,3 @@",
            ),
            (
                "mod old",
                "mod new",
                "mod old {}\n",
                "mod new {}",
                "@@ -1,1 +1,1 @@",
            ),
        ];
        for (minus, plus, src, want, range) in cases {
            let patch = rename_patch(minus, plus, range);
            let out = apply(&patch, src).unwrap();
            assert!(out.contains(want), "case {minus}: got: {out}");
        }
    }

    #[test]
    fn renames_impl_self_ty_only() {
        let patch = rename_patch("impl Old", "impl New", "@@ -1,4 +1,4 @@");
        let src = "struct Old;\nimpl Old {\n    fn f(&self) {}\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("impl New {"), "got: {out}");
        // Generic self-ty keeps its arguments.
        let patch2 = rename_patch("impl Old", "impl New", "@@ -2,3 +2,3 @@");
        let src2 = "struct Old<T>(T);\nimpl<T> Old<T> {\n    fn f(&self) {}\n}\n";
        let out2 = apply(&patch2, src2).unwrap();
        assert!(out2.contains("impl<T> New<T>"), "got: {out2}");
        // `impl Trait for Type`: self-ty side matches.
        let patch3 = rename_patch("impl Old", "impl New", "@@ -2,3 +2,3 @@");
        let src3 = "struct Old;\nimpl Clone for Old {\n    fn clone(&self) -> Self { Old }\n}\n";
        let out3 = apply(&patch3, src3).unwrap();
        assert!(out3.contains("impl Clone for New"), "got: {out3}");
    }

    #[test]
    fn impl_trait_side_does_not_match() {
        // `impl Old for Foo` names Old as the trait, not the self-ty:
        // a bare `impl Old` rename must not touch it (T19 territory).
        let patch = rename_patch("impl Old", "impl New", "@@ -1,3 +1,3 @@");
        let src = "trait Old {}\nstruct Foo;\nimpl Old for Foo {}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn allow_many_renames_every_match() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=many\n# lang=rust\n# allow=many\n- struct Old\n+ struct New\n";
        let src = "struct Old;\nstruct Old;\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out.matches("struct New").count(), 2, "got: {out}");
    }

    #[test]
    fn nested_mod_items_found() {
        let patch = rename_patch("struct Helper", "struct Aid", "@@ -2,1 +2,1 @@");
        let src = "mod outer {\n    pub struct Helper;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("pub struct Aid;"), "got: {out}");
        assert!(out.contains("mod outer {"));
    }

    #[test]
    fn name_mismatch_is_no_match() {
        let patch = rename_patch("struct Old", "struct New", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, "struct Other;\n").is_err());
    }

    #[test]
    fn kind_mismatch_rejected_at_compile() {
        let patch = rename_patch("struct Old", "enum New", "@@ -1,1 +1,1 @@");
        let err = apply(&patch, "struct Old;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }

    #[test]
    fn concrete_body_rejected() {
        let patch = rename_patch("struct Old { x: i32 }", "struct New", "@@ -1,3 +1,3 @@");
        let err = apply(&patch, "struct Old {\n    x: i32,\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }

    #[test]
    fn attrs_policies() {
        // preserve (default) keeps attributes.
        let patch = rename_patch("struct Old", "struct New", "@@ -1,2 +1,2 @@");
        let out = apply(&patch, "#[derive(Debug)]\nstruct Old;\n").unwrap();
        assert!(out.contains("#[derive(Debug)]\nstruct New;"), "got: {out}");
        // absent rejects attributed items.
        let absent1 = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# attrs=absent\n- struct Old\n+ struct New\n";
        let absent2 = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# attrs=absent\n- struct Old\n+ struct New\n";
        assert!(apply(absent1, "struct Old;\n").is_ok());
        assert!(apply(absent2, "#[derive(Debug)]\nstruct Old;\n").is_err());
        // strip deletes attribute lines.
        let strip = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# attrs=strip\n- struct Old\n+ struct New\n";
        let out = apply(strip, "#[derive(Debug)]\nstruct Old;\n").unwrap();
        assert!(!out.contains("derive"), "got: {out}");
        assert!(out.contains("struct New;"), "got: {out}");
    }

    #[test]
    fn scope_modes_gate_matches() {
        let patch = rename_patch("struct Old", "struct New", "@@ -1,1 +1,1 @@");
        // Item spans lines 3-5; anchor-only range on line 1 misses.
        let src = "fn a() {}\nfn b() {}\nstruct Old;\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn does_not_shadow_fn_or_call_shapes() {
        // `const fn` still routes to the fn classifier (strict vis applies).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- const fn foo(...) {\n+ const fn bar(...) {\n      ...\n  }\n";
        let out = apply(patch, "const fn foo() {}\n").unwrap();
        assert!(out.contains("const fn bar()"), "got: {out}");
        // Ordinary calls still route to the call matcher.
        let call = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let out = apply(call, "fn a() {\n    foo(1);\n}\n").unwrap();
        assert!(out.contains("bar(1)"), "got: {out}");
    }
}
