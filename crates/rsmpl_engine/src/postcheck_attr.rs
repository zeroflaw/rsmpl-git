//! WI-10b attr-text postcondition lift (SPEC 24.6).
//!
//! Rename kinds verify the new ident span-locally ([`super::postcheck`]
//! via `syn` ident equality); that check is name-only, so an attr-only
//! evolution (`#[derive(Debug)]` → `#[derive(Debug, Clone)]` with the
//! name preserved) would verify without ever looking at the attributes.
//! This sibling closes the gap: for delta-carrying hunks (minus and plus
//! attr sets differ) it additionally requires the plus attr text at
//! ident-overlapping definitions — present for add/replace, absent for
//! remove/replace — counted with the same span-overlap + `matches`
//! formulation as the ident check. No-delta hunks return `None` (no
//! opinion; the ident verdict stands alone), and unparseable post-images
//! are the caller's `reparse-only` as before.
//!
//! New module under the D5 file-size gate (`postcheck.rs` only swaps its
//! call sites to the combined counter, shrinking overall).

use crate::matcher_attr_delta::{
    attr_delta, norm_pat, norm_pat_list, site_attr_texts,
    subsequence_positions as subsequence_indices, AttrDelta,
};
use crate::postcheck::DefKind;

/// One definition ident with its normalized attached attributes.
struct AttrDef {
    kind: DefKind,
    name: String,
    start_line: usize,
    end_line: usize,
    attrs: Vec<String>,
}

fn span_lines<S: syn::spanned::Spanned>(node: &S) -> (usize, usize) {
    let s = node.span();
    (s.start().line.max(1), s.end().line.max(s.start().line))
}

fn ranges_overlap(def: (usize, usize), span: (u32, u32)) -> bool {
    (def.0 as u32) <= span.1 && span.0 <= (def.1 as u32)
}

#[derive(Default)]
struct AttrCollector {
    defs: Vec<AttrDef>,
}

impl AttrCollector {
    fn push(&mut self, kind: DefKind, name: &syn::Ident, attrs: &[syn::Attribute]) {
        let (start_line, end_line) = span_lines(name);
        self.defs.push(AttrDef {
            kind,
            name: name.to_string(),
            start_line,
            end_line,
            attrs: site_attr_texts(attrs),
        });
    }

    fn push_field(&mut self, field: &syn::Field) {
        if let Some(ident) = field.ident.as_ref() {
            self.push(DefKind::Field, ident, &field.attrs);
        }
    }

    fn push_variant(&mut self, variant: &syn::Variant) {
        self.push(DefKind::Variant, &variant.ident, &variant.attrs);
    }

