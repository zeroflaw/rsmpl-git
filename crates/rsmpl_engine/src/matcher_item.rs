//! Non-function item renames (SPEC 14.8, T12).
//!
//! Rename-only rewrite for `struct`, `enum`, `trait`, `union`, `type`,
//! `const`, `static`, `mod` and `impl` self-ty: one bare
//! `<kind> <Old>` / `<kind> <New>` line pair replaces a single name-ident
//! span per match. Visibility, generics, bodies, attributes and formatting
//! are preserved byte-for-byte, exactly like the fn rename fast-path.
//! `impl` renames the self-ty ident and the trait-side path's last-segment
//! ident (T19b): in valid Rust one name cannot be both a type and a trait in
//! the same scope, so a bare `impl Old` pair targets whichever side(s) name it.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

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

/// Collect rename candidates of `want` kind from file items, inline
/// modules, and block-level items (T23: fn bodies, closures, method
/// bodies — anywhere the syntactic walk reaches; macro bodies stay
/// excluded per T133). Each item node is visited once; the push logic
/// is unchanged, only the traversal widened.
fn collect(items: &[syn::Item], want: ItemKind, out: &mut Vec<ItemCandidate>) {
    struct Collector {
        want: ItemKind,
        found: Vec<ItemCandidate>,
    }
    impl<'ast> Visit<'ast> for Collector {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            consider(it, self.want, &mut self.found);
            visit::visit_item(self, it);
        }
    }
    let mut collector = Collector {
        want,
        found: Vec::new(),
    };
    for it in items {
        collector.visit_item(it);
    }
    out.extend(collector.found);
}

/// Push `it` as a rename candidate when it has the wanted kind.
/// Traversal (inline mods, bodies) is the caller's `Visit` default.
fn consider(it: &syn::Item, want: ItemKind, out: &mut Vec<ItemCandidate>) {
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
        syn::Item::Mod(m) if want == ItemKind::Mod => push_candidate(
            out,
            m.ident.to_string(),
            m.ident.span(),
            m.attrs.clone(),
            m.span(),
        ),
        syn::Item::Macro(m) if want == ItemKind::MacroRules => {
            // `macro_rules! Old { ... }`: the definition name is the
            // `ident` (not the `macro_rules` path); attribute-style
            // macros (`my_attr!{...}` in item position) have other
            // paths and never match (WI-7b sibling predicate).
            if !super::use_graph_macro::is_macro_rules_path(&m.mac.path) {
                return;
            }
            if let Some(ident) = m.ident.as_ref() {
                push_candidate(
                    out,
                    ident.to_string(),
                    ident.span(),
                    m.attrs.clone(),
                    m.span(),
                );
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
            // T19b: trait-side path (`impl Old for Foo`). Match the last
            // segment ident so renaming a trait updates its impl headers;
            // generics/qualifiers on the path are preserved. The trait
            // position is a plain `syn::Path` (no qualified-self form
            // exists there), so no qself guard is needed.
            if let Some((_, path, _)) = &imp.trait_ {
                if let Some(seg) = path.segments.last() {
                    let ident = &seg.ident;
                    push_candidate(
                        out,
                        ident.to_string(),
                        ident.span(),
                        imp.attrs.clone(),
                        imp.span(),
                    );
                }
            }
        }
        _ => {}
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
        Scope::File => true,
    }
}

/// Whitespace-insensitive attribute comparison shared by the Explicit
/// arm, the Preserve site filter, and the plus/minus mirror gate
/// (T235): pattern text (token-joined at parse, `///` normalized to
/// `#[doc]`) vs `quote!`-rendered target attributes or minus-side text.
fn norm_attr_text(s: &str) -> String {
    s.split_whitespace().collect()
}

fn attrs_equal(pattern_attrs: &[String], attrs: &[syn::Attribute]) -> bool {
    let want: Vec<String> = pattern_attrs.iter().map(|a| norm_attr_text(a)).collect();
    let have: Vec<String> = attrs
        .iter()
        .map(|a| norm_attr_text(&quote::quote!(#a).to_string()))
        .collect();
    want == have
}

fn attrs_text_equal(a: &[String], b: &[String]) -> bool {
    let norm = |ss: &[String]| ss.iter().map(|s| norm_attr_text(s)).collect::<Vec<_>>();
    norm(a) == norm(b)
}

pub(crate) fn check_attrs(
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
                .with_file(file)
                .with_repair(StructuredError::fix_pattern_repair(
                    "attrs",
                    "remove `# attrs=absent` to ignore attributes (the default), or write the attributes into the pattern",
                )));
            }
            Ok(())
        }
        AttrsPolicy::Explicit => {
            if !attrs_equal(pattern_attrs, attrs) {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "attached attributes do not match explicitly written pattern attributes",
                )
                .with_rule(rule)
                .with_file(file)
                .with_suggestion("use `# attrs=preserve` (default) to ignore attributes")
                .with_repair(StructuredError::add_directive_repair("# attrs=preserve")));
            }
            Ok(())
        }
        AttrsPolicy::Strip => Ok(()),
    }
}

