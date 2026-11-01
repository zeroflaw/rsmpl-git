//! WI-5 slice 2b: struct rename + field-type evolution joint.
//!
//! Atomic application of the composed spelling (bare-def + member, no
//! container header — SPEC 14.8): the `ItemStruct Old` ident is renamed
//! to `New` and the named field's type span is replaced, in one matcher
//! pass over the pre-image. Twin `Old` definitions veto (0 matches —
//! the raiser's probe owns the error, never a partial rename). Field
//! vis must agree (subset posture, like the pure field-evo matcher).

use std::collections::HashMap;

use syn::spanned::Spanned;

use rsmpl_patch::{Allow, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_item::{attrs_equal, check_attrs, span_bytes};

struct JointCandidate<'ast> {
    ident_span: proc_macro2::Span,
    field_ty: &'ast syn::Type,
    field_vis_norm: String,
    field_attrs: Vec<syn::Attribute>,
    container_start: u32,
    container_end: u32,
    anchor_line: u32,
}

fn collect<'ast>(
    items: &'ast [syn::Item],
    old_container: &str,
    field: &str,
    out: &mut Vec<JointCandidate<'ast>>,
) {
    use syn::visit::{visit_item, Visit};
    struct V<'a, 'ast> {
        old: &'a str,
        field: &'a str,
        out: &'a mut Vec<JointCandidate<'ast>>,
    }
    impl<'ast> Visit<'ast> for V<'_, 'ast> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if let syn::Item::Struct(s) = it {
                if s.ident == self.old {
                    if let syn::Fields::Named(fields) = &s.fields {
                        for f in &fields.named {
                            if f.ident.as_ref().is_some_and(|i| i == self.field) {
                                self.out.push(JointCandidate {
                                    ident_span: s.ident.span(),
                                    field_ty: &f.ty,
                                    field_vis_norm: norm_vis(&f.vis),
                                    field_attrs: f.attrs.clone(),
                                    container_start: s.span().start().line as u32,
                                    container_end: s.span().end().line as u32,
                                    anchor_line: f
                                        .ident
                                        .as_ref()
                                        .map(|i| i.span().start().line as u32)
                                        .unwrap_or(s.span().start().line as u32),
                                });
                            }
                        }
                    }
                }
            }
            visit_item(self, it);
        }
    }
    let mut v = V {
        old: old_container,
        field,
        out,
    };
    for it in items {
        v.visit_item(it);
    }
}

fn norm_vis(v: &syn::Visibility) -> String {
    quote::quote!(#v)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn norm_vis_of(s: &str) -> String {
    s.split_whitespace().collect()
}

fn in_scope(
    c_start: u32,
    c_end: u32,
    anchor: u32,
    scope: Scope,
    old_start: u32,
    old_len: u32,
) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c_start >= old_start && c_end <= end.max(old_start),
        Scope::Anchor => anchor >= old_start && anchor <= end.max(old_start),
        Scope::Overlap => c_start <= end.max(old_start) && c_end >= old_start,
        Scope::Exact => old_len > 0 && c_start == old_start && c_end == end,
        Scope::File => true,
    }
}

