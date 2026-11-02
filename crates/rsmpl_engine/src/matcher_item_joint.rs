//! WI-5 slice 2b (+next): struct/enum rename + member-evolution joint.
//!
//! Atomic application of the composed spelling (bare-def + member, no
//! container header — SPEC 14.8): the `ItemStruct`/`ItemEnum Old` ident
//! is renamed to `New` and the named member's evolved region span(s)
//! are replaced, in one matcher pass over the pre-image. Twin `Old`
//! definitions veto (0 matches — the raiser's probe owns the error,
//! never a partial rename). Field vis must agree (subset posture, like
//! the pure field-evo matcher).

use std::collections::HashMap;

use syn::spanned::Spanned;

use rsmpl_patch::{Allow, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind, VariantPayload};

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
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "item-joint-struct-dispatch-template",
                    "write `- struct Old` / `+ struct New` plus `- x: Ty` / `+ x: Ty2` (exactly one field-type delta, no container header; SPEC 14.8)",
                )));
            }
        };
    let rule = hunk.rule_name();
    let pat_ty: syn::Type = syn::parse_str(&minus_sub).map_err(|e| {
        StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid joint field type pattern: {e}"),
        )
        .with_rule(rule.clone())
        .with_repair(StructuredError::fix_pattern_repair(
            "item-joint-struct-field-template",
            "write the `- x: Ty` side as a valid Rust type (e.g. `- x: u32`), field name concrete and preserved",
        ))
    })?;
    syn::parse_str::<syn::Type>(&plus_raw).map_err(|e| {
        StructuredError::new(
            ErrorCode::PatternConflict,
            format!("substituted joint field type does not parse: {e}"),
        )
        .with_rule(rule.clone())
        .with_repair(StructuredError::fix_pattern_repair(
            "item-joint-struct-field-plus-template",
            "write the `+ x: Ty` side as a valid Rust type (e.g. `+ x: u64`), field name concrete and preserved",
        ))
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

/// One enum-joint candidate: the enum ident plus the named variant's
/// payload spans.
struct EnumJointCandidate<'ast> {
    ident_span: proc_macro2::Span,
    tuple_tys: Vec<&'ast syn::Type>,
    struct_fields: Vec<(String, &'ast syn::Type)>,
    discriminant: Option<&'ast syn::Expr>,
    is_tuple: bool,
    is_struct: bool,
    variant_attrs: Vec<syn::Attribute>,
    container_start: u32,
    container_end: u32,
    anchor_line: u32,
}

fn collect_enum_joint<'ast>(
    items: &'ast [syn::Item],
    old_container: &str,
    variant: &str,
    out: &mut Vec<EnumJointCandidate<'ast>>,
) {
    use syn::visit::{visit_item, Visit};
    struct V<'a, 'ast> {
        old: &'a str,
        variant: &'a str,
        out: &'a mut Vec<EnumJointCandidate<'ast>>,
    }
    impl<'ast> Visit<'ast> for V<'_, 'ast> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if let syn::Item::Enum(e) = it {
                if e.ident == self.old {
                    for v in &e.variants {
                        if v.ident == self.variant {
                            let (tuple_tys, struct_fields, is_tuple, is_struct) = match &v.fields {
                                syn::Fields::Unnamed(u) => (
                                    u.unnamed.iter().map(|f| &f.ty).collect(),
                                    Vec::new(),
                                    true,
                                    false,
                                ),
                                syn::Fields::Named(n) => (
                                    Vec::new(),
                                    n.named
                                        .iter()
                                        .filter_map(|f| {
                                            f.ident.as_ref().map(|i| (i.to_string(), &f.ty))
                                        })
                                        .collect(),
                                    false,
                                    true,
                                ),
                                syn::Fields::Unit => (Vec::new(), Vec::new(), false, false),
                            };
                            self.out.push(EnumJointCandidate {
                                ident_span: e.ident.span(),
                                tuple_tys,
                                struct_fields,
                                discriminant: v.discriminant.as_ref().map(|(_, e)| e),
                                is_tuple,
                                is_struct,
                                variant_attrs: v.attrs.clone(),
                                container_start: e.span().start().line as u32,
                                container_end: e.span().end().line as u32,
                                anchor_line: v.ident.span().start().line as u32,
                            });
                        }
                    }
                }
            }
            visit_item(self, it);
        }
    }
    let mut v = V {
        old: old_container,
        variant,
        out,
    };
    for it in items {
        v.visit_item(it);
    }
}

