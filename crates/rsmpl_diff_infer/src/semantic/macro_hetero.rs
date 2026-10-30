//! Cross-site hetero macro-interior grouping (WI-9b2): N ≥ 2
//! whole-line same-name interior pairs that differ ACROSS sites in
//! stable args but share one literal delta — e.g. `log!(alpha, 1)` →
//! `log!(alpha, 2)` beside `log!(beta, 1)` → `log!(beta, 2)` compress
//! to `- log!(E0, 1)` / `+ log!(E0, 2)`. The identical-pair classes in
//! `macros.rs` veto these as singletons; this secondary pass groups
//! leftovers by (path, delimiter, arity) and holes the varying stable
//! positions. Same WI-9b1 rules: strings never hole, hole names stay
//! fresh past every site's idents (SPEC 15.7), real-pair totality
//! (`template_pair_holds`) counts exactly N, per-group byte sanity
//! keeps a losing candidate from flipping the section-global T375.
//! Vetoed shapes (stay ordinary): varying deltas across sites, mixed
//! stable/delta positions, arity mismatch, varying string stables.
//! Split from `macros.rs` per the D5 hygiene gate.

use super::macro_parts::{
    is_string_arg, macro_delim_open, macro_path, macro_raw_args, parse_macro_expr, scan_arg_idents,
    split_top_spans, trim_range,
};

/// Cross-site grouping key for one whole-line interior pair: the
/// shared macro path, the open delimiter, and the top-level arity.
/// `None` for every shape the interior sweep declines (renames —
/// path delta, mixed delimiters, leading-`::`/generic paths,
/// unbalanced spans, identical bodies): only same-path same-delim
/// body deltas with a clean arity key group here.
pub(crate) fn hetero_group_key(minus: &str, plus: &str) -> Option<(String, char, usize)> {
    let o_mac = parse_macro_expr(minus)?;
    let n_mac = parse_macro_expr(plus)?;
    let o_path = macro_path(&o_mac)?;
    let n_path = macro_path(&n_mac)?;
    if o_path != n_path {
        return None;
    }
    let (Some(o_open), Some(n_open)) = (macro_delim_open(&o_mac), macro_delim_open(&n_mac)) else {
        return None;
    };
    if o_open != n_open {
        return None;
    }
    if o_mac.mac.tokens.to_string() == n_mac.mac.tokens.to_string() {
        return None;
    }
    let m_args = macro_raw_args(minus)?;
    let p_args = macro_raw_args(plus)?;
    let m_spans = split_top_spans(&m_args)?;
    let p_spans = split_top_spans(&p_args)?;
    if m_spans.len() != p_spans.len() {
        return None;
    }
    Some((o_path, o_open, m_spans.len()))
}

/// One parsed member pair: the shared path/open plus both sides' raw
/// inner-args text with top-level group spans.
struct Member {
    m_args: String,
    p_args: String,
    m_spans: Vec<(usize, usize)>,
    p_spans: Vec<(usize, usize)>,
}

