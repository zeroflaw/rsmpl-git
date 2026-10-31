//! Struct-field and enum-variant renames (SPEC 13.4, WI-9a).
//!
//! Authored shapes reuse the impl-context spelling family: a `struct S {`
//! (resp. `enum E {`) context header plus a member pair. Only the member
//! ident span is replaced; containers, visibility, types, discriminants,
//! and variant fields are preserved byte-for-byte — the item-rename
//! posture (`matcher_item`) scoped to one named container.
//!
//! ```diff
//!   struct S {
//! - old: T
//! + new: T
//! ```
//!
//! Field renames cover named fields only; unit/tuple structs and unions
//! never match (ordinary hunks). Variant renames cover unit, tuple, and
//! struct variants alike (the ident span is uniform); only the variant
//! ident is replaced.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_item::{check_attrs, span_bytes};

/// One member-rename candidate: the container item span plus the member
/// ident span. `container` names the `struct`/`enum` ident for messages.
struct MemberCandidate {
    member_ident: proc_macro2::Span,
    member_attrs: Vec<syn::Attribute>,
    /// Named-field type (field renames only; the precision gate compares
    /// it against the authored annotation).
    field_ty: Option<syn::Type>,
    container_start: u32,
    container_end: u32,
    anchor_line: u32,
    attr_start_line: u32,
}

