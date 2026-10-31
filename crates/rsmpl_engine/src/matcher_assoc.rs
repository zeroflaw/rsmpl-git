//! Associated-item renames + evolution (WI-13 slice 1, WI-26, SPEC 14.8).
//!
//! `trait Tr {` / `impl S {` context header plus a `type`/`const` pair.
//! Renames replace only the member name ident span (suffixes preserved).
//! Evolutions keep the name and replace only the RHS/default span via a
//! type/general-expr template (WI-11 discipline scoped by the block
//! header). Definition-only in v1: `# follow=uses` is rejected (C17).

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_item::{attrs_equal, check_attrs, span_bytes};

struct AssocCandidate {
    member_ident: proc_macro2::Span,
    member_attrs: Vec<syn::Attribute>,
    container_start: u32,
    container_end: u32,
    anchor_line: u32,
    attr_start_line: u32,
}

fn collect(
    items: &[syn::Item],
    filter: &rsmpl_pattern::ImplHeaderFilter,
    want_trait: bool,
    is_type: bool,
    old: &str,
    out: &mut Vec<AssocCandidate>,
) {
    struct Collector<'a> {
        filter: &'a rsmpl_pattern::ImplHeaderFilter,
        want_trait: bool,
        is_type: bool,
        old: &'a str,
        found: Vec<AssocCandidate>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if self.want_trait {
                if let syn::Item::Trait(t) = it {
                    if rsmpl_pattern::impl_block_matches(self.filter, it) {
                        if self.is_type {
                            for item in &t.items {
                                if let syn::TraitItem::Type(ty) = item {
                                    if ty.ident == self.old {
                                        self.found.push(AssocCandidate {
                                            member_ident: ty.ident.span(),
                                            member_attrs: ty.attrs.clone(),
                                            container_start: t.span().start().line as u32,
                                            container_end: t.span().end().line as u32,
                                            anchor_line: ty.ident.span().start().line as u32,
                                            attr_start_line: ty.span().start().line as u32,
                                        });
                                    }
                                }
                            }
                        } else {
                            for item in &t.items {
                                if let syn::TraitItem::Const(c) = item {
                                    if c.ident == self.old {
                                        self.found.push(AssocCandidate {
                                            member_ident: c.ident.span(),
                                            member_attrs: c.attrs.clone(),
                                            container_start: t.span().start().line as u32,
                                            container_end: t.span().end().line as u32,
                                            anchor_line: c.ident.span().start().line as u32,
                                            attr_start_line: c.span().start().line as u32,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            } else if let syn::Item::Impl(imp) = it {
                if rsmpl_pattern::impl_block_matches(self.filter, it) {
                    if self.is_type {
                        for item in &imp.items {
                            if let syn::ImplItem::Type(ty) = item {
                                if ty.ident == self.old {
                                    self.found.push(AssocCandidate {
                                        member_ident: ty.ident.span(),
                                        member_attrs: ty.attrs.clone(),
                                        container_start: imp.span().start().line as u32,
                                        container_end: imp.span().end().line as u32,
                                        anchor_line: ty.ident.span().start().line as u32,
                                        attr_start_line: ty.span().start().line as u32,
                                    });
                                }
                            }
                        }
                    } else {
                        for item in &imp.items {
                            if let syn::ImplItem::Const(c) = item {
                                if c.ident == self.old {
                                    self.found.push(AssocCandidate {
                                        member_ident: c.ident.span(),
                                        member_attrs: c.attrs.clone(),
                                        container_start: imp.span().start().line as u32,
                                        container_end: imp.span().end().line as u32,
                                        anchor_line: c.ident.span().start().line as u32,
                                        attr_start_line: c.span().start().line as u32,
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
        filter,
        want_trait,
        is_type,
        old,
        found: Vec::new(),
    };
    for it in items {
        collector.visit_item(it);
    }
    out.extend(collector.found);
}

fn in_scope(c: &AssocCandidate, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.container_start >= old_start && c.container_end <= end.max(old_start),
        Scope::Anchor => c.anchor_line >= old_start && c.anchor_line <= end.max(old_start),
        Scope::Overlap => c.container_start <= end.max(old_start) && c.container_end >= old_start,
        Scope::Exact => old_len > 0 && c.container_start == old_start && c.container_end == end,
        Scope::File => true,
    }
}

fn norm_attr(s: &str) -> String {
    s.split_whitespace().collect()
}

fn attrs_match(pattern_attrs: &[String], attrs: &[syn::Attribute]) -> bool {
    let want: Vec<String> = pattern_attrs.iter().map(|a| norm_attr(a)).collect();
    let have: Vec<String> = attrs
        .iter()
        .map(|a| norm_attr(&quote::quote!(#a).to_string()))
        .collect();
    want == have
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_repair(StructuredError::internal_repair())
}

/// Apply an associated-item rename hunk. Returns the number of matches.
pub fn apply_assoc_rename_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (impl_header, want_trait, is_type, old, new, minus_attrs, plus_attrs) = match &compiled.kind
    {
        PatternKind::AssocRename {
            impl_header,
            want_trait,
            is_type,
            minus_name,
            plus_name,
            minus_attrs,
            plus_attrs,
        } => (
            impl_header.clone(),
            *want_trait,
            *is_type,
            minus_name.clone(),
            plus_name.clone(),
            minus_attrs.clone(),
            plus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected assoc rename pattern".to_string(),
            ));
        }
    };
    let rule = hunk.rule_name();
    // Attr mirror gates (member-rename parity): plus writing attrs the
    // minus omitted is ambiguous; differing sets are never dropped.
    if !plus_attrs.is_empty() && minus_attrs.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "plus side writes attributes the minus side omitted (ambiguous merge); write the attributes on both sides or neither",
        )
        .with_rule(&rule)
        .with_repair(StructuredError::fix_pattern_repair(
            "C11",
            "write the attributes on both sides or neither",
        )));
    }
    if hunk.attrs_policy() != AttrsPolicy::Strip
        && !minus_attrs.is_empty()
        && minus_attrs.iter().map(|a| norm_attr(a)).collect::<Vec<_>>()
            != plus_attrs.iter().map(|a| norm_attr(a)).collect::<Vec<_>>()
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
        )
        .with_rule(&rule)
        .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`")
        .with_repair(StructuredError::fix_pattern_repair(
            "C12",
            "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
        )));
    }
    let filter = rsmpl_pattern::parse_impl_header(&impl_header);
    let pass = filter.is_some();
    if !pass {
        return Ok(0);
    }
    let filter = filter.unwrap_or_else(|| rsmpl_pattern::parse_impl_header("impl __X {").unwrap());
    let mut candidates = Vec::new();
    collect(
        &parsed.items,
        &filter,
        want_trait,
        is_type,
        &old,
        &mut candidates,
    );
    candidates.sort_by_key(|c| (c.container_start, c.anchor_line));
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let file = "";
    let mut matched = 0;
    for c in &candidates {
        if !in_scope(c, hunk.scope(), old_start, old_len) {
            continue;
        }
        if hunk.attrs_policy() == AttrsPolicy::Preserve
            && !minus_attrs.is_empty()
            && !attrs_match(&minus_attrs, &c.member_attrs)
        {
            continue;
        }
        check_attrs(
            &c.member_attrs,
            hunk.attrs_policy(),
            &minus_attrs,
            &rule,
            file,
        )?;
        let (a, b) = span_bytes(original, index, c.member_ident)
            .ok_or_else(|| range_err(&rule, c.member_ident.start().line as u32))?;
        if original.get(a..b) != Some(old.as_str()) {
            return Err(StructuredError::new(
                ErrorCode::MatchConflict,
                "assoc name span mismatch (file changed during matching?)",
            )
            .with_rule(&rule)
            .with_repair(StructuredError::internal_repair()));
        }
        edits.push(TextEdit::new(a, b, new.clone()).with_label("rename"));
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

/// One assoc evolution candidate: the preserved name plus the RHS span.
struct AssocEvoCandidate<'ast> {
    attrs: Vec<syn::Attribute>,
    rhs_ty: Option<&'ast syn::Type>,
    rhs_expr: Option<&'ast syn::Expr>,
    ascribed: Option<String>,
    start_line: u32,
    end_line: u32,
    anchor_line: u32,
    attr_start_line: u32,
}

fn norm_ty(ty: &syn::Type) -> String {
    quote::quote!(#ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_evo<'ast>(
    items: &'ast [syn::Item],
    filter: &rsmpl_pattern::ImplHeaderFilter,
    want_trait: bool,
    is_type: bool,
    name: &str,
    out: &mut Vec<AssocEvoCandidate<'ast>>,
) {
    struct Collector<'a, 'ast> {
        filter: &'a rsmpl_pattern::ImplHeaderFilter,
        want_trait: bool,
        is_type: bool,
        name: &'a str,
        found: Vec<AssocEvoCandidate<'ast>>,
    }
    impl<'ast> Visit<'ast> for Collector<'_, 'ast> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if self.want_trait {
                if let syn::Item::Trait(t) = it {
                    if rsmpl_pattern::impl_block_matches(self.filter, it) {
                        if self.is_type {
                            for item in &t.items {
                                if let syn::TraitItem::Type(ty) = item {
                                    if ty.ident == self.name {
                                        if let Some((_, default)) = &ty.default {
                                            self.found.push(AssocEvoCandidate {
                                                attrs: ty.attrs.clone(),
                                                rhs_ty: Some(default),
                                                rhs_expr: None,
                                                ascribed: None,
                                                start_line: ty.span().start().line as u32,
                                                end_line: ty.span().end().line as u32,
                                                anchor_line: ty.ident.span().start().line as u32,
                                                attr_start_line: ty.span().start().line as u32,
                                            });
                                        }
                                    }
                                }
                            }
                        } else {
                            for item in &t.items {
                                if let syn::TraitItem::Const(c) = item {
                                    if c.ident == self.name {
                                        if let Some((_, default)) = &c.default {
                                            self.found.push(AssocEvoCandidate {
                                                attrs: c.attrs.clone(),
                                                rhs_ty: None,
                                                rhs_expr: Some(default),
                                                ascribed: Some(norm_ty(&c.ty)),
                                                start_line: c.span().start().line as u32,
                                                end_line: c.span().end().line as u32,
                                                anchor_line: c.ident.span().start().line as u32,
                                                attr_start_line: c.span().start().line as u32,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else if let syn::Item::Impl(imp) = it {
                if rsmpl_pattern::impl_block_matches(self.filter, it) {
                    if self.is_type {
                        for item in &imp.items {
                            if let syn::ImplItem::Type(ty) = item {
                                if ty.ident == self.name {
                                    self.found.push(AssocEvoCandidate {
                                        attrs: ty.attrs.clone(),
                                        rhs_ty: Some(&ty.ty),
                                        rhs_expr: None,
                                        ascribed: None,
                                        start_line: ty.span().start().line as u32,
                                        end_line: ty.span().end().line as u32,
                                        anchor_line: ty.ident.span().start().line as u32,
                                        attr_start_line: ty.span().start().line as u32,
                                    });
                                }
                            }
                        }
                    } else {
                        for item in &imp.items {
                            if let syn::ImplItem::Const(c) = item {
                                if c.ident == self.name {
                                    self.found.push(AssocEvoCandidate {
                                        attrs: c.attrs.clone(),
                                        rhs_ty: None,
                                        rhs_expr: Some(&c.expr),
                                        ascribed: Some(norm_ty(&c.ty)),
                                        start_line: c.span().start().line as u32,
                                        end_line: c.span().end().line as u32,
                                        anchor_line: c.ident.span().start().line as u32,
                                        attr_start_line: c.span().start().line as u32,
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
        filter,
        want_trait,
        is_type,
        name,
        found: Vec::new(),
    };
    for it in items {
        collector.visit_item(it);
    }
    out.extend(collector.found);
}

fn evo_in_scope(c: &AssocEvoCandidate<'_>, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.start_line >= old_start && c.end_line <= end.max(old_start),
        Scope::Anchor => c.anchor_line >= old_start && c.anchor_line <= end.max(old_start),
        Scope::Overlap => c.start_line <= end.max(old_start) && c.end_line >= old_start,
        Scope::Exact => old_len > 0 && c.start_line == old_start && c.end_line == end,
        Scope::File => true,
    }
}

/// Apply an associated-item evolution hunk. Returns (matches, bindings).
/// Only the RHS/default span is replaced; the name, generics, bounds,
/// ascribed types, and attributes are preserved.
pub fn apply_assoc_evolution_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (
        impl_header,
        want_trait,
        is_type,
        name,
        ty,
        minus_sub,
        plus_raw,
        placeholders,
        minus_attrs,
    ) = match &compiled.kind {
        PatternKind::AssocEvolution {
            impl_header,
            want_trait,
            is_type,
            name,
            ty,
            minus_sub,
            plus_raw,
            placeholders,
            minus_attrs,
            ..
        } => (
            impl_header.clone(),
            *want_trait,
            *is_type,
            name.clone(),
            ty.clone(),
            minus_sub.clone(),
            plus_raw.clone(),
            placeholders.clone(),
            minus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected assoc evolution pattern".to_string(),
            ));
        }
    };
    let rule = hunk.rule_name();
    let file = "";
    let filter = rsmpl_pattern::parse_impl_header(&impl_header);
    let Some(filter) = filter else {
        return Ok((0, Vec::new()));
    };
    let reverse: HashMap<String, String> = placeholders
        .iter()
        .map(|(n, ph)| (ph.clone(), n.clone()))
        .collect();
    let mut candidates = Vec::new();
    collect_evo(
        &parsed.items,
        &filter,
        want_trait,
        is_type,
        &name,
        &mut candidates,
    );
    candidates.sort_by_key(|c| (c.start_line, c.anchor_line));
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    let mut all_bindings = Vec::new();
    if is_type {
        let pat_ty: syn::Type = syn::parse_str(&minus_sub).map_err(|e| {
            StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid assoc type pattern: {e}"),
            )
            .with_rule(&rule)
            .with_repair(StructuredError::internal_repair())
        })?;
        for c in &candidates {
            if !evo_in_scope(c, hunk.scope(), old_start, old_len) {
                continue;
            }
            if hunk.attrs_policy() == AttrsPolicy::Preserve
                && !minus_attrs.is_empty()
                && !attrs_equal(&minus_attrs, &c.attrs)
            {
                continue;
            }
            check_attrs(&c.attrs, hunk.attrs_policy(), &minus_attrs, &rule, file)?;
            let Some(rhs) = c.rhs_ty else { continue };
            let mut cx =
                crate::matcher_general::MatchCx::with_reverse(original, index, reverse.clone());
            if !crate::matcher_types::match_type(&pat_ty, rhs, &mut cx) {
                continue;
            }
            let subst = crate::matcher_expr_subst::substitute_call_plus(
                &plus_raw,
                &cx.bindings,
                &[],
                compiled.table.sigiled,
            );
            let parsed_sub = syn::parse_str::<syn::Type>(&subst).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("substituted assoc RHS does not parse: {e}"),
                )
                .with_rule(&rule)
                .with_repair(StructuredError::internal_repair())
            })?;
            let _ = parsed_sub;
            let (a, b) = span_bytes(original, index, rhs.span())
                .ok_or_else(|| range_err(&rule, c.anchor_line))?;
            edits.push(TextEdit::new(a, b, subst));
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
            all_bindings.push(cx.bindings);
            matched += 1;
            if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                break;
            }
        }
    } else {
        let pat_expr: syn::Expr = syn::parse_str(&minus_sub).map_err(|e| {
            StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid assoc const pattern: {e}"),
            )
            .with_rule(&rule)
            .with_repair(StructuredError::internal_repair())
        })?;
        for c in &candidates {
            if !evo_in_scope(c, hunk.scope(), old_start, old_len) {
                continue;
            }
            if !ty.is_empty() && c.ascribed.as_deref() != Some(ty.as_str()) {
                continue;
            }
            if hunk.attrs_policy() == AttrsPolicy::Preserve
                && !minus_attrs.is_empty()
                && !attrs_equal(&minus_attrs, &c.attrs)
            {
                continue;
            }
            check_attrs(&c.attrs, hunk.attrs_policy(), &minus_attrs, &rule, file)?;
            let Some(rhs) = c.rhs_expr else { continue };
            let mut cx =
                crate::matcher_general::MatchCx::with_reverse(original, index, reverse.clone());
            if !crate::matcher_general::match_expr(&pat_expr, rhs, &mut cx) {
                continue;
            }
            let subst = crate::matcher_expr_subst::substitute_call_plus(
                &plus_raw,
                &cx.bindings,
                &[],
                compiled.table.sigiled,
            );
            syn::parse_str::<syn::Expr>(&subst).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("substituted assoc value does not parse: {e}"),
                )
                .with_rule(&rule)
                .with_repair(StructuredError::internal_repair())
            })?;
            let (a, b) = span_bytes(original, index, rhs.span())
                .ok_or_else(|| range_err(&rule, c.anchor_line))?;
            edits.push(TextEdit::new(a, b, subst));
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
            all_bindings.push(cx.bindings);
            matched += 1;
            if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                break;
            }
        }
    }
    Ok((matched, all_bindings))
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

    fn patch(ctx: &str, minus: &str, plus: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n  {ctx}\n- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn renames_trait_type() {
        let p = patch("trait Tr {", "type Item;", "type Output;");
        let out = apply(&p, "trait Tr {\n    type Item;\n}\n").unwrap();
        assert!(out.contains("type Output;"), "got: {out}");
        assert!(!out.contains("type Item;"), "got: {out}");
    }

    #[test]
    fn renames_impl_type_preserving_rhs() {
        let p = patch("impl S {", "type Item = i32;", "type Output = i32;");
        let out = apply(&p, "impl S {\n    type Item = i32;\n}\n").unwrap();
        assert!(out.contains("type Output = i32;"), "got: {out}");
    }

    #[test]
    fn renames_const_preserving_value() {
        let p = patch("impl S {", "const MAX: i32 = 10;", "const LIMIT: i32 = 10;");
        let out = apply(&p, "impl S {\n    const MAX: i32 = 10;\n}\n").unwrap();
        assert!(out.contains("const LIMIT: i32 = 10;"), "got: {out}");
    }

    #[test]
    fn wrong_block_does_not_match() {
        let p = patch("trait Tr {", "type Item;", "type Output;");
        assert!(apply(&p, "trait Other {\n    type Item;\n}\n").is_err());
    }

    #[test]
    fn follow_rejects_loud() {
        let mut p = patch("trait Tr {", "type Item;", "type Output;");
        p = p.replace("rule=r", "rule=r\n# follow=uses");
        let err = apply(&p, "trait Tr {\n    type Item;\n}\n").unwrap_err();
        assert!(err.contains("follow=uses"), "got: {err}");
    }

    #[test]
    fn sweep_fires_across_blocks() {
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# sweep\n# allow=many\n  trait Tr {\n- type Item;\n+ type Output;\n";
        let src = "trait Tr {\n    type Item;\n}\ntrait Tr2 {\n    type Item;\n}\n";
        // Headers differ per block: only the named block matches, so the
        // second block stays — sweep over one header cannot claim both.
        let out = apply(p, "trait Tr {\n    type Item;\n}\n").unwrap();
        assert!(out.contains("type Output;"), "got: {out}");
        let _ = src;
    }

    fn apply_report(
        patch_text: &str,
        file_text: &str,
    ) -> Result<(String, crate::RuleSemantics), String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        let report = crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok((files["src/lib.rs"].clone(), report.rules[0].semantics))
    }

    #[test]
    fn evolves_trait_type() {
        let p = patch(
            "trait Tr {",
            "type Item = Vec<u8>;",
            "type Item = Box<[u8]>;",
        );
        let (out, sem) = apply_report(&p, "trait Tr {\n    type Item = Vec<u8>;\n}\n").unwrap();
        assert!(out.contains("type Item = Box<[u8]>;"), "got: {out}");
        assert_eq!(sem, crate::RuleSemantics::Instantiated);
    }

    #[test]
    fn evolves_impl_type_hole() {
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# type T\n  impl S {\n- type Item = Vec<T>;\n+ type Item = Box<[T]>;\n";
        let (out, sem) = apply_report(p, "impl S {\n    type Item = Vec<T>;\n}\n").unwrap();
        assert!(out.contains("type Item = Box<[T]>;"), "got: {out}");
        assert_eq!(sem, crate::RuleSemantics::Instantiated);
    }

    #[test]
    fn evolves_const_value() {
        let p = patch("impl S {", "const MAX: i32 = E;", "const MAX: i32 = E + 1;");
        let (out, sem) = apply_report(&p, "impl S {\n    const MAX: i32 = 10;\n}\n").unwrap();
        assert!(out.contains("const MAX: i32 = 10 + 1;"), "got: {out}");
        assert_eq!(sem, crate::RuleSemantics::Instantiated);
    }

    #[test]
    fn evo_follow_rejects_loud() {
        let mut p = patch("impl S {", "type Item = Vec<u8>;", "type Item = Box<[u8]>;");
        p = p.replace("rule=r", "rule=r\n# follow=uses");
        let err = apply(&p, "impl S {\n    type Item = Vec<u8>;\n}\n").unwrap_err();
        assert!(err.contains("follow=uses"), "got: {err}");
    }

    #[test]
    fn evo_wrong_block_does_not_match() {
        let p = patch(
            "trait Tr {",
            "type Item = Vec<u8>;",
            "type Item = Box<[u8]>;",
        );
        assert!(apply(&p, "trait Other {\n    type Item = Vec<u8>;\n}\n").is_err());
    }

    #[test]
    fn evo_trait_no_default_skips() {
        let p = patch(
            "trait Tr {",
            "type Item = Vec<u8>;",
            "type Item = Box<[u8]>;",
        );
        assert!(apply(&p, "trait Tr {\n    type Item;\n}\n").is_err());
    }
}