/// Apply the joint hunk. Returns (matches, bindings).
pub fn apply_struct_rename_field_evo_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (old_container, new_container, field, vis, minus_sub, plus_raw, placeholders, minus_attrs) =
        match &compiled.kind {
            PatternKind::StructRenameFieldEvo {
                old_container,
                new_container,
                field,
                vis,
                minus_sub,
                plus_raw,
                placeholders,
                minus_attrs,
                ..
            } => (
                old_container.clone(),
                new_container.clone(),
                field.clone(),
                vis.clone(),
                minus_sub.clone(),
                plus_raw.clone(),
                placeholders.clone(),
                minus_attrs.clone(),
            ),
            _ => {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "expected struct-rename-field-evolution pattern".to_string(),
                ));
            }
        };
    let rule = hunk.rule_name();
    let pat_ty: syn::Type = syn::parse_str(&minus_sub).map_err(|e| {
        StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid joint field type pattern: {e}"),
        )
        .with_rule(rule.clone())
    })?;
    syn::parse_str::<syn::Type>(&plus_raw).map_err(|e| {
        StructuredError::new(
            ErrorCode::PatternConflict,
            format!("substituted joint field type does not parse: {e}"),
        )
        .with_rule(rule.clone())
    })?;
    let mut candidates = Vec::new();
    collect(&parsed.items, &old_container, &field, &mut candidates);
    candidates.sort_by_key(|c| (c.container_start, c.anchor_line));
    let reverse: HashMap<String, String> = placeholders
        .iter()
        .map(|(n, ph)| (ph.clone(), n.clone()))
        .collect();
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    let mut all_bindings = Vec::new();
    for c in &candidates {
        if !in_scope(
            c.container_start,
            c.container_end,
            c.anchor_line,
            hunk.scope(),
            old_start,
            old_len,
        ) {
            continue;
        }
        if hunk.attrs_policy() == rsmpl_patch::AttrsPolicy::Preserve
            && !minus_attrs.is_empty()
            && !attrs_equal(&minus_attrs, &c.field_attrs)
        {
            continue;
        }
        check_attrs(&c.field_attrs, hunk.attrs_policy(), &minus_attrs, &rule, "")?;
        if norm_vis_of(&c.field_vis_norm) != norm_vis_of(&vis) {
            continue;
        }
        let mut cx =
            crate::matcher_general::MatchCx::with_reverse(original, index, reverse.clone());
        if !crate::matcher_types::match_type(&pat_ty, c.field_ty, &mut cx) {
            continue;
        }
        let (a, b) = span_bytes(original, index, c.ident_span)
            .ok_or_else(|| range_err(&rule, c.anchor_line))?;
        edits.push(TextEdit::new(a, b, new_container.clone()).with_label("rename"));
        let (a, b) = span_bytes(original, index, c.field_ty.span())
            .ok_or_else(|| range_err(&rule, c.anchor_line))?;
        edits.push(TextEdit::new(a, b, plus_raw.clone()).with_label("type"));
        all_bindings.push(cx.bindings);
        matched += 1;
        if hunk.allow() == Allow::One && matched > 1 {
            break;
        }
    }
    Ok((matched, all_bindings))
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_repair(StructuredError::internal_repair())
}

#[cfg(test)]
mod tests {

    fn apply(patch: &str, pre: &str) -> String {
        let parsed_patch = rsmpl_patch::parse_patch(patch).expect("must parse");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine_apply(&parsed_patch, &mut files);
        files["f.rs"].clone()
    }

    fn rsmpl_engine_apply(
        parsed: &rsmpl_patch::Patch,
        files: &mut std::collections::HashMap<String, String>,
    ) {
        crate::apply_patch_to_tree(files, parsed).expect("must apply");
    }

    fn joint_patch() -> String {
        "diff --git f.rs\n@@ -1,1 @@ rule=r1\n# sweep\n# follow=uses\n- struct Old\n+ struct New\n- x: u32\n+ x: u64\n"
            .to_string()
    }

    #[test]
    fn joint_renames_container_and_field_with_follow() {
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let post = apply(&joint_patch(), pre);
        assert!(post.contains("struct New {"), "got:\n{post}");
        assert!(post.contains("x: u64,"), "got:\n{post}");
        assert!(post.contains("fn f(s: New) {"), "got:\n{post}");
        assert!(!post.contains("Old"), "got:\n{post}");
    }

    #[test]
    fn twin_containers_both_match_so_raiser_must_veto() {
        // Two `struct Old` definitions: the sweep matches both (the
        // engine has no uniqueness guard here — the raiser's
        // `count_kind_defs` veto owns it and never emits the joint).
        // Pinned so a future engine guard cannot silently change the
        // layering.
        let pre = "struct Old {\n    x: u32,\n}\nstruct Old {\n    x: u32,\n}\n";
        let post = apply(&joint_patch(), pre);
        assert_eq!(post.matches("struct New").count(), 2, "got:\n{post}");
        assert!(!post.contains("Old"), "got:\n{post}");
    }

    #[test]
    fn vis_mismatch_skips_member() {
        // File field is `pub x` but the pattern says bare: subset
        // posture skips the member (0 matches → apply errors, never a
        // lie under a wrong vis).
        let pre = "struct Old {\n    pub x: u32,\n}\n";
        let parsed_patch = rsmpl_patch::parse_patch(&joint_patch()).expect("must parse");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        assert!(crate::apply_patch_to_tree(&mut files, &parsed_patch).is_err());
    }
}