/// Byte range of `span` in the original text, or `None` outside the file.
pub(crate) fn span_bytes(
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
    .with_repair(StructuredError::internal_repair())
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
            .with_rule(hunk.rule_name())
            .with_repair(StructuredError::fix_pattern_repair(
                "C11",
                "write the attributes on both sides or neither",
            )));
        }
    }
    // Plus-side attribute text must mirror the minus side (same contract
    // as fn/general hunks, T235): the item rewrite only replaces the
    // ident span, so differing plus-side attrs would be silently dropped.
    // `# attrs=strip` skips this gate — the envelope is deleted anyway.
    if let PatternKind::ItemRename { plus_attrs, .. } = &compiled.kind {
        if hunk.attrs_policy() != AttrsPolicy::Strip
            && !minus_attrs.is_empty()
            && !attrs_text_equal(&minus_attrs, plus_attrs)
        {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
            )
            .with_rule(hunk.rule_name())
            .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`")
            .with_repair(StructuredError::fix_pattern_repair(
                "C12",
                "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
            )));
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
        // Preserve plus written minus-side attributes constrains the match
        // (T235, SPEC 14.6 — same as fn hunks): sites carrying different
        // attributes are skipped, never silently rewritten.
        if hunk.attrs_policy() == AttrsPolicy::Preserve
            && !minus_attrs.is_empty()
            && !attrs_equal(&minus_attrs, &c.attrs)
        {
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
            .with_rule(&rule)
            .with_repair(StructuredError::internal_repair()));
        }
        edits.push(TextEdit::new(a, b, plus_name.clone()).with_label("rename"));
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
    fn explicit_doc_comment_mirrors_apply() {
        // T156: item parity with the T155 fn/general normalization —
        // `///` and single-line `/** */` mirrors compare like `#[doc]`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# attrs=explicit\n- /// Doc.\n- struct Old\n+ /// Doc.\n+ struct New\n";
        let out = apply(patch, "/// Doc.\nstruct Old {\n    x: i32,\n}\n").unwrap();
        assert!(out.contains("struct New {"), "got: {out}");
        assert!(out.contains("/// Doc."), "got: {out}");
        let block = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# attrs=explicit\n- /** Block doc. */\n- struct Old\n+ /** Block doc. */\n+ struct New\n";
        let out = apply(block, "/** Block doc. */\nstruct Old {\n    x: i32,\n}\n").unwrap();
        assert!(out.contains("struct New {"), "got: {out}");
        assert!(out.contains("/** Block doc. */"), "got: {out}");
        // Wrong doc text still fails instead of silently matching.
        let wrong = patch.replace("- /// Doc.", "- /// Other.");
        assert!(apply(&wrong, "/// Doc.\nstruct Old {\n    x: i32,\n}\n").is_err());
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
    fn impl_trait_side_matches() {
        // T19b: `impl Old for Foo` names Old as the trait — a bare
        // `impl Old` rename rewrites the trait-side ident too.
        let patch = rename_patch("impl Old", "impl New", "@@ -1,3 +1,3 @@");
        let src = "trait Old {}\nstruct Foo;\nimpl Old for Foo {}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("impl New for Foo"), "got: {out}");
        assert!(!out.contains("impl Old for Foo"), "got: {out}");
    }

    #[test]
    fn impl_trait_side_path_shapes() {
        // Generic, qualified, and negative trait paths: only the last
        // segment ident is replaced; arguments/qualifiers preserved.
        let cases = [
            (
                "impl<T> Old<T> for Foo {}\nstruct Foo;\n",
                "impl<T> New<T> for Foo {}",
            ),
            (
                "impl crate::Old for Foo {}\nstruct Foo;\n",
                "impl crate::New for Foo {}",
            ),
            (
                "impl !Old for Foo {}\nstruct Foo;\n",
                "impl !New for Foo {}",
            ),
        ];
        for (src, want) in cases {
            let patch = rename_patch("impl Old", "impl New", "@@ -1,1 +1,1 @@");
            let out = apply(&patch, src).unwrap();
            assert!(out.contains(want), "src {src:?}: got: {out}");
        }
    }

    #[test]
    fn impl_both_sides_match_with_allow_many() {
        // `impl Old for Old` names Old twice: two ident-span edits.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# allow=many\n- impl Old\n+ impl New\n";
        let out = apply(patch, "impl Old for Old {}\n").unwrap();
        assert!(out.contains("impl New for New"), "got: {out}");
    }

    #[test]
    fn impl_unrelated_trait_side_untouched() {
        // Only the named trait is rewritten; other headers do not match.
        let patch = rename_patch("impl Old", "impl New", "@@ -1,3 +1,3 @@");
        let src = "trait Other {}\nstruct Foo;\nimpl Other for Foo {}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
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
        // WI-4: terse hunk cites line 1 while the item lives on line 3 —
        // pattern-anchored drift applies it (uniqueness + disjointness).
        let src = "fn a() {}\nfn b() {}\nstruct Old;\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("struct New;"), "got: {out}");
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

    #[test]
    fn renames_block_level_struct() {
        // T23: items nested in fn bodies are collected by the syntactic walk.
        let patch = rename_patch("struct Local", "struct Renamed", "@@ -2,1 @@");
        let src = "fn outer() {\n    struct Local;\n    let _ = 1;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("struct Renamed;"), "got: {out}");
        assert!(out.contains("fn outer() {"));
        assert!(out.contains("let _ = 1;"));
        assert!(!out.contains("Local"));
    }

    #[test]
    fn allow_many_reaches_block_and_top_level() {
        // T23: one hunk, `# allow=many`, renames the top-level item and the
        // same-named local together (traversal + allow semantics compose).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=many\n# allow=many\n- struct Old\n+ struct New\n";
        let src = "struct Old;\nfn outer() {\n    struct Old;\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out.matches("struct New").count(), 2, "got: {out}");
        assert!(!out.contains("Old"));
    }

    #[test]
    fn preserve_written_attrs_constrain_and_mirror() {
        // T235: written minus-side attrs constrain the match even under
        // the default preserve policy (SPEC 14.6, fn parity via T165);
        // sites carrying different attrs are skipped, never rewritten.
        let mismatch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- /// Local thing.\n- struct Local\n+ /// Local thing.\n+ struct Remote\n";
        let err = apply(mismatch, "struct Local;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        // Mirrored attrs on a matching site apply byte-exact.
        let mirror = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- /// Local thing.\n- struct Local\n+ /// Local thing.\n+ struct Remote\n";
        let out = apply(mirror, "/// Local thing.\nstruct Local;\n").unwrap();
        assert!(
            out.contains("/// Local thing.\nstruct Remote;"),
            "got: {out}"
        );
        // Plus-side attrs differing from the minus side are rejected
        // instead of silently dropped (fn/general mirror-gate parity).
        let differ = mirror.replace("+ /// Local thing.", "+ /// Changed thing.");
        let err = apply(&differ, "/// Local thing.\nstruct Local;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("differ from matched attributes"), "got: {err}");
        // `# attrs=strip` skips the differ gate: the envelope is deleted.
        let strip = differ.replace("# lang=rust", "# attrs=strip\n# lang=rust");
        let out = apply(&strip, "/// Local thing.\nstruct Local;\n").unwrap();
        assert!(out.contains("struct Remote;"), "got: {out}");
        assert!(!out.contains("Local thing"), "got: {out}");
        // Plus-side attr omission is also a differ (T241, fn/general
        // parity): the plus side must mirror the minus side exactly
        // (SPEC 14.6), never silently drop.
        let drop = mirror.replace("+ /// Local thing.\n", "");
        let err = apply(&drop, "/// Local thing.\nstruct Local;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("differ from matched attributes"), "got: {err}");
    }

    #[test]
    fn renames_macro_rules_preserving_body_and_attrs() {
        // WI-7b: `macro_rules!` definitions rename the ident only.
        let patch = rename_patch("macro_rules! Old", "macro_rules! New", "@@ -1,4 +1,4 @@");
        let src = "#[macro_export]\nmacro_rules! Old {\n    ($x:expr) => { $x + 1 };\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("macro_rules! New {"), "got: {out}");
        assert!(out.contains("#[macro_export]"));
        assert!(out.contains("($x:expr) => { $x + 1 };"));
        assert!(!out.contains("macro_rules! Old"));
    }

    #[test]
    fn macro_rules_distractor_untouched() {
        let patch = rename_patch("macro_rules! Old", "macro_rules! New", "@@ -1,4 +1,4 @@");
        let src = "macro_rules! Old {\n    () => {};\n}\nmacro_rules! Other {\n    () => {};\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("macro_rules! New {"), "got: {out}");
        assert!(out.contains("macro_rules! Other {"), "got: {out}");
    }
}