/// Cross-site hetero template over N ≥ 2 `(minus, plus)` line pairs:
/// every pair keys alike (same path/delim/arity — checked by the
/// caller), and every top-level arg position is either stable in ALL
/// pairs (token-equal everywhere → literal; varying → one shared
/// `E{i}` hole, strings veto) or a delta in ALL pairs with the same
/// trimmed `(minus, plus)` everywhere (→ literal delta; varying
/// deltas or mixed stable/delta positions veto). Emission rebuilds
/// the first pair's raw skeleton with holes substituted, separators
/// verbatim (the engine matches macro interiors token-wise — spacing
/// is irrelevant, probed). Returns `None` when nothing holes (then
/// the pairs are identical and belong to an identical class, not
/// here), on any veto, or when a hole would not shorten every
/// site's text (per-position byte-win, WI-9b1 `> 2` rule lifted to
/// the shortest site).
pub(crate) fn hetero_group_template(member_lines: &[(&str, &str)]) -> Option<(String, String)> {
    if member_lines.len() < 2 {
        return None;
    }
    let mut members: Vec<Member> = Vec::with_capacity(member_lines.len());
    let mut path: Option<String> = None;
    let mut open: Option<char> = None;
    let mut arity: Option<usize> = None;
    for (m_line, p_line) in member_lines {
        let o_mac = parse_macro_expr(m_line)?;
        let n_mac = parse_macro_expr(p_line)?;
        let o_path = macro_path(&o_mac)?;
        if macro_path(&n_mac)? != o_path {
            return None;
        }
        let (Some(o_open), Some(n_open)) = (macro_delim_open(&o_mac), macro_delim_open(&n_mac))
        else {
            return None;
        };
        if o_open != n_open {
            return None;
        }
        if o_mac.mac.tokens.to_string() == n_mac.mac.tokens.to_string() {
            return None;
        }
        let m_args = macro_raw_args(m_line)?;
        let p_args = macro_raw_args(p_line)?;
        let m_spans = split_top_spans(&m_args)?;
        let p_spans = split_top_spans(&p_args)?;
        if m_spans.len() != p_spans.len() {
            return None;
        }
        if path.as_ref().is_some_and(|x| *x != o_path)
            || open.is_some_and(|x| x != o_open)
            || arity.is_some_and(|x| x != m_spans.len())
        {
            return None;
        }
        path = Some(o_path);
        open = Some(o_open);
        arity = Some(m_spans.len());
        members.push(Member {
            m_args,
            p_args,
            m_spans,
            p_spans,
        });
    }
    let (path, open, arity) = (path?, open?, arity?);
    // Occupied hole names: every ident in the path and in ANY arg
    // slice across all sites (WI-9b1 precedent — a hole name never
    // coincides with a literal ident, so SPEC 15.7 equality can never
    // fuse a hole to a literal, including positions kept literal).
    let mut occupied: Vec<String> = scan_arg_idents(&path);
    for m in &members {
        for ((ms, me), (ps, pe)) in m.m_spans.iter().zip(m.p_spans.iter()) {
            occupied.extend(scan_arg_idents(m.m_args[*ms..*me].trim()));
            occupied.extend(scan_arg_idents(m.p_args[*ps..*pe].trim()));
        }
    }
    let mut next_hole = 0u32;
    let mut fresh = |used: &mut Vec<String>| loop {
        let name = if next_hole == 0 {
            "E0".to_string()
        } else {
            format!("E{next_hole}")
        };
        next_hole += 1;
        if !used
            .iter()
            .any(|u| u == &name || u.trim_start_matches('$') == name)
        {
            used.push(name.clone());
            return name;
        }
    };
    // Per-position plan: literal text (first pair's raw slice) or a
    // shared hole. A position stable in some pairs but a delta in
    // others, or a delta whose trimmed texts differ across sites, is
    // not this slice (varying deltas stay ordinary — future
    // anti-unification work, noted in the module docs).
    enum Plan {
        Literal,
        Hole(String),
    }
    let mut plans: Vec<Plan> = Vec::with_capacity(arity);
    let mut holed_any = false;
    let mut delta_any = false;
    for j in 0..arity {
        let groups: Vec<(&str, &str)> = members
            .iter()
            .map(|m| {
                let (ms, me) = m.m_spans[j];
                let (ps, pe) = m.p_spans[j];
                (m.m_args[ms..me].trim(), m.p_args[ps..pe].trim())
            })
            .collect();
        let stable_all = groups.iter().all(|(a, b)| a == b);
        let delta_all = groups.iter().all(|(a, b)| a != b);
        if stable_all {
            let first = groups[0].0;
            if groups.iter().all(|(a, _)| *a == first) {
                plans.push(Plan::Literal);
            } else {
                if groups.iter().any(|(a, _)| is_string_arg(a)) {
                    return None;
                }
                let hole = fresh(&mut occupied);
                if groups.iter().any(|(a, _)| a.len() <= hole.len()) {
                    return None;
                }
                plans.push(Plan::Hole(hole));
                holed_any = true;
            }
        } else if delta_all {
            let (f_m, f_p) = groups[0];
            if !groups.iter().all(|(a, b)| *a == f_m && *b == f_p) {
                return None;
            }
            plans.push(Plan::Literal);
            delta_any = true;
        } else {
            return None;
        }
    }
    if !holed_any || !delta_any {
        return None;
    }
    // Rebuild from the first pair's raw skeleton, preserving
    // separators verbatim; holed positions substitute the trimmed
    // group text with the hole (WI-9b1 `hetero_pair` surgery).
    let first = &members[0];
    let mut m_rebuilt = String::new();
    let mut p_rebuilt = String::new();
    let mut m_cursor = 0;
    let mut p_cursor = 0;
    for (j, plan) in plans.iter().enumerate() {
        let (ms, me) = first.m_spans[j];
        let (ps, pe) = first.p_spans[j];
        m_rebuilt.push_str(&first.m_args[m_cursor..ms]);
        p_rebuilt.push_str(&first.p_args[p_cursor..ps]);
        match plan {
            Plan::Literal => {
                m_rebuilt.push_str(&first.m_args[ms..me]);
                p_rebuilt.push_str(&first.p_args[ps..pe]);
            }
            Plan::Hole(hole) => {
                let (mts, mte) = trim_range(&first.m_args, ms, me);
                let (pts, pte) = trim_range(&first.p_args, ps, pe);
                m_rebuilt.push_str(&first.m_args[ms..mts]);
                m_rebuilt.push_str(hole);
                m_rebuilt.push_str(&first.m_args[mte..me]);
                p_rebuilt.push_str(&first.p_args[ps..pts]);
                p_rebuilt.push_str(hole);
                p_rebuilt.push_str(&first.p_args[pte..pe]);
            }
        }
        m_cursor = me;
        p_cursor = pe;
    }
    m_rebuilt.push_str(&first.m_args[m_cursor..]);
    p_rebuilt.push_str(&first.p_args[p_cursor..]);
    let close = match open {
        '[' => ']',
        '{' => '}',
        _ => ')',
    };
    Some((
        format!("{path}!{open}{m_rebuilt}{close}"),
        format!("{path}!{open}{p_rebuilt}{close}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic;
    use super::{hetero_group_key, hetero_group_template};
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

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    #[test]
    fn xsite_key_groups_same_path_arity() {
        let k1 = hetero_group_key("    log!(alpha_value, 1);", "    log!(alpha_value, 2);");
        let k2 = hetero_group_key("    log!(beta_value, 1);", "    log!(beta_value, 2);");
        assert!(k1.is_some() && k1 == k2, "same key, got: {k1:?} vs {k2:?}");
    }

    #[test]
    fn xsite_key_rejects_rename_and_mixed_delim() {
        assert!(hetero_group_key("    old!(a);", "    new!(a);").is_none());
        assert!(hetero_group_key("    log!(a);", "    log![a];").is_none());
        assert!(hetero_group_key("    log!(a, 1);", "    log!(a, 1);").is_none());
    }

    #[test]
    fn xsite_template_holes_varying_stable() {
        let out = hetero_group_template(&[
            ("    log!(alpha_value, 1);", "    log!(alpha_value, 2);"),
            ("    log!(beta_value, 1);", "    log!(beta_value, 2);"),
        ])
        .expect("must template");
        assert_eq!(out.0, "log!(E0, 1)");
        assert_eq!(out.1, "log!(E0, 2)");
    }

    #[test]
    fn xsite_template_rejects_varying_deltas() {
        // Same path/arity, but the delta itself differs across sites —
        // not this slice (future anti-unification work).
        assert!(hetero_group_template(&[
            ("    log!(alpha_value, 1);", "    log!(alpha_value, 2);"),
            ("    log!(beta_value, 10);", "    log!(beta_value, 20);"),
        ])
        .is_none());
    }

    #[test]
    fn xsite_template_rejects_mixed_stable_delta() {
        // Position 1 is stable in pair one, a delta in pair two.
        assert!(hetero_group_template(&[
            ("    log!(alpha_value, 1);", "    log!(alpha_value, 2);"),
            ("    log!(beta_value, 1);", "    log!(gamma_value, 1);"),
        ])
        .is_none());
    }

    #[test]
    fn xsite_template_rejects_string_stable() {
        // Varying stable strings never hole (WI-9 opaque rule).
        assert!(hetero_group_template(&[
            ("    log!(\"aaa\", 1);", "    log!(\"aaa\", 2);"),
            ("    log!(\"bbb\", 1);", "    log!(\"bbb\", 2);"),
        ])
        .is_none());
    }

    #[test]
    fn xsite_template_rejects_arity_mismatch() {
        assert!(hetero_group_template(&[
            ("    log!(alpha_value, 1);", "    log!(alpha_value, 2);"),
            (
                "    log!(beta_value, 1, extra);",
                "    log!(beta_value, 2, extra);"
            ),
        ])
        .is_none());
    }

    #[test]
    fn xsite_template_requires_two_pairs() {
        assert!(hetero_group_template(&[(
            "    log!(alpha_value, 1);",
            "    log!(alpha_value, 2);"
        )])
        .is_none());
    }

    #[test]
    fn xsite_fire_holes_cross_site_stable() {
        // End to end: two singleton-class pairs group cross-site; the
        // emitted hetero hunk real-applies over both sites.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    log!(alpha_value, 1);\n-    log!(beta_value, 1);\n+    log!(alpha_value, 2);\n+    log!(beta_value, 2);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    log!(alpha_value, 1);\n    log!(beta_value, 1);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("xsite must fire");
        validated_live(&out, &pre);
        assert!(out.contains("- log!(E0, 1)\n"), "got: {out}");
        assert!(out.contains("+ log!(E0, 2)\n"), "got: {out}");
    }

    #[test]
    fn xsite_varying_delta_stays_ordinary() {
        // Varying deltas veto the cross-site group; each singleton
        // class also vetoes (N≥2) — the section stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    log!(alpha_value, 1);\n-    log!(beta_value, 10);\n+    log!(alpha_value, 2);\n+    log!(beta_value, 20);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn xsite_untouched_twin_vetoes() {
        // Whole-file conservative totality: an untouched
        // `log!(other_value, 1)` twin binds the `E0` hole too, so the
        // real-pair probe counts 3 ≠ 2 and the section stays ordinary
        // (same posture as the identical-hetero twin pin).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    log!(alpha_value, 1);\n-    log!(beta_value, 1);\n+    log!(alpha_value, 2);\n+    log!(beta_value, 2);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    log!(alpha_value, 1);\n    log!(beta_value, 1);\n    log!(other_value, 1);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }
}
