//! Header slot-only evolution sweep (WI-9): one uniform slot delta
//! over hetero preserved names.
//!
//! The sibling `sig_evolution` detector (in `generalized.rs`) owns
//! shared-name classes (one name delta plus slot deltas); this owns
//! the disjoint hetero-name class: N ≥ 2 single-line `{`-terminated
//! fn-header pairs where every name is preserved per site
//! (`minus name == plus name`) but names differ across sites, and
//! exactly one non-name slot (vis/qual/generics/params/return/
//! where) carries one uniform delta. Two evolving slots, any name
//! delta, or a single shared name stay ordinary (or with the
//! sibling) — never half-claimed here.
//!
//! Emission is the WI-5 name-hole spelling (`fn N` with the evolving
//! slot concrete and hetero preserved slots wildcarded, under the
//! same `V Q` pairing and `-> T W` fixups as the sibling).
//! Totality is the same engine-count posture: the emitted minus
//! header must match exactly N headers in the pre-image (free fns
//! via `template_fn_holds`, methods one hunk per enclosing `impl`
//! block via `probe_block_hunk`) — a same-shape header the diff
//! left alone vetoes. Trait-context `vis` deltas decline at emission
//! (`pub` is illegal in traits).

use super::block_headers;
use super::generalized::{
    build_sig_header, classify_slot, parse_sig_slots, slot_col, vq_pair, Emit, SlotClass,
};
use super::unique_rule;

/// N-site (N ≥ 2) header slot-only evolution sweep. See module docs.
pub(crate) fn try_header_evolution_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let all_indented = minuses.iter().all(|m| m.starts_with([' ', '\t']));
    let all_col0 = minuses
        .iter()
        .all(|m| !m.is_empty() && !m.starts_with([' ', '\t']));
    if !all_indented && !all_col0 {
        return None;
    }
    let plus_indented = pluses.iter().all(|p| p.starts_with([' ', '\t']));
    let plus_col0 = pluses
        .iter()
        .all(|p| !p.is_empty() && !p.starts_with([' ', '\t']));
    if all_indented != plus_indented || all_col0 != plus_col0 {
        return None;
    }
    let mut minus_slots = Vec::with_capacity(minuses.len());
    let mut plus_slots = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        minus_slots.push(parse_sig_slots(m)?);
        plus_slots.push(parse_sig_slots(p)?);
    }
    // Names preserved per site, hetero across sites (the sibling owns
    // shared-name classes; a name delta anywhere is not evolution).
    let mut names: Vec<&str> = Vec::with_capacity(minus_slots.len());
    for (m, p) in minus_slots.iter().zip(plus_slots.iter()) {
        if m.name != p.name {
            return None;
        }
        names.push(m.name.as_str());
    }
    names.sort();
    names.dedup();
    if names.len() < 2 {
        return None;
    }
    let vis = classify_slot(
        slot_col(&minus_slots, |s| &s.vis),
        slot_col(&plus_slots, |s| &s.vis),
        false,
    )?;
    let qual = classify_slot(
        slot_col(&minus_slots, |s| &s.qual),
        slot_col(&plus_slots, |s| &s.qual),
        false,
    )?;
    let generics = classify_slot(
        slot_col(&minus_slots, |s| &s.generics),
        slot_col(&plus_slots, |s| &s.generics),
        false,
    )?;
    let params = classify_slot(
        slot_col(&minus_slots, |s| &s.params),
        slot_col(&plus_slots, |s| &s.params),
        true,
    )?;
    let ret = classify_slot(
        slot_col(&minus_slots, |s| &s.ret),
        slot_col(&plus_slots, |s| &s.ret),
        false,
    )?;
    let where_ = classify_slot(
        slot_col(&minus_slots, |s| &s.where_),
        slot_col(&plus_slots, |s| &s.where_),
        true,
    )?;
    // Exactly one evolving non-name slot: zero is nothing to raise,
    // two or more stay ordinary (never half-claimed under one rule).
    let evolving = [&vis, &qual, &generics, &params, &ret, &where_]
        .iter()
        .filter(|c| matches!(c, SlotClass::Evolving { .. }))
        .count();
    if evolving != 1 {
        return None;
    }
    // Trait-context `pub` is illegal Rust — vis deltas over trait
    // members decline even though the sibling emission paths would
    // structurally exclude most of them already.
    if matches!(vis, SlotClass::Evolving { .. }) && any_site_in_trait_block(file_old, changed) {
        return None;
    }
    let (vis_m, vis_p) = vis.plain(Emit::Wild)?;
    let (qual_m, qual_p) = qual.plain(Emit::Wild)?;
    let (gen_m, gen_p) = generics.plain(Emit::Wild)?;
    let (params_m, params_p) = params.params();
    let (ret_m, ret_p) = ret.ret()?;
    let (where_m, where_p) = where_.plain(Emit::Wild)?;
    let (vis_m, qual_m) = vq_pair(vis_m, qual_m);
    let (vis_p, qual_p) = vq_pair(vis_p, qual_p);
    let (ret_m, where_m) = if ret_m.is_some() && matches!(where_m, Emit::Wild) {
        (None, where_m)
    } else {
        (ret_m, where_m)
    };
    let (ret_p, where_p) = if ret_p.is_some() && matches!(where_p, Emit::Wild) {
        (ret_p, Emit::Omit)
    } else {
        (ret_p, where_p)
    };
    let minus_header = build_sig_header(
        "N",
        vis_m.text("V").as_deref(),
        qual_m.text("Q").as_deref(),
        gen_m.text("G").as_deref(),
        params_m.as_deref(),
        ret_m.as_deref(),
        where_m.text("W").as_deref(),
    );
    let plus_header = build_sig_header(
        "N",
        vis_p.text("V").as_deref(),
        qual_p.text("Q").as_deref(),
        gen_p.text("G").as_deref(),
        params_p.as_deref(),
        ret_p.as_deref(),
        where_p.text("W").as_deref(),
    );
    if minus_header == plus_header {
        return None;
    }
    let rule = unique_rule(used_rules, "header_evolution");
    if all_indented {
        let pre = file_old?;
        let groups = block_headers::group_sites_by_impl_block(file_old, changed)?;
        let mut out = String::new();
        for g in &groups {
            if !block_headers::probe_block_hunk(
                path,
                pre,
                &g.header_line,
                &minus_header,
                &plus_header,
                g.sites.len(),
            ) {
                return None;
            }
            out.push_str(&crate::semantic::item_emission_header(
                path, &rule, file_old, &g.sites,
            ));
            out.push_str(&block_headers::block_hunk_body(
                &g.header_line,
                &minus_header,
                &plus_header,
            ));
        }
        return Some(out);
    }
    if !crate::totality::template_fn_holds(path, file_old, &minus_header, None, minuses.len()) {
        return None;
    }
    let mut hunk = crate::semantic::item_emission_header(path, &rule, file_old, changed);
    hunk.push_str(&format!(
        "- {minus_header}\n+ {plus_header}\n      ...\n  }}\n"
    ));
    Some(hunk)
}