/// Apply the enum joint hunk: rename the `ItemEnum Old` ident to `New`
/// and replace the evolved variant region span(s), in one pass.
pub fn apply_enum_rename_variant_evo_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (old_container, new_container, variant, payload, minus_attrs) = match &compiled.kind {
        PatternKind::EnumRenameVariantEvo {
            old_container,
            new_container,
            variant,
            payload,
            minus_attrs,
            ..
        } => (
            old_container.clone(),
            new_container.clone(),
            variant.clone(),
            payload.clone(),
            minus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected enum-rename-variant-evolution pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "item-joint-enum-dispatch-template",
                "write `- enum Old` / `+ enum New` plus one variant delta (`- A(u32)` / `+ A(u64)`, `- A { x: u32 }` / `+ A { x: u64 }`, or `- A = 1` / `+ A = 2`; no container header; SPEC 14.8)",
            )));
        }
    };
    let rule = hunk.rule_name();
    let (minus_subs, plus_raws, disc_minus, disc_plus, placeholders, mode) = match &payload {
        VariantPayload::Tuple {
            minus_subs,
            plus_raws,
            placeholders,
        } => (
            minus_subs.clone(),
            plus_raws.clone(),
            None,
            None,
            placeholders.clone(),
            "tuple",
        ),
        VariantPayload::Struct {
            minus_subs,
            plus_raws,
            placeholders: sph,
            ..
        } => (
            minus_subs.clone(),
            plus_raws.clone(),
            None,
            None,
            sph.clone(),
            "struct",
        ),
        VariantPayload::Discriminant {
            minus_sub,
            plus_raw,
            placeholders,
        } => (
            vec![minus_sub.clone()],
            vec![plus_raw.clone()],
            Some(minus_sub.clone()),
            Some(plus_raw.clone()),
            placeholders.clone(),
            "disc",
        ),
    };
    // Concrete-only v1: placeholders stay empty, but wire the reverse
    // map anyway (matches the struct-joint shape).
    let mut candidates = Vec::new();
    collect_enum_joint(&parsed.items, &old_container, &variant, &mut candidates);
    candidates.sort_by_key(|c| (c.container_start, c.anchor_line));
    let reverse: HashMap<String, String> = placeholders
        .iter()
        .map(|(n, ph)| (ph.clone(), n.clone()))
        .collect();
    let pat_tys: Vec<syn::Type> = if mode == "disc" {
        Vec::new()
    } else {
        minus_subs
            .iter()
            .map(|s| {
                syn::parse_str::<syn::Type>(s).map_err(|e| {
                    StructuredError::new(
                        ErrorCode::PatternConflict,
                        format!("invalid joint variant payload type pattern: {e}"),
                    )
                    .with_rule(rule.clone())
                    .with_repair(StructuredError::fix_pattern_repair(
                        "item-joint-variant-payload-template",
                        "write the `- A(...)` side payload as valid Rust types (e.g. `- A(u32)`, `- A { x: u32 }`), variant name concrete and preserved",
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if mode != "disc" {
        for s in &plus_raws {
            syn::parse_str::<syn::Type>(s).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("substituted joint variant payload type does not parse: {e}"),
                )
                .with_rule(rule.clone())
                .with_repair(StructuredError::fix_pattern_repair(
                    "item-joint-variant-payload-plus-template",
                    "write the `+ A(...)` side payload as valid Rust types (e.g. `+ A(u64)`, `+ A { x: u64 }`), variant name concrete and preserved",
                ))
            })?;
        }
    }
    let pat_disc: Option<syn::Expr> = if mode == "disc" {
        Some(
            syn::parse_str::<syn::Expr>(disc_minus.as_ref().expect("disc")).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("invalid joint discriminant pattern: {e}"),
                )
                .with_rule(rule.clone())
                .with_repair(StructuredError::fix_pattern_repair(
                    "item-joint-discriminant-template",
                    "write the `- A = N` side discriminant as a valid Rust expression (e.g. `- A = 1`), variant name concrete and preserved",
                ))
            })?,
        )
    } else {
        None
    };
    if mode == "disc" {
        if let Some(p) = &disc_plus {
            syn::parse_str::<syn::Expr>(p).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("substituted joint discriminant does not parse: {e}"),
                )
                .with_rule(rule.clone())
                .with_repair(StructuredError::fix_pattern_repair(
                    "item-joint-discriminant-plus-template",
                    "write the `+ A = N` side discriminant as a valid Rust expression (e.g. `+ A = 2`), variant name concrete and preserved",
                ))
            })?;
        }
    }
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
            && !attrs_equal(&minus_attrs, &c.variant_attrs)
        {
            continue;
        }
        check_attrs(
            &c.variant_attrs,
            hunk.attrs_policy(),
            &minus_attrs,
            &rule,
            "",
        )?;
        let mut cx =
            crate::matcher_general::MatchCx::with_reverse(original, index, reverse.clone());
        let ok = match mode {
            "tuple" => {
                if !c.is_tuple || c.tuple_tys.len() != pat_tys.len() {
                    false
                } else {
                    pat_tys
                        .iter()
                        .zip(c.tuple_tys.iter())
                        .all(|(p, t)| crate::matcher_types::match_type(p, t, &mut cx))
                }
            }
            "struct" => {
                if !c.is_struct {
                    false
                } else {
                    let want: Vec<String> = match &payload {
                        VariantPayload::Struct { fields, .. } => fields.clone(),
                        _ => Vec::new(),
                    };
                    let have: Vec<String> =
                        c.struct_fields.iter().map(|(n, _)| n.clone()).collect();
                    if want != have {
                        false
                    } else {
                        pat_tys
                            .iter()
                            .zip(c.struct_fields.iter().map(|(_, t)| *t))
                            .all(|(p, t)| crate::matcher_types::match_type(p, t, &mut cx))
                    }
                }
            }
            _ => match (&pat_disc, c.discriminant) {
                (Some(p), Some(t)) => crate::matcher_general::match_expr(p, t, &mut cx),
                _ => false,
            },
        };
        if !ok {
            continue;
        }
        let (a, b) = span_bytes(original, index, c.ident_span)
            .ok_or_else(|| range_err(&rule, c.anchor_line))?;
        edits.push(TextEdit::new(a, b, new_container.clone()).with_label("rename"));
        match mode {
            "tuple" => {
                for (i, target) in c.tuple_tys.iter().enumerate() {
                    let (a, b) = span_bytes(original, index, target.span())
                        .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                    edits.push(TextEdit::new(a, b, plus_raws[i].clone()).with_label("payload"));
                }
            }
            "struct" => {
                for (i, (_, target)) in c.struct_fields.iter().enumerate() {
                    let (a, b) = span_bytes(original, index, target.span())
                        .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                    edits.push(TextEdit::new(a, b, plus_raws[i].clone()).with_label("payload"));
                }
            }
            _ => {
                let target = c.discriminant.expect("checked above");
                let (a, b) = span_bytes(original, index, target.span())
                    .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                edits.push(
                    TextEdit::new(a, b, disc_plus.clone().expect("disc")).with_label("payload"),
                );
            }
        }
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

    fn enum_joint_patch(member_minus: &str, member_plus: &str) -> String {
        format!(
            "diff --git f.rs\n@@ -1,1 @@ rule=r1\n# sweep\n# follow=uses\n- enum Old\n+ enum New\n- {member_minus}\n+ {member_plus}\n"
        )
    }

    #[test]
    fn enum_joint_tuple_renames_container_and_payload_with_follow() {
        let pre = "enum Old {\n    A(u32),\n}\nfn f(x: Old) {\n}\n";
        let post = apply(&enum_joint_patch("A(u32)", "A(u64)"), pre);
        assert!(post.contains("enum New"), "got:\n{post}");
        assert!(post.contains("A(u64)"), "got:\n{post}");
        assert!(post.contains("fn f(x: New)"), "got:\n{post}");
        assert!(!post.contains("Old"), "got:\n{post}");
    }

    #[test]
    fn enum_joint_discriminant_applies() {
        let pre = "enum Old {\n    A = 1,\n}\nfn f(x: Old) {\n}\n";
        let post = apply(&enum_joint_patch("A = 1", "A = 2"), pre);
        assert!(post.contains("enum New"), "got:\n{post}");
        assert!(post.contains("A = 2"), "got:\n{post}");
        assert!(!post.contains("Old"), "got:\n{post}");
    }

    #[test]
    fn enum_joint_struct_payload_applies() {
        let pre = "enum Old {\n    A { x: u32 },\n}\nfn f(x: Old) {\n}\n";
        let post = apply(&enum_joint_patch("A { x: u32 }", "A { x: u64 }"), pre);
        assert!(post.contains("enum New"), "got:\n{post}");
        assert!(post.contains("x: u64"), "got:\n{post}");
        assert!(!post.contains("Old"), "got:\n{post}");
    }

    #[test]
    fn enum_joint_twin_containers_both_match() {
        let pre = "enum Old {\n    A(u32),\n}\nenum Old {\n    A(u32),\n}\n";
        let post = apply(&enum_joint_patch("A(u32)", "A(u64)"), pre);
        assert_eq!(post.matches("enum New").count(), 2, "got:\n{post}");
        assert!(!post.contains("Old"), "got:\n{post}");
    }

    fn wi18_joint_struct_setup() -> (
        rsmpl_patch::RsmplHunk,
        rsmpl_pattern::CompiledPattern,
        syn::File,
        crate::lines::LineIndex,
        String,
    ) {
        let mut parsed_patch = rsmpl_patch::parse_patch(&joint_patch()).expect("test patch parses");
        let hunk = match parsed_patch.files.remove(0).hunks.remove(0) {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&hunk).expect("valid joint compiles");
        let src = "struct Old {\n    x: u32,\n}\n".to_string();
        let parsed: syn::File = syn::parse_str(&src).expect("src parses");
        let index = crate::lines::LineIndex::new(&src);
        (hunk, compiled, parsed, index, src)
    }

    fn wi18_joint_enum_setup(
        member_minus: &str,
        member_plus: &str,
        src: &str,
    ) -> (
        rsmpl_patch::RsmplHunk,
        rsmpl_pattern::CompiledPattern,
        syn::File,
        crate::lines::LineIndex,
        String,
    ) {
        let mut parsed_patch =
            rsmpl_patch::parse_patch(&enum_joint_patch(member_minus, member_plus))
                .expect("test patch parses");
        let hunk = match parsed_patch.files.remove(0).hunks.remove(0) {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&hunk).expect("valid joint compiles");
        let src = src.to_string();
        let parsed: syn::File = syn::parse_str(&src).expect("src parses");
        let index = crate::lines::LineIndex::new(&src);
        (hunk, compiled, parsed, index, src)
    }

    fn wi18_joint_repair(err: &rsmpl_patch::StructuredError) -> (String, String) {
        let repair = err
            .repair
            .as_ref()
            .expect("WI-18 slice 6: must carry repair");
        let code = repair
            .get("code")
            .and_then(|v| v.as_str())
            .expect("code str")
            .to_string();
        let write_instead = repair
            .get("write_instead")
            .and_then(|v| v.as_str())
            .expect("template str")
            .to_string();
        assert_ne!(code, "pattern", "must beat the generic filler");
        (code, write_instead)
    }

    #[test]
    fn wi18_slice6_struct_dispatch_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_joint_struct_setup();
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err = super::apply_struct_rename_field_evo_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("wrong kind must fail");
        assert_eq!(
            err.message,
            "expected struct-rename-field-evolution pattern"
        );
        let (code, w) = wi18_joint_repair(&err);
        assert_eq!(code, "item-joint-struct-dispatch-template");
        assert!(w.contains("`- struct Old`"), "got: {w}");
    }

    #[test]
    fn wi18_slice6_struct_field_minus_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_joint_struct_setup();
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::StructRenameFieldEvo { minus_sub, .. } => {
                *minus_sub = "u32 {{{".to_string();
            }
            _ => panic!("want struct joint"),
        }
        let mut edits = Vec::new();
        let err = super::apply_struct_rename_field_evo_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("invalid minus type must fail");
        assert!(
            err.message.starts_with("invalid joint field type pattern"),
            "got: {}",
            err.message
        );
        let (code, w) = wi18_joint_repair(&err);
        assert_eq!(code, "item-joint-struct-field-template");
        assert!(w.contains("`- x: u32`"), "got: {w}");
    }

    #[test]
    fn wi18_slice6_struct_field_plus_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_joint_struct_setup();
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::StructRenameFieldEvo { plus_raw, .. } => {
                *plus_raw = "u64 {{{".to_string();
            }
            _ => panic!("want struct joint"),
        }
        let mut edits = Vec::new();
        let err = super::apply_struct_rename_field_evo_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("invalid plus type must fail");
        assert!(
            err.message
                .starts_with("substituted joint field type does not parse"),
            "got: {}",
            err.message
        );
        let (code, w) = wi18_joint_repair(&err);
        assert_eq!(code, "item-joint-struct-field-plus-template");
        assert!(w.contains("`+ x: u64`"), "got: {w}");
    }

    #[test]
    fn wi18_slice6_enum_dispatch_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) =
            wi18_joint_enum_setup("A(u32)", "A(u64)", "enum Old {\n    A(u32),\n}\n");
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err = super::apply_enum_rename_variant_evo_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("wrong kind must fail");
        assert_eq!(
            err.message,
            "expected enum-rename-variant-evolution pattern"
        );
        let (code, w) = wi18_joint_repair(&err);
        assert_eq!(code, "item-joint-enum-dispatch-template");
        assert!(w.contains("`- enum Old`"), "got: {w}");
    }

    #[test]
    fn wi18_slice6_variant_payload_minus_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) =
            wi18_joint_enum_setup("A(u32)", "A(u64)", "enum Old {\n    A(u32),\n}\n");
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::EnumRenameVariantEvo { payload, .. } => match payload {
                rsmpl_pattern::VariantPayload::Tuple { minus_subs, .. } => {
                    minus_subs[0] = "u32 {{{".to_string();
                }
                _ => panic!("want tuple payload"),
            },
            _ => panic!("want enum joint"),
        }
        let mut edits = Vec::new();
        let err = super::apply_enum_rename_variant_evo_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("invalid payload minus must fail");
        assert!(
            err.message
                .starts_with("invalid joint variant payload type pattern"),
            "got: {}",
            err.message
        );
        let (code, w) = wi18_joint_repair(&err);
        assert_eq!(code, "item-joint-variant-payload-template");
        assert!(w.contains("`- A(u32)`"), "got: {w}");
    }

    #[test]
    fn wi18_slice6_variant_payload_plus_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) =
            wi18_joint_enum_setup("A(u32)", "A(u64)", "enum Old {\n    A(u32),\n}\n");
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::EnumRenameVariantEvo { payload, .. } => match payload {
                rsmpl_pattern::VariantPayload::Tuple { plus_raws, .. } => {
                    plus_raws[0] = "u64 {{{".to_string();
                }
                _ => panic!("want tuple payload"),
            },
            _ => panic!("want enum joint"),
        }
        let mut edits = Vec::new();
        let err = super::apply_enum_rename_variant_evo_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("invalid payload plus must fail");
        assert!(
            err.message
                .starts_with("substituted joint variant payload type does not parse"),
            "got: {}",
            err.message
        );
        let (code, w) = wi18_joint_repair(&err);
        assert_eq!(code, "item-joint-variant-payload-plus-template");
        assert!(w.contains("`+ A(u64)`"), "got: {w}");
    }

    #[test]
    fn wi18_slice6_discriminant_minus_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) =
            wi18_joint_enum_setup("A = 1", "A = 2", "enum Old {\n    A = 1,\n}\n");
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::EnumRenameVariantEvo { payload, .. } => match payload {
                rsmpl_pattern::VariantPayload::Discriminant { minus_sub, .. } => {
                    *minus_sub = "1 +{{".to_string();
                }
                _ => panic!("want discriminant payload"),
            },
            _ => panic!("want enum joint"),
        }
        let mut edits = Vec::new();
        let err = super::apply_enum_rename_variant_evo_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("invalid discriminant minus must fail");
        assert!(
            err.message
                .starts_with("invalid joint discriminant pattern"),
            "got: {}",
            err.message
        );
        let (code, w) = wi18_joint_repair(&err);
        assert_eq!(code, "item-joint-discriminant-template");
        assert!(w.contains("`- A = 1`"), "got: {w}");
    }

    #[test]
    fn wi18_slice6_discriminant_plus_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) =
            wi18_joint_enum_setup("A = 1", "A = 2", "enum Old {\n    A = 1,\n}\n");
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::EnumRenameVariantEvo { payload, .. } => match payload {
                rsmpl_pattern::VariantPayload::Discriminant { plus_raw, .. } => {
                    *plus_raw = "2 +{{".to_string();
                }
                _ => panic!("want discriminant payload"),
            },
            _ => panic!("want enum joint"),
        }
        let mut edits = Vec::new();
        let err = super::apply_enum_rename_variant_evo_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("invalid discriminant plus must fail");
        assert!(
            err.message
                .starts_with("substituted joint discriminant does not parse"),
            "got: {}",
            err.message
        );
        let (code, w) = wi18_joint_repair(&err);
        assert_eq!(code, "item-joint-discriminant-plus-template");
        assert!(w.contains("`+ A = 2`"), "got: {w}");
    }
}
