//! Definition RHS evolution (SPEC 14.8, WI-11a): type-alias and
//! const/static value rewrites.
//!
//! Authored shapes reuse the bare-pair discipline: the name is concrete
//! and preserved, only the RHS evolves.
//!
//! ```diff
//! - type A = Vec<T>;
//! + type A = Box<[T]>;
//! ```
//!
//! ```diff
//! - const MAX: i32 = E;
//! + const MAX: i32 = E + 1;
//! ```
//!
//! Only the RHS span (alias type, const/static value) is replaced;
//! visibility, generics, `where` clauses, and attributes are preserved
//! byte-for-byte — the item-rename posture scoped to the value side.
//! An ascribed type that differs from the authored annotation skips the
//! site (subset semantics: a wrong annotation never rewrites under a
//! lie); `static mut` must agree on both sides.

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{Allow, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_item::{attrs_equal, check_attrs, span_bytes};

/// One definition candidate: the name ident plus the evolved RHS span.
struct ValueCandidate<'ast> {
    attrs: Vec<syn::Attribute>,
    /// Alias RHS type or const/static value expression.
    rhs_ty: Option<&'ast syn::Type>,
    rhs_expr: Option<&'ast syn::Expr>,
    /// Normalized ascribed type (const/static only).
    ascribed: Option<String>,
    /// `static mut` (static only).
    is_mut: bool,
    start_line: u32,
    end_line: u32,
    anchor_line: u32,
    attr_start_line: u32,
}

/// Whitespace-normalized token rendering (same normalization the
/// classifier applies to the authored ascribed type).
fn norm_ty(ty: &syn::Type) -> String {
    quote::quote!(#ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Collect alias/const/static candidates named `name` from file items,
/// inline modules, and block-level items (the `matcher_item` walk —
/// macro bodies stay excluded).
fn collect<'ast>(
    items: &'ast [syn::Item],
    name: &str,
    want_alias: bool,
    want_const: bool,
    want_static: bool,
    out: &mut Vec<ValueCandidate<'ast>>,
) {
    struct Collector<'a, 'ast> {
        name: &'a str,
        want_alias: bool,
        want_const: bool,
        want_static: bool,
        found: Vec<ValueCandidate<'ast>>,
    }
    impl<'ast> Visit<'ast> for Collector<'_, 'ast> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            match it {
                syn::Item::Type(t) if self.want_alias && t.ident == self.name => {
                    self.found.push(ValueCandidate {
                        attrs: t.attrs.clone(),
                        rhs_ty: Some(&t.ty),
                        rhs_expr: None,
                        ascribed: None,
                        is_mut: false,
                        start_line: t.span().start().line as u32,
                        end_line: t.span().end().line as u32,
                        anchor_line: t.ident.span().start().line as u32,
                        attr_start_line: t.span().start().line as u32,
                    });
                }
                syn::Item::Const(c) if self.want_const && c.ident == self.name => {
                    self.found.push(ValueCandidate {
                        attrs: c.attrs.clone(),
                        rhs_ty: None,
                        rhs_expr: Some(&c.expr),
                        ascribed: Some(norm_ty(&c.ty)),
                        is_mut: false,
                        start_line: c.span().start().line as u32,
                        end_line: c.span().end().line as u32,
                        anchor_line: c.ident.span().start().line as u32,
                        attr_start_line: c.span().start().line as u32,
                    });
                }
                syn::Item::Static(s) if self.want_static && s.ident == self.name => {
                    self.found.push(ValueCandidate {
                        attrs: s.attrs.clone(),
                        rhs_ty: None,
                        rhs_expr: Some(&s.expr),
                        ascribed: Some(norm_ty(&s.ty)),
                        is_mut: matches!(s.mutability, syn::StaticMutability::Mut(_)),
                        start_line: s.span().start().line as u32,
                        end_line: s.span().end().line as u32,
                        anchor_line: s.ident.span().start().line as u32,
                        attr_start_line: s.span().start().line as u32,
                    });
                }
                _ => {}
            }
            visit::visit_item(self, it);
        }
    }
    let mut collector = Collector {
        name,
        want_alias,
        want_const,
        want_static,
        found: Vec::new(),
    };
    for it in items {
        collector.visit_item(it);
    }
    out.extend(collector.found);
}