/// True when any changed 1-based site lies inside a `trait` block
/// span in the pre-image (top-level and inline-mod items). `None`
/// pre-image (or unparseable) never counts — the sibling probes
/// keep their legacy allow and the emission probe re-verifies.
fn any_site_in_trait_block(file_old: Option<&str>, changed: &[usize]) -> bool {
    let Some(pre) = file_old else {
        return false;
    };
    let Ok(file) = syn::parse_str::<syn::File>(pre) else {
        return false;
    };
    let mut spans: Vec<(usize, usize)> = Vec::new();
    fn walk(items: &[syn::Item], spans: &mut Vec<(usize, usize)>) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Trait(tr) => {
                    spans.push((tr.span().start().line, tr.span().end().line));
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, spans);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, &mut spans);
    changed
        .iter()
        .any(|site| spans.iter().any(|(s, e)| *s <= *site && *site <= *e))
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic, infer_semantic_with_pre_images};
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
        files
    }

    const PRE_VIS: &str = "fn alpha(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nfn beta(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\nfn gamma(third_argument: ThirdLongTypeArgument) {\n    third_argument\n}\nfn delta(fourth_argument: FourthLongTypeArgument) {\n    fourth_argument\n}\n";
    const POST_VIS: &str = "pub fn alpha(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\npub fn beta(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\npub fn gamma(third_argument: ThirdLongTypeArgument) {\n    third_argument\n}\npub fn delta(fourth_argument: FourthLongTypeArgument) {\n    fourth_argument\n}\n";

    fn vis_body() -> String {
        "@@ -1,1 +1,1 @@\n-fn alpha(first_argument: FirstLongTypeArgument) {\n+pub fn alpha(first_argument: FirstLongTypeArgument) {\n@@ -4,1 +4,1 @@\n-fn beta(second_argument: SecondLongTypeArgument) {\n+pub fn beta(second_argument: SecondLongTypeArgument) {\n@@ -7,1 +7,1 @@\n-fn gamma(third_argument: ThirdLongTypeArgument) {\n+pub fn gamma(third_argument: ThirdLongTypeArgument) {\n@@ -10,1 +10,1 @@\n-fn delta(fourth_argument: FourthLongTypeArgument) {\n+pub fn delta(fourth_argument: FourthLongTypeArgument) {\n"
            .to_string()
    }

    #[test]
    fn vis_add_hetero_names_fires_and_round_trips() {
        let raw = section("f.rs", &vis_body());
        let pre = pre_map(&[("f.rs", PRE_VIS)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("vis sweep must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("- fn N(...) {\n"), "got: {out}");
        assert!(out.contains("+ pub fn N(...) {\n"), "got: {out}");
        assert!(!out.contains("# visibility"), "got: {out}");
        let files = validated_live(&out, &pre);
        assert_eq!(files["f.rs"], POST_VIS);
    }

    #[test]
    fn vis_add_twin_vetoes() {
        // An untouched fifth plain fn matches the minus header too, so
        // the engine-count totality (5 != 4) keeps the section ordinary.
        let raw = section("f.rs", &vis_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nfn beta(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\nfn gamma(third_argument: ThirdLongTypeArgument) {\n    third_argument\n}\nfn delta(fourth_argument: FourthLongTypeArgument) {\n    fourth_argument\n}\nfn extra(fifth_argument: FifthLongTypeArgument) {\n    fifth_argument\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn generics_bound_add_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn foo<T>(x: T) {\n+fn foo<T : Clone>(x: T) {\n@@ -4,1 +4,1 @@\n-fn bar<T>(x: T) {\n+fn bar<T : Clone>(x: T) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn foo<T>(x: T) {\n    x\n}\nfn bar<T>(x: T) {\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("bound sweep must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("fn N"), "got: {out}");
        assert!(out.contains("Clone"), "got: {out}");
        let files = validated_live(&out, &pre);
        // The engine compacts slot spacing at compare time, so the
        // post-image carries `<T: Clone>` (not the pretty `<T : Clone>`).
        assert_eq!(
            files["f.rs"],
            "fn foo<T: Clone>(x: T) {\n    x\n}\nfn bar<T: Clone>(x: T) {\n    x\n}\n"
        );
    }

    #[test]
    fn name_delta_declines() {
        // One renamed site is not evolution — the sibling detectors own
        // name deltas, this class stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(first_argument: FirstLongTypeArgument) {\n+pub fn alpha(first_argument: FirstLongTypeArgument) {\n@@ -4,1 +4,1 @@\n-fn beta(second_argument: SecondLongTypeArgument) {\n+pub fn gamma(second_argument: SecondLongTypeArgument) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nfn beta(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn two_slot_mixed_declines() {
        // Three `pub` adds plus one `async` add: the plus side is
        // divergent, so no uniform class exists — stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(first_argument: FirstLongTypeArgument) {\n+pub fn alpha(first_argument: FirstLongTypeArgument) {\n@@ -4,1 +4,1 @@\n-fn beta(second_argument: SecondLongTypeArgument) {\n+pub fn beta(second_argument: SecondLongTypeArgument) {\n@@ -7,1 +7,1 @@\n-fn gamma(third_argument: ThirdLongTypeArgument) {\n+pub fn gamma(third_argument: ThirdLongTypeArgument) {\n@@ -10,1 +10,1 @@\n-fn delta(fourth_argument: FourthLongTypeArgument) {\n+async fn delta(fourth_argument: FourthLongTypeArgument) {\n",
        );
        let pre = pre_map(&[("f.rs", PRE_VIS)]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn trait_vis_add_declines() {
        // `pub` inside a trait is illegal — trait-context vis deltas
        // decline at emission even though the impl-block path would
        // structurally exclude them too.
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    fn alpha(&self) {\n+    pub fn alpha(&self) {\n@@ -5,1 +5,1 @@\n-    fn beta(&self) {\n+    pub fn beta(&self) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "trait T {\n    fn alpha(&self) {\n        1\n    }\n    fn beta(&self) {\n        2\n    }\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn method_vis_add_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    fn alpha(&self, first_argument: FirstLongTypeArgument) {\n+    pub fn alpha(&self, first_argument: FirstLongTypeArgument) {\n@@ -5,1 +5,1 @@\n-    fn beta(&self, second_argument: SecondLongTypeArgument) {\n+    pub fn beta(&self, second_argument: SecondLongTypeArgument) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "impl Svc {\n    fn alpha(&self, first_argument: FirstLongTypeArgument) {\n        1\n    }\n    fn beta(&self, second_argument: SecondLongTypeArgument) {\n        2\n    }\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("method vis sweep must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("fn N"), "got: {out}");
        assert!(out.contains("impl Svc {"), "got: {out}");
        let files = validated_live(&out, &pre);
        assert_eq!(
            files["f.rs"],
            "impl Svc {\n    pub fn alpha(&self, first_argument: FirstLongTypeArgument) {\n        1\n    }\n    pub fn beta(&self, second_argument: SecondLongTypeArgument) {\n        2\n    }\n}\n"
        );
    }

    #[test]
    fn qual_async_add_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha() {\n+async fn alpha() {\n@@ -4,1 +4,1 @@\n-fn beta() {\n+async fn beta() {\n",
        );
        let pre = pre_map(&[("f.rs", "fn alpha() {\n    1\n}\nfn beta() {\n    2\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("qual sweep must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("- fn N() {\n"), "got: {out}");
        assert!(out.contains("+ async fn N() {\n"), "got: {out}");
        let files = validated_live(&out, &pre);
        assert_eq!(
            files["f.rs"],
            "async fn alpha() {\n    1\n}\nasync fn beta() {\n    2\n}\n"
        );
    }

    #[test]
    fn params_add_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) {\n+fn alpha(x: i32, y: i32) {\n@@ -4,1 +4,1 @@\n-fn beta(x: i32) {\n+fn beta(x: i32, y: i32) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) {\n    x\n}\nfn beta(x: i32) {\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("params sweep must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("fn N"), "got: {out}");
        let files = validated_live(&out, &pre);
        assert_eq!(
            files["f.rs"],
            "fn alpha(x: i32, y: i32) {\n    x\n}\nfn beta(x: i32, y: i32) {\n    x\n}\n"
        );
    }

    #[test]
    fn return_add_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) {\n+fn alpha(x: i32) -> i32 {\n@@ -4,1 +4,1 @@\n-fn beta(x: i32) {\n+fn beta(x: i32) -> i32 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) {\n    x\n}\nfn beta(x: i32) {\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("return sweep must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("-> i32"), "got: {out}");
        let files = validated_live(&out, &pre);
        assert_eq!(
            files["f.rs"],
            "fn alpha(x: i32) -> i32 {\n    x\n}\nfn beta(x: i32) -> i32 {\n    x\n}\n"
        );
    }

    #[test]
    fn where_add_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha<T>(x: T) {\n+fn alpha<T>(x: T) where T: Clone {\n@@ -4,1 +4,1 @@\n-fn beta<T>(x: T) {\n+fn beta<T>(x: T) where T: Clone {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha<T>(x: T) {\n    x\n}\nfn beta<T>(x: T) {\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("where sweep must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("where"), "got: {out}");
        let files = validated_live(&out, &pre);
        assert_eq!(
            files["f.rs"],
            "fn alpha<T>(x: T) where T: Clone {\n    x\n}\nfn beta<T>(x: T) where T: Clone {\n    x\n}\n"
        );
    }

    #[test]
    fn vis_removal_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-pub fn alpha() {\n+fn alpha() {\n@@ -4,1 +4,1 @@\n-pub fn beta() {\n+fn beta() {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "pub fn alpha() {\n    1\n}\npub fn beta() {\n    2\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("vis-removal sweep must fire");
        assert!(out.contains("- pub fn N() {\n"), "got: {out}");
        assert!(out.contains("+ fn N() {\n"), "got: {out}");
        let files = validated_live(&out, &pre);
        assert_eq!(
            files["f.rs"],
            "fn alpha() {\n    1\n}\nfn beta() {\n    2\n}\n"
        );
    }

    #[test]
    fn return_change_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn alpha(x: i32) -> u64 {\n@@ -4,1 +4,1 @@\n-fn beta(x: i32) -> i32 {\n+fn beta(x: i32) -> u64 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) -> i32 {\n    x\n}\nfn beta(x: i32) -> i32 {\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("return-change sweep must fire");
        assert!(out.contains("-> u64"), "got: {out}");
        let files = validated_live(&out, &pre);
        assert_eq!(
            files["f.rs"],
            "fn alpha(x: i32) -> u64 {\n    x\n}\nfn beta(x: i32) -> u64 {\n    x\n}\n"
        );
    }

    #[test]
    fn single_pair_declines() {
        // Singles never reach the pair chain; the detector itself also
        // requires N ≥ 2.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(first_argument: FirstLongTypeArgument) {\n+pub fn alpha(first_argument: FirstLongTypeArgument) {\n",
        );
        assert!(infer_semantic(&[raw]).is_none());
        let mut used = Vec::new();
        assert!(super::try_header_evolution_sweep(
            &["fn alpha(first_argument: FirstLongTypeArgument) {"],
            &["pub fn alpha(first_argument: FirstLongTypeArgument) {"],
            "f.rs",
            None,
            &[1],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }
}