/// Whitespace-normalized token rendering (same normalization the
/// classifier applies to the authored type, so spacing never matters).
fn norm_ty(ty: &syn::Type) -> String {
    quote::quote!(#ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Collect field candidates named `old` inside `struct S` (named fields
/// only). Traversal mirrors `matcher_item` (inline mods, block-level
/// items); macro bodies stay excluded.
fn collect_fields(
    items: &[syn::Item],
    struct_name: &str,
    old: &str,
    out: &mut Vec<MemberCandidate>,
) {
    struct Collector<'a> {
        struct_name: &'a str,
        old: &'a str,
        found: Vec<MemberCandidate>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if let syn::Item::Struct(s) = it {
                if s.ident == self.struct_name {
                    if let syn::Fields::Named(fields) = &s.fields {
                        for f in &fields.named {
                            if let Some(ident) = &f.ident {
                                if ident == self.old {
                                    self.found.push(MemberCandidate {
                                        member_ident: ident.span(),
                                        member_attrs: f.attrs.clone(),
                                        field_ty: Some(f.ty.clone()),
                                        container_start: s.span().start().line as u32,
                                        container_end: s.span().end().line as u32,
                                        anchor_line: ident.span().start().line as u32,
                                        attr_start_line: f.span().start().line as u32,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            visit::visit_item(self, it);
        }
    }
    let mut collector = Collector {
        struct_name,
        old,
        found: Vec::new(),
    };
    for it in items {
        collector.visit_item(it);
    }
    out.extend(collector.found);
}

/// Collect variant candidates named `old` inside `enum E` (unit, tuple,
/// and struct variants alike — the ident span is uniform).
fn collect_variants(
    items: &[syn::Item],
    enum_name: &str,
    old: &str,
    out: &mut Vec<MemberCandidate>,
) {
    struct Collector<'a> {
        enum_name: &'a str,
        old: &'a str,
        found: Vec<MemberCandidate>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if let syn::Item::Enum(e) = it {
                if e.ident == self.enum_name {
                    for v in &e.variants {
                        if v.ident == self.old {
                            self.found.push(MemberCandidate {
                                member_ident: v.ident.span(),
                                member_attrs: v.attrs.clone(),
                                field_ty: None,
                                container_start: e.span().start().line as u32,
                                container_end: e.span().end().line as u32,
                                anchor_line: v.ident.span().start().line as u32,
                                attr_start_line: v.span().start().line as u32,
                            });
                        }
                    }
                }
            }
            visit::visit_item(self, it);
        }
    }
    let mut collector = Collector {
        enum_name,
        old,
        found: Vec::new(),
    };
    for it in items {
        collector.visit_item(it);
    }
    out.extend(collector.found);
}

/// Scope filtering (SPEC 17.3): container envelope for range modes, the
/// member ident line as the anchor — the `matcher_item` posture scoped
/// to one container.
fn in_scope(c: &MemberCandidate, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.container_start >= old_start && c.container_end <= end.max(old_start),
        Scope::Anchor => c.anchor_line >= old_start && c.anchor_line <= end.max(old_start),
        Scope::Overlap => c.container_start <= end.max(old_start) && c.container_end >= old_start,
        Scope::Exact => old_len > 0 && c.container_start == old_start && c.container_end == end,
        Scope::File => true,
    }
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
}

/// Shared ident-span rewrite over collected candidates (attrs policies
/// mirror `matcher_item`, including the mirror gate checked by callers
/// via `check_attrs`).
#[allow(clippy::too_many_arguments)]
fn apply_candidates(
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    candidates: &mut [MemberCandidate],
    old: &str,
    new: &str,
    pattern_attrs: &[String],
    extra_check: &dyn Fn(&MemberCandidate) -> Result<(), StructuredError>,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let rule = hunk.rule_name();
    let file = "";
    candidates.sort_by_key(|c| (c.container_start, c.anchor_line));
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    for c in candidates.iter() {
        if !in_scope(c, hunk.scope(), old_start, old_len) {
            continue;
        }
        if hunk.attrs_policy() == AttrsPolicy::Preserve
            && !pattern_attrs.is_empty()
            && !attrs_match(pattern_attrs, &c.member_attrs)
        {
            continue;
        }
        check_attrs(
            &c.member_attrs,
            hunk.attrs_policy(),
            pattern_attrs,
            &rule,
            file,
        )?;
        extra_check(c)?;
        let (a, b) = span_bytes(original, index, c.member_ident)
            .ok_or_else(|| range_err(&rule, c.member_ident.start().line as u32))?;
        if original.get(a..b) != Some(old) {
            return Err(StructuredError::new(
                ErrorCode::MatchConflict,
                "member name span mismatch (file changed during matching?)",
            )
            .with_rule(&rule));
        }
        edits.push(TextEdit::new(a, b, new.to_string()).with_label("rename"));
        if hunk.attrs_policy() == AttrsPolicy::Strip && !c.member_attrs.is_empty() {
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
            break;
        }
    }
    Ok(matched)
}

fn norm_attr_text(s: &str) -> String {
    s.split_whitespace().collect()
}

fn attrs_match(pattern_attrs: &[String], attrs: &[syn::Attribute]) -> bool {
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

/// SPEC 14.6 mirror parity with item renames: a plus side writing
/// attributes the minus side omitted is an ambiguous merge (fail fast),
/// and differing plus-side attributes are never silently dropped
/// (`# attrs=strip` skips the differ gate — the envelope is deleted).
fn check_attr_mirror(
    hunk: &RsmplHunk,
    minus_attrs: &[String],
    plus_attrs: &[String],
) -> Result<(), StructuredError> {
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
    if hunk.attrs_policy() != AttrsPolicy::Strip
        && !minus_attrs.is_empty()
        && !attrs_text_equal(minus_attrs, plus_attrs)
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
    Ok(())
}

/// Apply a struct-field rename hunk. The target field's type must render
/// equal to the authored type (precision-first: a wrong annotation skips
/// the site, never rewrites under a lie).
pub fn apply_field_rename_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (struct_name, old, new, ty, minus_attrs, plus_attrs) = match &compiled.kind {
        PatternKind::FieldRename {
            struct_name,
            old,
            new,
            ty,
            minus_attrs,
            plus_attrs,
        } => (
            struct_name.clone(),
            old.clone(),
            new.clone(),
            ty.clone(),
            minus_attrs.clone(),
            plus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected field rename pattern".to_string(),
            ));
        }
    };
    check_attr_mirror(hunk, &minus_attrs, &plus_attrs)?;
    let mut candidates = Vec::new();
    collect_fields(&parsed.items, &struct_name, &old, &mut candidates);
    // Precision gate: the target field's type must render equal to the
    // authored annotation (a wrong annotation skips the site, never
    // rewrites under a lie).
    let check = |c: &MemberCandidate| {
        match &c.field_ty {
        Some(field_ty) if norm_ty(field_ty) == ty => Ok(()),
        _ => Err(StructuredError::new(
            ErrorCode::MatchConflict,
            format!(
                "field `{struct_name}::{old}` has a different type than the authored annotation (expected `{ty}`)"
            ),
        )
        .with_rule(hunk.rule_name())
        .with_suggestion("write the field's real type on both sides, or use an ordinary hunk")),
    }
    };
    apply_candidates(
        original,
        index,
        hunk,
        &mut candidates,
        &old,
        &new,
        &minus_attrs,
        &check,
        edits,
    )
}

/// Apply an enum-variant rename hunk. Discriminants and variant fields
/// are preserved; only the variant ident span is replaced.
pub fn apply_variant_rename_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (enum_name, old, new, minus_attrs, plus_attrs) = match &compiled.kind {
        PatternKind::VariantRename {
            enum_name,
            old,
            new,
            minus_attrs,
            plus_attrs,
        } => (
            enum_name.clone(),
            old.clone(),
            new.clone(),
            minus_attrs.clone(),
            plus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected variant rename pattern".to_string(),
            ));
        }
    };
    check_attr_mirror(hunk, &minus_attrs, &plus_attrs)?;
    let mut candidates = Vec::new();
    collect_variants(&parsed.items, &enum_name, &old, &mut candidates);
    let noop = |_: &MemberCandidate| Ok(());
    apply_candidates(
        original,
        index,
        hunk,
        &mut candidates,
        &old,
        &new,
        &minus_attrs,
        &noop,
        edits,
    )
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

    const FIELD_CTX: &str = "  struct S {\n";

    fn field_patch(minus: &str, plus: &str) -> String {
        field_patch_at(minus, plus, "@@ -1,4 +1,4 @@")
    }

    fn field_patch_at(minus: &str, plus: &str, range: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=r\n{FIELD_CTX}- {minus}\n+ {plus}\n"
        )
    }

    const VARIANT_CTX: &str = "  enum E {\n";

    fn variant_patch(minus: &str, plus: &str) -> String {
        variant_patch_at(minus, plus, "@@ -1,3 +1,3 @@")
    }

    fn variant_patch_at(minus: &str, plus: &str, range: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=r\n{VARIANT_CTX}- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn renames_field_preserving_type_and_container() {
        let patch = field_patch("old: u32", "new: u32");
        let src = "struct S {\n    old: u32,\n    keep: bool,\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("new: u32,"), "got: {out}");
        assert!(out.contains("keep: bool,"));
        assert!(!out.contains("old: u32"));
    }

    #[test]
    fn field_wrong_type_annotation_skips() {
        let patch = field_patch_at("old: bool", "new: bool", "@@ -1,3 +1,3 @@");
        let src = "struct S {\n    old: u32,\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn field_other_container_untouched() {
        let patch = field_patch_at("old: u32", "new: u32", "@@ -1,2 +1,2 @@");
        let src = "struct S {\n    x: u32,\n}\nstruct T {\n    old: u32,\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn field_tuple_struct_never_matches() {
        let patch = field_patch_at("old: u32", "new: u32", "@@ -1,1 +1,1 @@");
        let src = "struct S(u32);\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn renames_variant_preserving_discriminant() {
        let patch = variant_patch_at("Old", "New", "@@ -1,4 +1,4 @@");
        let src = "enum E {\n    Old = 3,\n    Keep,\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("New = 3,"), "got: {out}");
        assert!(out.contains("Keep,"));
        assert!(!out.contains("Old"));
    }

    #[test]
    fn renames_tuple_variant_preserving_fields() {
        let patch = variant_patch("Old", "New");
        let src = "enum E {\n    Old(u32, bool),\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("New(u32, bool),"), "got: {out}");
    }

    #[test]
    fn renames_struct_variant_preserving_fields() {
        let patch = variant_patch("Old", "New");
        let src = "enum E {\n    Old { x: u32 },\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("New { x: u32 },"), "got: {out}");
    }

    #[test]
    fn field_mirrored_attrs_apply() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n  struct S {\n- /// Doc.\n- old: u32\n+ /// Doc.\n+ new: u32\n";
        let src = "struct S {\n    /// Doc.\n    old: u32,\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("new: u32,"), "got: {out}");
        assert!(out.contains("/// Doc."), "got: {out}");
    }

    #[test]
    fn field_plus_attr_omission_rejected() {
        // T-mirror: the plus side must mirror the minus side exactly.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n  struct S {\n- /// Doc.\n- old: u32\n+ new: u32\n";
        let src = "struct S {\n    /// Doc.\n    old: u32,\n}\n";
        let err = apply(patch, src).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }

    #[test]
    fn variant_other_enum_untouched() {
        let patch = variant_patch_at("Old", "New", "@@ -1,2 +1,2 @@");
        let src = "enum E {\n    Keep,\n}\nenum F {\n    Old,\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }
}
