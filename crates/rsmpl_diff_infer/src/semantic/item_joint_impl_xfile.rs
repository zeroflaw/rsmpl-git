//! WI-12 cross-file joint-`impl` path (D5 split from `item_joint_impl`).
//!
//! The trait pair, self-type pair, and composed `impl` double-delta may
//! sit in different files; extras ride the follow from any file. The
//! emission is one `diff --git` section per def file sharing one rule
//! (assoc pre-pass shape). Single-section-pure shapes decline here (the
//! single-file arm owns them byte-identically).
//!
//! WI-12 slice 3 parity: mixed use-only sections decline at section
//! level — even when the claimed and unclaimed pairs sit in different
//! hunks. Per-hunk leftover surgery is unsound here by engine
//! construction: pass 0 parses a patch-target file only when some
//! section for that path carries an RSmPL hunk (T137), so a file that
//! would keep an ordinary remainder is never Rust-parsed and no
//! cross-file follow can reach into it — only fully-collapsed
//! (follow-owned) use-only files compose with the joint (probed: the
//! follow rewrites a use-only file with no patch section, but leaves
//! the same file untouched once it gains an ordinary-only section).
//! Same-hunk mixed declines by hunk atomicity on top of the same law.
//! Def files stay pure. All three boundaries are pinned below.

use std::collections::{HashMap, HashSet};

use super::item_intent::{count_kind_defs, Pair};
use super::item_joint_impl::{extra_claimable, parse_impl_double_delta, split_def_pair};
use super::{split_item, unique_rule};

/// One harvested balanced pair.
struct GPair {
    sec: usize,
    path: String,
    minus: String,
    plus: String,
    site: usize,
}

