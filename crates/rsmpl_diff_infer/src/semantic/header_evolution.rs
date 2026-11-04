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
use super::prov::Counted;
use super::unique_rule;

/// N-site (N ≥ 2) header slot-only evolution sweep. See module docs.
pub(crate) fn try_header_evolution_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    // WI-19b: N=1 name-preserving evolutions belong to
    // `try_sig_evolution_sweep` (concrete-name emission, tighter
    // totality); this detector keeps its hetero-names class (N≥2).
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return Counted::Miss;
    }
    let all_indented = minuses.iter().all(|m| m.starts_with([' ', '\t']));
    let all_col0 = minuses
        .iter()
        .all(|m| !m.is_empty() && !m.starts_with([' ', '\t']));
    if !all_indented && !all_col0 {
        return Counted::Miss;
    }
    let plus_indented = pluses.iter().all(|p| p.starts_with([' ', '\t']));
    let plus_col0 = pluses
        .iter()
        .all(|p| !p.is_empty() && !p.starts_with([' ', '\t']));
    if all_indented != plus_indented || all_col0 != plus_col0 {
        return Counted::Miss;
    }
    let mut minus_slots = Vec::with_capacity(minuses.len());
    let mut plus_slots = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some(__m) = parse_sig_slots(m) else {
            return Counted::Miss;
        };
        let Some(__p) = parse_sig_slots(p) else {
            return Counted::Miss;
        };
        minus_slots.push(__m);
        plus_slots.push(__p);
    }
    // Names preserved per site, hetero across sites (the sibling owns
    // shared-name classes; a name delta anywhere is not evolution).
    let mut names: Vec<&str> = Vec::with_capacity(minus_slots.len());
    for (m, p) in minus_slots.iter().zip(plus_slots.iter()) {
        if m.name != p.name {
            return Counted::Miss;
        }
        names.push(m.name.as_str());
    }
    names.sort();
    names.dedup();
    if names.len() < 2 {
        return Counted::Miss;
    }
    let vis = match classify_slot(
        slot_col(&minus_slots, |s| &s.vis),
        slot_col(&plus_slots, |s| &s.vis),
        false,
    ) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let qual = match classify_slot(
        slot_col(&minus_slots, |s| &s.qual),
        slot_col(&plus_slots, |s| &s.qual),
        false,
    ) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let generics = match classify_slot(
        slot_col(&minus_slots, |s| &s.generics),
        slot_col(&plus_slots, |s| &s.generics),
        false,
    ) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let params = match classify_slot(
        slot_col(&minus_slots, |s| &s.params),
        slot_col(&plus_slots, |s| &s.params),
        true,
    ) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let ret = match classify_slot(
        slot_col(&minus_slots, |s| &s.ret),
        slot_col(&plus_slots, |s| &s.ret),
        false,
    ) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let where_ = match classify_slot(
        slot_col(&minus_slots, |s| &s.where_),
        slot_col(&plus_slots, |s| &s.where_),
        true,
    ) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    // Exactly one evolving non-name slot: zero is nothing to raise,
    // two or more stay ordinary (never half-claimed under one rule).
    let evolving = [&vis, &qual, &generics, &params, &ret, &where_]
        .iter()
        .filter(|c| matches!(c, SlotClass::Evolving { .. }))
        .count();
    if evolving != 1 {
        return Counted::Miss;
    }
    // Trait-context `pub` is illegal Rust — vis deltas over trait
    // members decline even though the sibling emission paths would
    // structurally exclude most of them already.
    if matches!(vis, SlotClass::Evolving { .. }) && any_site_in_trait_block(file_old, changed) {
        return Counted::Miss;
    }
    let (vis_m, vis_p) = match vis.plain(Emit::Wild) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let (qual_m, qual_p) = match qual.plain(Emit::Wild) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let (gen_m, gen_p) = match generics.plain(Emit::Wild) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let (params_m, params_p) = params.params();
    let (ret_m, ret_p) = match ret.ret() {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
    let (where_m, where_p) = match where_.plain(Emit::Wild) {
        Some(__v) => __v,
        None => return Counted::Miss,
    };
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
        return Counted::Miss;
    }
    let rule = unique_rule(used_rules, "header_evolution");
    if all_indented {
        let pre = match file_old {
            Some(__v) => __v,
            None => return Counted::Miss,
        };
        let groups = match block_headers::group_sites_by_impl_block(file_old, changed) {
            Some(__v) => __v,
            None => return Counted::Miss,
        };
        let mut out = String::new();
        for g in &groups {
            // WI-4 slice 2b-1: per-group block counts name `totality:`;
            // a probe-only failure stays `Miss`.
            match crate::totality_counted::engine_verdict(
                file_old,
                crate::totality_counted::template_fn_block_counts(
                    path,
                    file_old,
                    &minus_header,
                    &g.header_line,
                ),
                g.sites.len(),
            ) {
                crate::totality_counted::FamilyVerdict::Holds => {}
                crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
                    return Counted::Totality { found, want };
                }
                crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
                crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
            }
            if !block_headers::probe_block_hunk(
                path,
                pre,
                &g.header_line,
                &minus_header,
                &plus_header,
                g.sites.len(),
            ) {
                return Counted::Miss;
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
        return Counted::Fire(out);
    }
    // WI-4 slice 2b-1: free-fn engine count names `totality:` like the
    // block path above. No opaque arm by construction.
    match crate::totality_counted::engine_verdict(
        file_old,
        crate::totality_counted::template_fn_counts(path, file_old, &minus_header),
        minuses.len(),
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let mut hunk = crate::semantic::item_emission_header(path, &rule, file_old, changed);
    hunk.push_str(&format!(
        "- {minus_header}\n+ {plus_header}\n      ...\n  }}\n"
    ));
    Counted::Fire(hunk)
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
    fn vis_add_twin_vetoes_whole_section() {
        // WI-19b: an untouched fifth plain fn matches the `N`-hole minus
        // header too, so the whole-section engine-count totality
        // (5 != 4) still vetoes the sweep. Per-hunk, each pair is a
        // sound sig single (concrete names never match `extra`), so the
        // pipeline claims all four and leaves `extra` alone.
        let raw = section("f.rs", &vis_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nfn beta(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\nfn gamma(third_argument: ThirdLongTypeArgument) {\n    third_argument\n}\nfn delta(fourth_argument: FourthLongTypeArgument) {\n    fourth_argument\n}\nfn extra(fifth_argument: FifthLongTypeArgument) {\n    fifth_argument\n}\n",
        )]);
        let mut used = Vec::new();
        // WI-4 slice 2b-1: the twin veto now names `totality:5:4`.
        assert!(matches!(
            super::try_header_evolution_sweep(
                &[
                    "fn alpha(first_argument: FirstLongTypeArgument) {",
                    "fn beta(second_argument: SecondLongTypeArgument) {",
                    "fn gamma(third_argument: ThirdLongTypeArgument) {",
                    "fn delta(fourth_argument: FourthLongTypeArgument) {"
                ],
                &[
                    "pub fn alpha(first_argument: FirstLongTypeArgument) {",
                    "pub fn beta(second_argument: SecondLongTypeArgument) {",
                    "pub fn gamma(third_argument: ThirdLongTypeArgument) {",
                    "pub fn delta(fourth_argument: FourthLongTypeArgument) {"
                ],
                "f.rs",
                Some(&pre["f.rs"]),
                &[1, 4, 7, 10],
                &mut used,
            ),
            super::super::prov::Counted::Totality { found: 5, want: 4 }
        ));
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("per-hunk sig singles must fire");
        let files = validated_live(&out, &pre);
        assert!(
            files["f.rs"].contains("pub fn delta("),
            "got: {}",
            files["f.rs"]
        );
        assert!(
            files["f.rs"].contains("fn extra(fifth_argument: FifthLongTypeArgument) {"),
            "twin untouched, got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn generics_bound_add_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            // WI-1b: the plus side uses the engine-normalized `<T: Clone>`
            // spacing — the floor proves git-post == engine-post
            // byte-identically, and the engine compacts slot spacing at
            // compare time (so the unformatted `<T : Clone>` input the
            // detector also accepts is not floor-provable).
            "@@ -1,1 +1,1 @@\n-fn foo<T>(x: T) {\n+fn foo<T: Clone>(x: T) {\n@@ -4,1 +4,1 @@\n-fn bar<T>(x: T) {\n+fn bar<T: Clone>(x: T) {\n",
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
    fn name_delta_declines_whole_section() {
        // One renamed site is not a header sweep — the whole-section
        // detector declines (name delta). Per-hunk, each pair is a
        // sound WI-19b sig single (concrete names), so the pipeline
        // claims both hunks and round-trips.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(first_argument: FirstLongTypeArgument) {\n+pub fn alpha(first_argument: FirstLongTypeArgument) {\n@@ -4,1 +4,1 @@\n-fn beta(second_argument: SecondLongTypeArgument) {\n+pub fn gamma(second_argument: SecondLongTypeArgument) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nfn beta(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(super::try_header_evolution_sweep(
            &[
                "fn alpha(first_argument: FirstLongTypeArgument) {",
                "fn beta(second_argument: SecondLongTypeArgument) {"
            ],
            &[
                "pub fn alpha(first_argument: FirstLongTypeArgument) {",
                "pub fn gamma(second_argument: SecondLongTypeArgument) {"
            ],
            "f.rs",
            Some(&pre["f.rs"]),
            &[1, 4],
            &mut used,
        )
        .is_miss());
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("per-hunk sig singles must fire");
        let files = validated_live(&out, &pre);
        assert!(
            files["f.rs"].contains("pub fn alpha("),
            "got: {}",
            files["f.rs"]
        );
        assert!(
            files["f.rs"].contains("pub fn gamma("),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn two_slot_mixed_declines_whole_section() {
        // Three `pub` adds plus one `async` add: the plus side is
        // divergent, so no uniform sweep class exists — the
        // whole-section detector declines. Per-hunk, each pair is a
        // sound WI-19b sig single, so the pipeline claims all four.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(first_argument: FirstLongTypeArgument) {\n+pub fn alpha(first_argument: FirstLongTypeArgument) {\n@@ -4,1 +4,1 @@\n-fn beta(second_argument: SecondLongTypeArgument) {\n+pub fn beta(second_argument: SecondLongTypeArgument) {\n@@ -7,1 +7,1 @@\n-fn gamma(third_argument: ThirdLongTypeArgument) {\n+pub fn gamma(third_argument: ThirdLongTypeArgument) {\n@@ -10,1 +10,1 @@\n-fn delta(fourth_argument: FourthLongTypeArgument) {\n+async fn delta(fourth_argument: FourthLongTypeArgument) {\n",
        );
        let pre = pre_map(&[("f.rs", PRE_VIS)]);
        let mut used = Vec::new();
        assert!(super::try_header_evolution_sweep(
            &[
                "fn alpha(first_argument: FirstLongTypeArgument) {",
                "fn beta(second_argument: SecondLongTypeArgument) {",
                "fn gamma(third_argument: ThirdLongTypeArgument) {",
                "fn delta(fourth_argument: FourthLongTypeArgument) {"
            ],
            &[
                "pub fn alpha(first_argument: FirstLongTypeArgument) {",
                "pub fn beta(second_argument: SecondLongTypeArgument) {",
                "pub fn gamma(third_argument: ThirdLongTypeArgument) {",
                "async fn delta(fourth_argument: FourthLongTypeArgument) {"
            ],
            "f.rs",
            Some(&pre["f.rs"]),
            &[1, 4, 7, 10],
            &mut used,
        )
        .is_miss());
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("per-hunk sig singles must fire");
        let files = validated_live(&out, &pre);
        assert!(
            files["f.rs"].contains("pub fn alpha("),
            "got: {}",
            files["f.rs"]
        );
        assert!(
            files["f.rs"].contains("async fn delta("),
            "got: {}",
            files["f.rs"]
        );
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
        // WI-19b: N=1 name-preserving evolutions belong to the sig sweep
        // (concrete-name emission); this detector keeps N ≥ 2.
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
        .is_miss());
        assert!(used.is_empty());
    }
}