    fn push_path_last(&mut self, path: &syn::Path, attrs: &[syn::Attribute]) {
        if let Some(seg) = path.segments.last() {
            self.push(DefKind::Item, &seg.ident, attrs);
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for AttrCollector {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.push(DefKind::Fn, &node.sig.ident, &node.attrs);
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.push(DefKind::Fn, &node.sig.ident, &node.attrs);
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.push(DefKind::Fn, &node.sig.ident, &node.attrs);
        syn::visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.push(DefKind::Item, &node.ident, &node.attrs);
        for field in &node.fields {
            self.push_field(field);
        }
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.push(DefKind::Item, &node.ident, &node.attrs);
        for variant in &node.variants {
            self.push_variant(variant);
        }
        syn::visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.push(DefKind::Item, &node.ident, &node.attrs);
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
        self.push(DefKind::Item, &node.ident, &node.attrs);
        syn::visit::visit_item_union(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.push(DefKind::Item, &node.ident, &node.attrs);
        syn::visit::visit_item_type(self, node);
    }

    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.push(DefKind::Item, &node.ident, &node.attrs);
        syn::visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.push(DefKind::Item, &node.ident, &node.attrs);
        syn::visit::visit_item_static(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.push(DefKind::Item, &node.ident, &node.attrs);
        syn::visit::visit_item_mod(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if let syn::Type::Path(tp) = node.self_ty.as_ref() {
            self.push_path_last(&tp.path, &node.attrs);
        }
        if let Some((_, path, _)) = node.trait_.as_ref() {
            self.push_path_last(path, &node.attrs);
        }
        syn::visit::visit_item_impl(self, node);
    }

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        if let Some(ident) = node.ident.as_ref() {
            self.push(DefKind::Item, ident, &node.attrs);
        }
        syn::visit::visit_item_macro(self, node);
    }
}

/// Attr-delta verdict for one post-image definition's attached set:
/// add requires every plus line present; remove requires no minus line
/// present; replace requires plus present and minus absent. Mirror
/// (equal sets) is always true — the caller only reaches here with a
/// delta, but the arm keeps the helper total.
fn site_attrs_ok(have: &[String], minus: &[String], plus: &[String]) -> bool {
    match attr_delta(minus, plus) {
        AttrDelta::Mirror => true,
        AttrDelta::Add => subsequence_indices(have, &norm_pat_list(plus)).is_some(),
        AttrDelta::Remove => subsequence_indices(have, &norm_pat_list(minus)).is_none(),
        AttrDelta::Replace => {
            let m = norm_pat_list(minus);
            let p = norm_pat_list(plus);
            subsequence_indices(have, &p).is_some() && subsequence_indices(have, &m).is_none()
        }
    }
}

/// Combined ident + attr-text span count for rename kinds.
///
/// Counts reported spans overlapping a collected definition that matches
/// `name` (`None` = hole: any name) *and* passes the attr-delta check.
/// With no delta the attr check is trivially true, so this subsumes the
/// name-only formulation exactly. Empty spans (byte-identical
/// self-rewrites) count file-wide, mirroring the name-only fallback.
pub(crate) fn count_spans_named_with_attrs(
    parsed: &syn::File,
    kind: DefKind,
    name: Option<&str>,
    minus_attrs: &[String],
    plus_attrs: &[String],
    spans: &[(u32, u32)],
) -> usize {
    let mut collector = AttrCollector::default();
    use syn::visit::Visit;
    collector.visit_file(parsed);
    let defs: Vec<&AttrDef> = collector
        .defs
        .iter()
        .filter(|d| d.kind == kind && name.is_none_or(|n| d.name == n))
        .filter(|d| site_attrs_ok(&d.attrs, minus_attrs, plus_attrs))
        .collect();
    if defs.is_empty() {
        return 0;
    }
    if spans.is_empty() {
        return defs.len();
    }
    spans
        .iter()
        .filter(|sp| {
            defs.iter()
                .any(|d| ranges_overlap((d.start_line, d.end_line), **sp))
        })
        .count()
}

/// `true` when the hunk carries an attr delta (normalized minus and plus
/// attr sets differ). The lift only runs on deltas; mirror hunks keep
/// the name-only verdict byte-identical.
pub(crate) fn has_attr_delta(minus_attrs: &[String], plus_attrs: &[String]) -> bool {
    norm_pat_list(minus_attrs) != norm_pat_list(plus_attrs)
}

/// Leading `-`/`+` attribute/doc lines of an RSmPL hunk body. Function
/// bodies are always `...`, so `-`/`+` lines starting with `#[` or `///`
/// are exactly the written attr delta (context lines carry no prefix and
/// never contribute).
pub(crate) fn hunk_attr_lines(rh: &rsmpl_patch::RsmplHunk) -> (Vec<String>, Vec<String>) {
    let mut minus = Vec::new();
    let mut plus = Vec::new();
    for l in &rh.lines {
        let t = l.text.trim();
        if !(t.starts_with("#[") || t.starts_with("///")) {
            continue;
        }
        // Normalize like the matcher side so the lift compares equal sets.
        let normed = norm_pat(&l.text);
        match l.prefix {
            '-' => minus.push(normed),
            '+' => plus.push(normed),
            _ => {}
        }
    }
    (minus, plus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lines::LineIndex;
    use std::collections::HashMap;

    fn parse(text: &str) -> syn::File {
        syn::parse_str(text).expect("fixture must parse")
    }

    /// Lift-level helper mirroring the production [`crate::plan`] call
    /// shape: parse the patch, compile it, and run the real
    /// `plus_instantiated_parsed` over a freshly parsed post-image.
    fn plus_instantiated(
        patch: &str,
        post_text: &str,
        spans: &[(u32, u32)],
        matches: usize,
    ) -> Option<bool> {
        let parsed = rsmpl_patch::parse_patch(patch).unwrap();
        let rh = match &parsed.files[0].hunks[0] {
            rsmpl_patch::Hunk::Rsmpl(r) => r.clone(),
            _ => panic!("expected rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&rh).unwrap();
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        let post_parsed = syn::parse_str::<syn::File>(post_text).ok();
        let index = LineIndex::new(post_text);
        crate::postcheck::plus_instantiated_parsed(
            &compiled,
            &rh,
            &bindings,
            post_text,
            post_parsed.as_ref(),
            Some(&index),
            spans,
            matches,
        )
    }

    #[test]
    fn counts_ident_overlapping_spans_with_attrs() {
        let file = parse("#[derive(Debug)]\nstruct New {}\nstruct New {}\n");
        // Both defs carry the plus attr: both spans count.
        assert_eq!(
            count_spans_named_with_attrs(
                &file,
                DefKind::Item,
                Some("New"),
                &["#[derive(Debug)]".to_string()],
                &["#[derive(Debug, Clone)]".to_string()],
                &[(1, 2)]
            ),
            0,
            "stale derive must not count"
        );
        let file = parse("#[derive(Debug, Clone)]\nstruct New {}\nstruct Other {}\n");
        assert_eq!(
            count_spans_named_with_attrs(
                &file,
                DefKind::Item,
                Some("New"),
                &["#[derive(Debug)]".to_string()],
                &["#[derive(Debug, Clone)]".to_string()],
                &[(1, 2)]
            ),
            1
        );
        // Hole names match any ident with the attr state.
        assert_eq!(
            count_spans_named_with_attrs(
                &file,
                DefKind::Item,
                None,
                &["#[derive(Debug)]".to_string()],
                &["#[derive(Debug, Clone)]".to_string()],
                &[(1, 2)]
            ),
            1
        );
        // Name mismatch never counts, even with the attr present.
        assert_eq!(
            count_spans_named_with_attrs(
                &file,
                DefKind::Item,
                Some("Missing"),
                &["#[derive(Debug)]".to_string()],
                &["#[derive(Debug, Clone)]".to_string()],
                &[(1, 2)]
            ),
            0
        );
    }

    #[test]
    fn mirror_delta_is_trivially_true() {
        let file = parse("struct New {}\n");
        assert_eq!(
            count_spans_named_with_attrs(
                &file,
                DefKind::Item,
                Some("New"),
                &["#[inline]".to_string()],
                &["#[inline]".to_string()],
                &[(1, 1)]
            ),
            1,
            "mirror keeps the name-only count"
        );
    }

    #[test]
    fn remove_requires_absence() {
        let kept = parse("struct New {}\n");
        assert_eq!(
            count_spans_named_with_attrs(
                &kept,
                DefKind::Item,
                Some("New"),
                &["#[inline]".to_string()],
                &[],
                &[(1, 1)]
            ),
            1
        );
        let stale = parse("#[inline]\nstruct New {}\n");
        assert_eq!(
            count_spans_named_with_attrs(
                &stale,
                DefKind::Item,
                Some("New"),
                &["#[inline]".to_string()],
                &[],
                &[(1, 2)]
            ),
            0,
            "stale minus attr must not count"
        );
    }

    #[test]
    fn hunk_attr_lines_collects_only_signed_attrs() {
        let patch = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 @@ rule=r\n  trait T {\n- #[inline]\n- fn a();\n+ #[other]\n+ fn a();\n      ...\n  }\n";
        let parsed = rsmpl_patch::parse_patch(patch).unwrap();
        let rh = match &parsed.files[0].hunks[0] {
            rsmpl_patch::Hunk::Rsmpl(r) => r.clone(),
            _ => panic!("expected rsmpl hunk"),
        };
        let (minus, plus) = hunk_attr_lines(&rh);
        assert_eq!(minus, vec!["#[inline]".to_string()]);
        assert_eq!(plus, vec!["#[other]".to_string()]);
    }

    #[test]
    fn item_attr_add_lift_fires_and_tamper_fails() {
        // An attr-add evolution verifies the plus attr text at the span; a
        // span whose attr was reverted fails even when the same attr occurs
        // on another line.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- struct Old\n+ #[derive(Debug)]\n+ struct Old\n";
        assert_eq!(
            plus_instantiated(patch, "#[derive(Debug)]\nstruct Old {}\n", &[(1, 2)], 1),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(
                patch,
                "struct Old {}\nstruct Other {}\n#[derive(Debug)]\nstruct Third {}\n",
                &[(1, 1)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn item_attr_remove_lift_requires_absence() {
        // An attr-remove evolution verifies the minus attr is gone at the
        // span; a post-image still carrying it fails.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- #[inline]\n- struct Old\n+ struct Old\n";
        assert_eq!(
            plus_instantiated(patch, "struct Old {}\n", &[(1, 1)], 1),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(patch, "#[inline]\nstruct Old {}\n", &[(1, 2)], 1),
            Some(false)
        );
    }

    #[test]
    fn item_hole_attr_lift_binds_any_name() {
        // An `N`-hole evolution verifies the attr text at any ident
        // overlapping the span (names are preserved, so no name verdict is
        // owed); a reverted span fails.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- struct N\n+ #[derive(Debug)]\n+ struct N\n";
        assert_eq!(
            plus_instantiated(
                patch,
                "#[derive(Debug)]\nstruct Whatever {}\n",
                &[(1, 2)],
                1,
            ),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(patch, "struct Whatever {}\n", &[(1, 1)], 1),
            Some(false)
        );
    }

    #[test]
    fn fn_attr_add_lift_fires_and_tamper_fails() {
        // A fn attr-add evolution verifies the plus attr text at the span;
        // the ident alone never suffices.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n- fn foo(...) {\n+ #[inline]\n+ fn foo(...) {\n      ...\n  }\n";
        assert_eq!(
            plus_instantiated(patch, "#[inline]\nfn foo() {}\n", &[(1, 2)], 1),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(patch, "fn foo() {}\n", &[(1, 1)], 1),
            Some(false)
        );
    }
}