/// Whole-patch cross-file joint-`impl` pre-pass: returns the rebuilt
/// section list (def files get their emission sections, pure use-only
/// files collapse, mixed use-only files keep an ordinary remainder)
/// plus the definition path and rule for provenance, or `None` when
/// the patch holds no provable cross-file double-delta intent.
/// Reserved rule names are released on decline.
pub(crate) fn try_joint_impl_cross_file(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    // Harvest balanced pairs over eligible sections. Sections with
    // pre-existing semantic hunks are skipped (hybrid stays ordinary);
    // sections with unbalanced hunks cannot host a claimed pair but do
    // not veto other files (they pass through as leftovers).
    let mut pairs: Vec<GPair> = Vec::new();
    let mut balanced_secs: HashSet<usize> = HashSet::new();
    let mut sec_path: HashMap<usize, String> = HashMap::new();
    for (i, sec) in sections.iter().enumerate() {
        let header = sec.lines().next().unwrap_or("");
        if !header.starts_with("diff --git ") || sec.contains('\r') {
            continue;
        }
        if super::is_rename_header(header) {
            continue;
        }
        let rest = header.strip_prefix("diff --git ")?;
        let path = crate::b_side_path(rest)?;
        if !path.ends_with(".rs") {
            continue;
        }
        let (_prelude, semantic, native_view) = super::split_semantic_native(sec);
        if !semantic.is_empty() {
            continue;
        }
        let Some(native_sec) = native_view else {
            continue;
        };
        let hunks = super::split_native_hunks(&native_sec);
        if hunks.iter().all(|h| h.balanced) {
            balanced_secs.insert(i);
        }
        sec_path.insert(i, path.clone());
        for h in hunks.iter() {
            if !h.balanced {
                continue;
            }
            if h.minuses.len() != h.sites.len() || h.minuses.len() != h.pluses.len() {
                used_rules.truncate(checkpoint);
                return None;
            }
            for ((m, p), s) in h.minuses.iter().zip(h.pluses.iter()).zip(h.sites.iter()) {
                pairs.push(GPair {
                    sec: i,
                    path: path.clone(),
                    minus: m.clone(),
                    plus: p.clone(),
                    site: *s,
                });
            }
        }
    }
    if pairs.len() < 3 {
        return None;
    }
    // Single-section-pure shapes stay with the single-file arm.
    {
        let mut secs: HashSet<usize> = HashSet::new();
        for p in &pairs {
            secs.insert(p.sec);
        }
        if secs.len() <= 1 {
            return None;
        }
    }
    // Global role assignment: exactly one trait def, one self-ty def,
    // one composed impl double-delta; everything else must be an
    // extra-claimable follow-use.
    let mut trait_leg: Option<(String, String, String, usize, String)> = None;
    let mut ty_leg: Option<(String, String, String, String, usize, String)> = None;
    let mut impl_leg: Option<(String, String, String, String, usize, String)> = None;
    let mut extras: Vec<usize> = Vec::new();
    for (idx, p) in pairs.iter().enumerate() {
        if let Some((prefix, kind, old, new)) = split_def_pair(&p.minus, &p.plus) {
            if kind == "trait" {
                if trait_leg.is_some() {
                    used_rules.truncate(checkpoint);
                    return None;
                }
                trait_leg = Some((prefix, old, new, p.sec, p.path.clone()));
                continue;
            }
            if matches!(kind.as_str(), "struct" | "enum" | "union" | "type") {
                if ty_leg.is_some() {
                    used_rules.truncate(checkpoint);
                    return None;
                }
                ty_leg = Some((prefix, kind, old, new, p.sec, p.path.clone()));
                continue;
            }
            // Any other def-shaped pair is a third rename, never a use.
            // It may live in an uninvolved file as a leftover only when
            // it is not extra-claimable; a def-shaped delta naming a
            // renamed ident vetoes outright (mirrors the single-file
            // third-definition law) — decided below after legs resolve.
            extras.push(idx);
            continue;
        }
        if parse_impl_double_delta(&p.minus, &p.plus).is_some() {
            if impl_leg.is_some() {
                // A second double-delta is a second joint, never an extra.
                used_rules.truncate(checkpoint);
                return None;
            }
            let (ot, nt, oy, ny) = parse_impl_double_delta(&p.minus, &p.plus).expect("just parsed");
            impl_leg = Some((ot, nt, oy, ny, p.sec, p.path.clone()));
            continue;
        }
        extras.push(idx);
    }
    let Some((tr_prefix, ot, nt, _tr_sec, _tr_path)) = trait_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let Some((ty_prefix, ty_kind, oy, ny, _ty_sec, _ty_path)) = ty_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let Some((iot, int_, ioy, iny, _impl_sec, _impl_path)) = impl_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    if iot != ot || int_ != nt || ioy != oy || iny != ny {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Every non-triple pair in an involved file must be claimable; a
    // def-shaped third definition naming a renamed ident vetoes even
    // from uninvolved files (no lying follow). Unclaimable pairs in
    // uninvolved files stay as ordinary leftovers. Resolve the exact
    // triple pair indices for section membership.
    let mut involved_secs: HashSet<usize> = HashSet::new();
    let mut triple_idx: Vec<usize> = Vec::new();
    for (idx, p) in pairs.iter().enumerate() {
        if let Some((prefix, kind, old, new)) = split_def_pair(&p.minus, &p.plus) {
            let is_trait_leg = kind == "trait" && prefix == tr_prefix && old == ot && new == nt;
            let is_ty_leg = kind == ty_kind && prefix == ty_prefix && old == oy && new == ny;
            if is_trait_leg || is_ty_leg {
                triple_idx.push(idx);
            }
        } else if let Some((a, b, c, d)) = parse_impl_double_delta(&p.minus, &p.plus) {
            if a == ot && b == nt && c == oy && d == ny {
                triple_idx.push(idx);
            }
        }
    }
    if triple_idx.len() != 3 {
        used_rules.truncate(checkpoint);
        return None;
    }
    for idx in &triple_idx {
        involved_secs.insert(pairs[*idx].sec);
    }
    // Involved sections must be balanced-pure (no unbalanced hunks).
    for sec in &involved_secs {
        if !balanced_secs.contains(sec) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    // Classify extras: involved files must all claim; uninvolved files
    // claim when claimable, else stay leftovers.
    let mut claimed_idx: Vec<usize> = triple_idx.clone();
    let mut leftover_secs: HashSet<usize> = HashSet::new();
    for idx in extras {
        let p = &pairs[idx];
        // A same-kind definition delta naming a renamed ident is a
        // third definition delta, never a follow-use — veto outright.
        if let (Some((_, ok, oi, _)), Some((_, nk, ni, _))) = (
            split_item(p.minus.trim_end()),
            split_item(p.plus.trim_end()),
        ) {
            if ok == nk && (oi == ot || oi == oy || ni == nt || ni == ny) {
                used_rules.truncate(checkpoint);
                return None;
            }
        }
        if involved_secs.contains(&p.sec) {
            if !extra_claimable(&p.minus, &p.plus, &ot, &nt, &oy, &ny) {
                used_rules.truncate(checkpoint);
                return None;
            }
            claimed_idx.push(idx);
        } else if extra_claimable(&p.minus, &p.plus, &ot, &nt, &oy, &ny) {
            claimed_idx.push(idx);
        } else {
            leftover_secs.insert(p.sec);
        }
    }
    // Twin-definition veto workspace-wide (unique-definition rule).
    let probes: Vec<Pair> = claimed_idx
        .iter()
        .map(|idx| Pair {
            sec: pairs[*idx].sec,
            path: pairs[*idx].path.clone(),
            minus: String::new(),
            plus: String::new(),
            site: pairs[*idx].site,
        })
        .collect();
    if count_kind_defs(pre, &probes, "trait", &ot) != 1 {
        used_rules.truncate(checkpoint);
        return None;
    }
    if count_kind_defs(pre, &probes, &ty_kind, &oy) != 1 {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Pre-image alignment for every claimed site.
    for idx in &claimed_idx {
        let p = &pairs[*idx];
        let Some(pre_text) = pre.get(&p.path) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let got = pre_lines
            .get(p.site.saturating_sub(1))
            .copied()
            .unwrap_or("");
        if got.trim() != p.minus.trim() {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    // Emission: one section per def file sharing one rule. When both
    // defs share a file this collapses to the single-file two-hunk
    // shape; otherwise the document-first def section carries the
    // follow and the sibling shares the rule.
    let rule = unique_rule(used_rules, &format!("rename_{ot}_to_{nt}"));
    // Re-derive def section order in document order (lowest sec first).
    let mut order: Vec<(usize, String, String)> = Vec::new();
    // Find the exact triple entries again for emission text.
    let mut tr_pair: Option<&GPair> = None;
    let mut ty_pair: Option<&GPair> = None;
    for idx in &triple_idx {
        let p = &pairs[*idx];
        if let Some((prefix, kind, _, _)) = split_def_pair(&p.minus, &p.plus) {
            if kind == "trait" {
                tr_pair = Some(p);
            } else {
                let _ = prefix;
                ty_pair = Some(p);
            }
        }
    }
    let (Some(trp), Some(typ)) = (tr_pair, ty_pair) else {
        used_rules.truncate(checkpoint);
        return None;
    };
    if trp.path == typ.path {
        order.push((trp.sec, trp.path.clone(), "both".to_string()));
    } else if trp.sec < typ.sec {
        order.push((trp.sec, trp.path.clone(), "trait".to_string()));
        order.push((typ.sec, typ.path.clone(), "ty".to_string()));
    } else {
        order.push((typ.sec, typ.path.clone(), "ty".to_string()));
        order.push((trp.sec, trp.path.clone(), "trait".to_string()));
    }
    let mut sections_out: HashMap<String, String> = HashMap::new();
    for (pos, (sec_no, path, which)) in order.iter().enumerate() {
        let mut sec = crate::git_header(path);
        sec.push('\n');
        if *which == "both" {
            sec.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            sec.push_str("# sweep\n");
            sec.push_str("# follow=uses\n");
            sec.push_str(&format!(
                "- {tr_prefix}trait {ot}\n+ {tr_prefix}trait {nt}\n"
            ));
            sec.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            sec.push_str("# sweep\n");
            sec.push_str("# follow=uses\n");
            sec.push_str(&format!(
                "- {ty_prefix}{ty_kind} {oy}\n+ {ty_prefix}{ty_kind} {ny}\n"
            ));
        } else if *which == "trait" {
            sec.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            sec.push_str("# sweep\n");
            // Both def hunks carry the follow: the composed `impl`
            // header needs both halves rewritten (single-file shape
            // has both follows; splitting files must not drop one).
            sec.push_str("# follow=uses\n");
            sec.push_str(&format!(
                "- {tr_prefix}trait {ot}\n+ {tr_prefix}trait {nt}\n"
            ));
        } else {
            sec.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            sec.push_str("# sweep\n");
            sec.push_str("# follow=uses\n");
            sec.push_str(&format!(
                "- {ty_prefix}{ty_kind} {oy}\n+ {ty_prefix}{ty_kind} {ny}\n"
            ));
        }
        let _ = sec_no;
        let _ = pos;
        sections_out.insert(path.clone(), sec);
    }
    // The engine follow is workspace-wide, so section order among defs
    // does not matter; document order is kept for readability.
    let joined: String = {
        let mut v: Vec<(&usize, &String)> = order.iter().map(|(s, p, _)| (s, p)).collect();
        v.sort_by_key(|(s, _)| **s);
        v.iter()
            .map(|(_, p)| sections_out[*p].as_str())
            .collect::<String>()
    };
    // D13 workspace probe over the claimed pairs.
    let probe_claims: Vec<(String, usize, String)> = claimed_idx
        .iter()
        .map(|idx| {
            (
                pairs[*idx].path.clone(),
                pairs[*idx].site,
                pairs[*idx].plus.clone(),
            )
        })
        .collect();
    if !probe_joint_workspace(pre, &joined, &probe_claims) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Strict T375 over the summed normalized span of claimed sections.
    let mut claimed_secs: HashSet<usize> = HashSet::new();
    for idx in &claimed_idx {
        claimed_secs.insert(pairs[*idx].sec);
    }
    let mut ordinary: usize = 0;
    for sec_no in &claimed_secs {
        ordinary += super::guard::normalized_section_len(&sections[*sec_no]);
    }
    if !super::guard::within_strict(joined.len(), ordinary) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Rebuild: def files get their emission sections (pure by
    // construction — involved files hold only claimed pairs);
    // use-only claimed files collapse when pure, else keep leftovers;
    // untouched sections pass through.
    let claimed_set: HashSet<usize> = claimed_idx.iter().copied().collect();
    // Map sec -> all pair idx in that sec.
    let mut sec_pairs: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, p) in pairs.iter().enumerate() {
        sec_pairs.entry(p.sec).or_default().push(idx);
    }
    let mut def_paths: HashSet<String> = HashSet::new();
    def_paths.insert(trp.path.clone());
    def_paths.insert(typ.path.clone());
    let mut out: Vec<String> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        let Some(path) = sec_path.get(&i) else {
            out.push(sec.clone());
            continue;
        };
        let all: &[usize] = sec_pairs.get(&i).map(Vec::as_slice).unwrap_or(&[]);
        if all.is_empty() {
            out.push(sec.clone());
            continue;
        }
        let all_claimed = all.iter().all(|idx| claimed_set.contains(idx));
        if def_paths.contains(path) {
            // Def file: emission replaces the section when pure.
            if !all_claimed {
                used_rules.truncate(checkpoint);
                return None;
            }
            // The section index may differ from the triple pair's sec
            // when defs share a file — emit once per path.
            if order.iter().any(|(_, p, _)| p == path) {
                // Emit on first encounter of the path in section order.
                let first = order
                    .iter()
                    .filter(|(_, p, _)| p == path)
                    .map(|(s, _, _)| *s)
                    .min()
                    .unwrap_or(usize::MAX);
                if i == first {
                    out.push(sections_out[path].clone());
                }
            } else {
                out.push(sec.clone());
            }
        } else if all_claimed {
            // Use-only claimed file: the follow owns every line, so the
            // section collapses (mirrors the trait-method cross-file
            // single-hunk shape).
            continue;
        } else {
            // Mixed use file: keep the unclaimed leftover hunks.
            // Slice 1 keeps this honest by declining when a mixed file
            // also holds claimed pairs (no partial emission surgery);
            // WI-12 slice 3 pins the parity (separate-hunk mixed
            // declines exactly like same-hunk mixed: a file keeping an
            // ordinary remainder is never Rust-parsed, so no cross-file
            // follow can reach into it).
            if all.iter().any(|idx| claimed_set.contains(idx)) {
                used_rules.truncate(checkpoint);
                return None;
            }
            out.push(sec.clone());
        }
    }
    // Def-path for provenance: the document-first def file.
    let mut def_order: Vec<(usize, String)> =
        order.iter().map(|(s, p, _)| (*s, p.clone())).collect();
    def_order.sort_by_key(|(s, _)| *s);
    let def_path = def_order
        .first()
        .map(|(_, p)| p.clone())
        .unwrap_or(trp.path.clone());
    let _ = leftover_secs;
    Some((out, def_path, rule))
}

/// D13 workspace probe for the cross-file joint: the joined emission
/// applied to a `pre` clone must show each claimed plus line at its
/// site with every other line of every file byte-identical.
fn probe_joint_workspace(
    pre: &HashMap<String, String>,
    emission: &str,
    claimed: &[(String, usize, String)],
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(emission) else {
        return false;
    };
    let mut files = pre.clone();
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    let mut claims: HashMap<&str, HashMap<usize, &str>> = HashMap::new();
    for (path, site, plus) in claimed {
        claims
            .entry(path.as_str())
            .or_default()
            .insert(*site, plus.as_str());
    }
    for (path, pre_text) in pre {
        let Some(post_text) = files.get(path) else {
            return false;
        };
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let post_lines: Vec<&str> = post_text.lines().collect();
        if post_lines.len() != pre_lines.len() {
            return false;
        }
        let empty: HashMap<usize, &str> = HashMap::new();
        let claim = claims.get(path.as_str()).unwrap_or(&empty);
        for (lineno, (a, b)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
            if let Some(want) = claim.get(&(lineno + 1)) {
                if b.trim() != want.trim() {
                    return false;
                }
            } else if a != b {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const CROSS_A_PRE: &str = "trait Printable {}\n";
    const CROSS_B_PRE: &str = "struct Old;\nimpl Printable for Old {\n}\n";

    fn cross_sections() -> Vec<String> {
        vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-trait Printable {}\n+trait Labeled {}\n",
            ),
            section(
                "b.rs",
                "@@ -1,2 +1,2 @@\n-struct Old;\n-impl Printable for Old {\n+struct New;\n+impl Labeled for New {\n",
            ),
        ]
    }

    fn cross_pre() -> HashMap<String, String> {
        pre_map(&[("a.rs", CROSS_A_PRE), ("b.rs", CROSS_B_PRE)])
    }

    fn fire_cross(sections: &[String], pre: &HashMap<String, String>) -> Option<Vec<String>> {
        let mut used = Vec::new();
        super::super::item_joint_impl::try_joint_impl_rename(sections, pre, &mut used)
            .map(|(out, _, _)| out)
    }

    #[test]
    fn joint_impl_cross_file_fires_one_shared_rule() {
        let sections = cross_sections();
        let pre = cross_pre();
        let out = fire_cross(&sections, &pre).expect("cross-file triple must fire");
        // One emission section per def file sharing one rule, both
        // hunks carrying the follow (the composed header needs both
        // halves rewritten).
        assert_eq!(out.len(), 2, "got: {out:?}");
        let joint = out.join("");
        assert_eq!(joint.matches("rule=r1").count(), 2, "got: {joint}");
        assert_eq!(
            joint.matches("# follow=uses").count(),
            2,
            "both def hunks follow, got: {joint}"
        );
        assert!(joint.contains("- trait Printable"), "got: {joint}");
        assert!(joint.contains("+ trait Labeled"), "got: {joint}");
        assert!(joint.contains("- struct Old"), "got: {joint}");
        assert!(joint.contains("+ struct New"), "got: {joint}");
        assert!(
            !joint.contains("impl Printable"),
            "headers rewritten, never stated, got: {joint}"
        );
        let parsed = rsmpl_patch::parse_patch(&joint).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["a.rs"], "trait Labeled {}\n",
            "got: {}",
            files["a.rs"]
        );
        assert_eq!(
            files["b.rs"], "struct New;\nimpl Labeled for New {\n}\n",
            "got: {}",
            files["b.rs"]
        );
    }

    #[test]
    fn joint_impl_cross_file_pipeline_compresses() {
        let sections = cross_sections();
        let pre = cross_pre();
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- trait Printable"), "got:\n{out}");
        assert!(out.contains("- struct Old"), "got:\n{out}");
        // No ordinary def hunks survive alongside the intent.
        assert!(!out.contains("-struct Old;"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["a.rs"].contains("trait Labeled"),
            "got: {}",
            files["a.rs"]
        );
    }

    #[test]
    fn joint_impl_cross_file_single_section_stays_with_single_file_arm() {
        // One section holding the whole triple: the cross-file arm
        // declines (single-section-pure) and the single-file arm owns
        // the emission byte-identically (two hunks, one section).
        let pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\n",
        )]);
        let native = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n",
        );
        let mut used = Vec::new();
        let (out, _, _) = super::super::item_joint_impl::try_joint_impl_rename(
            std::slice::from_ref(&native),
            &pre,
            &mut used,
        )
        .expect("single-file triple must fire");
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert_eq!(out[0].matches("rule=r1").count(), 2, "got: {}", out[0]);
    }

    #[test]
    fn joint_impl_cross_file_veto_matrix() {
        let pre = cross_pre();
        // Mismatched composition (impl names a third ident) declines.
        let mismatch = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-trait Printable {}\n+trait Labeled {}\n",
            ),
            section(
                "b.rs",
                "@@ -1,2 +1,2 @@\n-struct Old;\n-impl Printable for Old {\n+struct New;\n+impl Labeled for Other {\n",
            ),
        ];
        assert!(fire_cross(&mismatch, &pre).is_none());
        // Third definition delta (extra fn rename) vetoes the joint.
        let third = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-trait Printable {}\n+trait Labeled {}\n",
            ),
            section(
                "b.rs",
                "@@ -1,3 +1,3 @@\n-struct Old;\n-impl Printable for Old {\n-fn helper() {}\n+struct New;\n+impl Labeled for New {\n+fn ren_helper() {}\n",
            ),
        ];
        assert!(fire_cross(&third, &pre).is_none());
        // Twin `struct Old` in an involved file vetoes (unique-definition).
        let twin_pre = pre_map(&[
            ("a.rs", CROSS_A_PRE),
            (
                "b.rs",
                "struct Old;\nimpl Printable for Old {\n}\nmod m {\n    pub struct Old;\n}\n",
            ),
        ]);
        assert!(fire_cross(&cross_sections(), &twin_pre).is_none());
    }

    #[test]
    fn joint_impl_cross_file_third_file_off_diff_use_vetoes_joint() {
        // `c.rs` is touched (comment) but leaves `x: Old` unrewritten:
        // the struct follow would rewrite it, so the D13 probe fails
        // and no joint emits (per-section residue stays ordinary or
        // honest-partial downstream, never a false joint).
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-trait Printable {}\n+trait Labeled {}\n",
            ),
            section(
                "b.rs",
                "@@ -1,2 +1,2 @@\n-struct Old;\n-impl Printable for Old {\n+struct New;\n+impl Labeled for New {\n",
            ),
            section("c.rs", "@@ -1,1 +1,1 @@\n-// touch\n+// touch2\n"),
        ];
        let pre = pre_map(&[
            ("a.rs", CROSS_A_PRE),
            ("b.rs", CROSS_B_PRE),
            ("c.rs", "// touch\nfn g(x: Old) {\n}\n"),
        ]);
        let mut used = Vec::new();
        let out = super::super::item_joint_impl::try_joint_impl_rename(&sections, &pre, &mut used)
            .map(|(o, _, _)| o.join(""));
        // No shared-rule two-file joint: either decline or a
        // joint-free residue (the joint itself must not fire).
        if let Some(joint) = out {
            assert!(
                joint.matches("rule=r1").count() < 2 || !joint.contains("- struct Old"),
                "off-diff use must veto the joint, got:\n{joint}"
            );
        }
    }

    #[test]
    fn joint_impl_cross_file_separate_hunk_mixed_use_file_declines() {
        // `c.rs` holds a claimable follow-use in hunk 1 and an
        // unrelated rename in hunk 2: the joint still declines at
        // section level (parity with same-hunk mixed). Per-hunk
        // leftover surgery is unsound by engine construction: a file
        // keeping an ordinary remainder is never Rust-parsed (pass-0
        // T137: ordinary-only sections skip the parse), so no
        // cross-file follow can reach into it — probed: applying the
        // two def-only follow hunks rewrites `fn f(x: Old)` in a
        // section-free `c.rs`, but leaves the same line untouched
        // once `c.rs` gains its own ordinary section.
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-trait Printable {}\n+trait Labeled {}\n",
            ),
            section(
                "b.rs",
                "@@ -1,2 +1,2 @@\n-struct Old;\n-impl Printable for Old {\n+struct New;\n+impl Labeled for New {\n",
            ),
            section(
                "c.rs",
                "@@ -2,1 +2,1 @@\n-fn f(x: Old) {}\n+fn f(x: New) {}\n@@ -4,1 +4,1 @@\n-fn helper() {}\n+fn ren_helper() {}\n",
            ),
        ];
        let pre = pre_map(&[
            ("a.rs", CROSS_A_PRE),
            ("b.rs", CROSS_B_PRE),
            (
                "c.rs",
                "use other::thing;\nfn f(x: Old) {}\nfn g() {}\nfn helper() {}\n",
            ),
        ]);
        let out = fire_cross(&sections, &pre).map(|o| o.join(""));
        // No shared-rule joint: either decline or a joint-free residue.
        if let Some(joint) = out {
            assert!(
                joint.matches("rule=r1").count() < 2 || !joint.contains("- struct Old"),
                "separate-hunk mixed must decline the joint, got:\n{joint}"
            );
        }
    }

    #[test]
    fn joint_impl_cross_file_same_hunk_mixed_use_file_declines() {
        // One hunk holding both a claimable follow-use and an
        // unrelated rename: hunk atomicity declines the joint.
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-trait Printable {}\n+trait Labeled {}\n",
            ),
            section(
                "b.rs",
                "@@ -1,2 +1,2 @@\n-struct Old;\n-impl Printable for Old {\n+struct New;\n+impl Labeled for New {\n",
            ),
            section(
                "c.rs",
                "@@ -2,2 +2,2 @@\n-fn f(x: Old) {}\n-fn helper() {}\n+fn f(x: New) {}\n+fn ren_helper() {}\n",
            ),
        ];
        let pre = pre_map(&[
            ("a.rs", CROSS_A_PRE),
            ("b.rs", CROSS_B_PRE),
            (
                "c.rs",
                "use other::thing;\nfn f(x: Old) {}\nfn helper() {}\n",
            ),
        ]);
        let out = fire_cross(&sections, &pre).map(|o| o.join(""));
        if let Some(joint) = out {
            assert!(
                joint.matches("rule=r1").count() < 2 || !joint.contains("- struct Old"),
                "same-hunk mixed must decline the joint, got:\n{joint}"
            );
        }
    }

    #[test]
    fn joint_impl_cross_file_def_file_mixed_stays_vetoed() {
        // An unclaimable pair in a def file vetoes even when a use-only
        // file is clean.
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,2 +1,2 @@\n-trait Printable {}\n-fn helper() {}\n+trait Labeled {}\n+fn ren_helper() {}\n",
            ),
            section(
                "b.rs",
                "@@ -1,2 +1,2 @@\n-struct Old;\n-impl Printable for Old {\n+struct New;\n+impl Labeled for New {\n",
            ),
        ];
        let pre = cross_pre();
        assert!(fire_cross(&sections, &pre).is_none());
    }
}