/// Scope filtering (SPEC 17.3): the item envelope for range modes, the
/// name ident line as the anchor — the `matcher_item` posture.
fn in_scope(c: &ValueCandidate<'_>, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.start_line >= old_start && c.end_line <= end.max(old_start),
        Scope::Anchor => c.anchor_line >= old_start && c.anchor_line <= end.max(old_start),
        Scope::Overlap => c.start_line <= end.max(old_start) && c.end_line >= old_start,
        Scope::Exact => old_len > 0 && c.start_line == old_start && c.end_line == end,
        Scope::File => true,
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

fn pattern_err(rule: &str, msg: String) -> StructuredError {
    StructuredError::new(ErrorCode::PatternConflict, msg)
        .with_rule(rule)
        .with_repair(StructuredError::internal_repair())
}

/// Unpack the shared shape fields of an evolution hunk.
struct EvoShape {
    name: String,
    minus_sub: String,
    plus_raw: String,
    placeholders: Vec<(String, String)>,
    minus_attrs: Vec<String>,
    sigiled: bool,
}

fn evo_shape(compiled: &CompiledPattern, alias: bool) -> Result<EvoShape, StructuredError> {
    let bad = || {
        StructuredError::new(
            ErrorCode::PatternConflict,
            "expected definition evolution pattern".to_string(),
        )
    };
    match &compiled.kind {
        PatternKind::TypeAliasEvolution {
            name,
            minus_sub,
            plus_raw,
            placeholders,
            minus_attrs,
            ..
        } if alias => Ok(EvoShape {
            name: name.clone(),
            minus_sub: minus_sub.clone(),
            plus_raw: plus_raw.clone(),
            placeholders: placeholders.clone(),
            minus_attrs: minus_attrs.clone(),
            sigiled: compiled.table.sigiled,
        }),
        PatternKind::ConstEvolution {
            name,
            minus_sub,
            plus_raw,
            placeholders,
            minus_attrs,
            ..
        } if !alias => Ok(EvoShape {
            name: name.clone(),
            minus_sub: minus_sub.clone(),
            plus_raw: plus_raw.clone(),
            placeholders: placeholders.clone(),
            minus_attrs: minus_attrs.clone(),
            sigiled: compiled.table.sigiled,
        }),
        _ => Err(bad()),
    }
}

/// Shared per-site loop: scope + attr gates, then `check` (structural
/// match with bindings) and `emit` (RHS replacement text). Returns
/// (matches, per-site bindings in span order).
#[allow(clippy::too_many_arguments)]
fn apply_evolution(
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    shape: &EvoShape,
    mut candidates: Vec<ValueCandidate<'_>>,
    check: impl Fn(&ValueCandidate<'_>, &mut crate::matcher_general::MatchCx<'_>) -> bool,
    emit: impl Fn(&ValueCandidate<'_>, &HashMap<String, String>) -> Result<String, StructuredError>,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let rule = hunk.rule_name();
    let file = "";
    candidates.sort_by_key(|c| (c.start_line, c.anchor_line));
    let reverse: HashMap<String, String> = shape
        .placeholders
        .iter()
        .map(|(n, ph)| (ph.clone(), n.clone()))
        .collect();
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    let mut all_bindings = Vec::new();
    for c in &candidates {
        if !in_scope(c, hunk.scope(), old_start, old_len) {
            continue;
        }
        // Preserve plus written minus-side attributes constrains the match
        // (T235, SPEC 14.6): sites carrying different attributes are
        // skipped, never silently rewritten.
        if hunk.attrs_policy() == rsmpl_patch::AttrsPolicy::Preserve
            && !shape.minus_attrs.is_empty()
            && !attrs_equal(&shape.minus_attrs, &c.attrs)
        {
            continue;
        }
        check_attrs(
            &c.attrs,
            hunk.attrs_policy(),
            &shape.minus_attrs,
            &rule,
            file,
        )?;
        let mut cx =
            crate::matcher_general::MatchCx::with_reverse(original, index, reverse.clone());
        if !check(c, &mut cx) {
            continue;
        }
        let replacement = emit(c, &cx.bindings)?;
        let (a, b) = span_bytes(original, index, rhs_span(c))
            .ok_or_else(|| range_err(&rule, c.anchor_line))?;
        edits.push(TextEdit::new(a, b, replacement));
        // `# attrs=strip`: delete attribute lines above the item.
        if hunk.attrs_policy() == rsmpl_patch::AttrsPolicy::Strip && !c.attrs.is_empty() {
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
        if hunk.allow() == Allow::One && matched > 1 {
            break; // count validation reports the error
        }
    }
    Ok((matched, all_bindings))
}

fn rhs_span(c: &ValueCandidate<'_>) -> proc_macro2::Span {
    use syn::spanned::Spanned;
    if let Some(ty) = c.rhs_ty {
        ty.span()
    } else {
        c.rhs_expr.expect("definition candidate").span()
    }
}

/// Apply a type-alias RHS evolution hunk. Only the alias RHS type span
/// is replaced; visibility, generics, `where` clauses, and attributes
/// are preserved.
pub fn apply_type_alias_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let shape = evo_shape(compiled, true)?;
    let rule = hunk.rule_name();
    let pat_ty: syn::Type = syn::parse_str(&shape.minus_sub)
        .map_err(|e| pattern_err(&rule, format!("invalid type alias pattern: {e}")))?;
    let mut candidates = Vec::new();
    collect(
        &parsed.items,
        &shape.name,
        true,
        false,
        false,
        &mut candidates,
    );
    let check = |c: &ValueCandidate<'_>, cx: &mut crate::matcher_general::MatchCx<'_>| {
        c.rhs_ty
            .is_some_and(|t| crate::matcher_types::match_type(&pat_ty, t, cx))
    };
    let emit = |_: &ValueCandidate<'_>, bindings: &HashMap<String, String>| {
        let subst = crate::matcher_expr_subst::substitute_call_plus(
            &shape.plus_raw,
            bindings,
            &[],
            shape.sigiled,
        );
        syn::parse_str::<syn::Type>(&subst)
            .map(|_| subst.clone())
            .map_err(|e| pattern_err(&rule, format!("substituted alias RHS does not parse: {e}")))
    };
    apply_evolution(
        original, index, hunk, &shape, candidates, check, emit, edits,
    )
}

/// Apply a const/static value evolution hunk. Only the value expression
/// span is replaced; the ascribed type must render equal to the
/// authored annotation (a different type skips the site, never rewrites
/// under a lie), and `static mut` must agree with the pattern.
pub fn apply_const_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (is_static, is_mut, ty, shape) = match &compiled.kind {
        PatternKind::ConstEvolution {
            is_static,
            is_mut,
            ty,
            ..
        } => (*is_static, *is_mut, ty.clone(), evo_shape(compiled, false)?),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected const/static evolution pattern".to_string(),
            ));
        }
    };
    let rule = hunk.rule_name();
    let pat_expr: syn::Expr = syn::parse_str(&shape.minus_sub)
        .map_err(|e| pattern_err(&rule, format!("invalid const/static value pattern: {e}")))?;
    let mut candidates = Vec::new();
    collect(
        &parsed.items,
        &shape.name,
        false,
        !is_static,
        is_static,
        &mut candidates,
    );
    let check = |c: &ValueCandidate<'_>, cx: &mut crate::matcher_general::MatchCx<'_>| {
        if c.is_mut != is_mut {
            return false;
        }
        if c.ascribed.as_deref() != Some(ty.as_str()) {
            return false;
        }
        c.rhs_expr
            .is_some_and(|e| crate::matcher_general::match_expr(&pat_expr, e, cx))
    };
    let emit = |_: &ValueCandidate<'_>, bindings: &HashMap<String, String>| {
        let subst = crate::matcher_expr_subst::substitute_call_plus(
            &shape.plus_raw,
            bindings,
            &[],
            shape.sigiled,
        );
        syn::parse_str::<syn::Expr>(&subst)
            .map(|_| subst.clone())
            .map_err(|e| pattern_err(&rule, format!("substituted value does not parse: {e}")))
    };
    apply_evolution(
        original, index, hunk, &shape, candidates, check, emit, edits,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::RuleSemantics;

    fn apply_report(
        patch_text: &str,
        file_text: &str,
    ) -> Result<(String, crate::ApplyReport), String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        let report = crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok((files["src/lib.rs"].clone(), report))
    }

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        apply_report(patch_text, file_text).map(|(s, _)| s)
    }

    fn evo_patch(minus: &str, plus: &str, range: &str, directives: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=evo\n# lang=rust\n{directives}- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn type_alias_concrete_fire_instantiated() {
        let patch = evo_patch(
            "type A = Vec<u8>;",
            "type A = Box<[u8]>;",
            "@@ -1,1 +1,1 @@",
            "",
        );
        let (out, report) = apply_report(&patch, "type A = Vec<u8>;\n").unwrap();
        assert_eq!(out, "type A = Box<[u8]>;\n", "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Instantiated);
    }

    #[test]
    fn type_alias_hole_preserves_vis_generics() {
        // WI-11a: declared `# type T` binds the RHS hole; vis/generics
        // ride along preserved.
        let patch = evo_patch(
            "type G = Vec<T>;",
            "type G = Box<[T]>;",
            "@@ -1,1 +1,1 @@",
            "# type T\n",
        );
        let src = "pub type G<T> = Vec<T>;\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert_eq!(out, "pub type G<T> = Box<[T]>;\n", "got: {out:?}");
        let sites = &report.rules[0].sites;
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].bindings, vec![("T".to_string(), "T".to_string())]);
    }

    #[test]
    fn type_alias_nested_module_found() {
        let patch = evo_patch(
            "type A = Vec<u8>;",
            "type A = Box<[u8]>;",
            "@@ -2,1 +2,1 @@",
            "",
        );
        let src = "mod m {\n    type A = Vec<u8>;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("type A = Box<[u8]>;"), "got: {out:?}");
    }

    #[test]
    fn type_alias_sweep_many() {
        let patch = evo_patch(
            "type A = Vec<u8>;",
            "type A = Box<[u8]>;",
            "@@ -1,1 +1,1 @@",
            "# sweep\n# allow=many\n",
        );
        let src = "type A = Vec<u8>;\nmod m {\n    type A = Vec<u8>;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(out.matches("Box<[u8]>").count(), 2, "got: {out:?}");
    }

    #[test]
    fn type_alias_name_change_fails_fast() {
        // Same RHS, different names: a rename, not an evolution.
        let patch = evo_patch(
            "type A = Vec<u8>;",
            "type B = Vec<u8>;",
            "@@ -1,1 +1,1 @@",
            "",
        );
        let err = apply(&patch, "type A = Vec<u8>;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("bare"), "got: {err}");
        // Both differ: never combined.
        let patch = evo_patch(
            "type A = Vec<u8>;",
            "type B = Box<[u8]>;",
            "@@ -1,1 +1,1 @@",
            "",
        );
        let err = apply(&patch, "type A = Vec<u8>;\n").unwrap_err();
        assert!(err.contains("never combine"), "got: {err}");
    }

    #[test]
    fn type_alias_vis_mismatch_rejected() {
        let patch = evo_patch(
            "pub type A = Vec<u8>;",
            "type A = Box<[u8]>;",
            "@@ -1,1 +1,1 @@",
            "",
        );
        let err = apply(&patch, "pub type A = Vec<u8>;\n").unwrap_err();
        assert!(err.contains("visibility"), "got: {err}");
    }

    #[test]
    fn const_value_diet_hole_fire() {
        // WI-11a: diet `E` infers from the value region (no declaration).
        let patch = evo_patch(
            "const MAX: i32 = E;",
            "const MAX: i32 = E + 1;",
            "@@ -1,1 +1,1 @@",
            "",
        );
        let (out, report) = apply_report(&patch, "const MAX: i32 = 10;\n").unwrap();
        assert_eq!(out, "const MAX: i32 = 10 + 1;\n", "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Instantiated);
        let sites = &report.rules[0].sites;
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].bindings, vec![("E".to_string(), "10".to_string())]);
    }

    #[test]
    fn const_ascribed_type_change_rejected_at_compile() {
        let patch = evo_patch(
            "const MAX: u32 = E;",
            "const MAX: i64 = E;",
            "@@ -1,1 +1,1 @@",
            "# expr E\n",
        );
        let err = apply(&patch, "const MAX: u32 = 1;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("same type"), "got: {err}");
    }

    #[test]
    fn const_ascribed_mismatch_skips_site() {
        // The name matches but the target's type differs from the authored
        // annotation: subset semantics skips instead of rewriting.
        let patch = evo_patch(
            "const MAX: u32 = E;",
            "const MAX: u32 = E + 1;",
            "@@ -1,1 +1,1 @@",
            "# expr E\n",
        );
        let err = apply(&patch, "const MAX: i32 = 10;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn static_mut_twins_fire() {
        let patch = evo_patch(
            "static mut C: u8 = E;",
            "static mut C: u8 = E + 1;",
            "@@ -1,1 +1,1 @@",
            "# expr E\n",
        );
        let out = apply(&patch, "static mut C: u8 = 1;\n").unwrap();
        assert_eq!(out, "static mut C: u8 = 1 + 1;\n", "got: {out:?}");
        let plain = evo_patch(
            "static C: u8 = E;",
            "static C: u8 = E + 1;",
            "@@ -1,1 +1,1 @@",
            "# expr E\n",
        );
        let out = apply(&plain, "static C: u8 = 1;\n").unwrap();
        assert_eq!(out, "static C: u8 = 1 + 1;\n", "got: {out:?}");
    }

    #[test]
    fn static_mut_mismatch_rejected() {
        // `mut` on one side only: different shapes, never an evolution.
        let patch = evo_patch(
            "static mut C: u8 = E;",
            "static C: u8 = E + 1;",
            "@@ -1,1 +1,1 @@",
            "# expr E\n",
        );
        let err = apply(&patch, "static mut C: u8 = 1;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        // `mut` mismatch at the site skips (never rewrites).
        let patch = evo_patch(
            "static C: u8 = E;",
            "static C: u8 = E + 1;",
            "@@ -1,1 +1,1 @@",
            "# expr E\n",
        );
        let err = apply(&patch, "static mut C: u8 = 1;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn const_kind_mismatch_rejected() {
        // `const` vs `static` never combine.
        let patch = evo_patch(
            "const C: u8 = E;",
            "static C: u8 = E + 1;",
            "@@ -1,1 +1,1 @@",
            "# expr E\n",
        );
        let err = apply(&patch, "const C: u8 = 1;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("same shape"), "got: {err}");
    }

    #[test]
    fn evolution_rejects_follow_uses() {
        // C17: evolving a type renames nothing — no follow.
        let patch = evo_patch(
            "type A = Vec<u8>;",
            "type A = Box<[u8]>;",
            "@@ -1,1 +1,1 @@",
            "# follow=uses\n",
        );
        let err = apply(&patch, "type A = Vec<u8>;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("C17"), "got: {err}");
    }

    #[test]
    fn evolution_attr_mirror_laws() {
        // C11: plus writes attrs the minus omitted.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=evo\n# lang=rust\n- type A = Vec<u8>;\n+ #[derive(Clone)]\n+ type A = Box<[u8]>;\n";
        let err = apply(patch, "type A = Vec<u8>;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        // Mirrored attrs constrain the match.
        let mirror = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=evo\n# lang=rust\n- /// Doc.\n- type A = Vec<u8>;\n+ /// Doc.\n+ type A = Box<[u8]>;\n";
        let out = apply(mirror, "/// Doc.\ntype A = Vec<u8>;\n").unwrap();
        assert!(out.contains("type A = Box<[u8]>;"), "got: {out:?}");
        assert!(out.contains("/// Doc."), "got: {out:?}");
        // Wrong-attr sites skip (range cites the single type line).
        let narrow = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=evo\n# lang=rust\n- /// Doc.\n- type A = Vec<u8>;\n+ /// Doc.\n+ type A = Box<[u8]>;\n";
        let err = apply(narrow, "type A = Vec<u8>;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn bare_type_rename_still_routes_to_item() {
        // No `=`: the bare pair stays an item rename (then no-matches
        // here, since the file holds `type A` — proving the classifier
        // did not swallow the shape).
        let patch = evo_patch("type Old", "type New", "@@ -1,1 +1,1 @@", "");
        let err = apply(&patch, "type A = Vec<u8>;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn single_word_const_still_routes_to_rename() {
        // No `:` — not a definition evolution: `- const E` / `+ const F;`
        // is the bare rename pair, preserving the value.
        let patch = evo_patch("const E", "const F;", "@@ -1,1 +1,1 @@", "");
        let out = apply(&patch, "const E: u8 = 1;\n").unwrap();
        assert_eq!(out, "const F: u8 = 1;\n", "got: {out:?}");
    }

    #[test]
    fn authored_generics_rejected_with_bare_form() {
        // Generics are preserved from the target, never authored.
        let patch = evo_patch(
            "type A<T> = Vec<T>;",
            "type A<T> = Box<[T]>;",
            "@@ -1,1 +1,1 @@",
            "# type T\n",
        );
        let err = apply(&patch, "type A<T> = Vec<T>;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }
}
