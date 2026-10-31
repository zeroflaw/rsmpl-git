//! WI-17b: `macro_rules!` rule-body interior sweep.
//!
//! Arm lines (`($x:expr) => { inner!(1); };`) parse as neither `Stmt`
//! nor `Expr`, so the whole-line (`parse_macro_interior_pair`) and
//! sub-line (`parse_nested_macro_pair`) interior sweeps never key them.
//! This detector extracts the inner-macro delta textually (one
//! same-path/same-delimiter inner invocation differs, the rest of the
//! line byte-identical) and groups identical rendered inner pairs at
//! N >= 2 with the standard soundness contract: real-pair engine-count
//! totality (`template_pair_holds`, which proves via the WI-17b engine
//! half that mints rule-body candidates), hunk atomicity, and the
//! section-global T375 byte guard.
//!
//! Concrete-only (no hetero holes — WI-17 hetero precedent stays
//! whole-line); no stranded claims; depth-2 vetoes fall out naturally
//! (a line like `outer!(inner!(1))` yields two textual deltas, so the
//! pair returns `None` and stays ordinary).

use std::collections::HashSet;

use super::macro_parts::{member_sites, scan_line_macros};
use super::unique_rule;

/// One rule-body inner-macro delta: both lines hold the same count
/// (>= 1) of textual macro invocations, every position shares path +
/// delimiter, exactly one position differs in full text, each side's
/// delta text occurs exactly once in its line, and the plus line equals
/// the minus line with exactly that one substring replaced. Returns
/// the rendered `(minus_macro, plus_macro)` inner pair.
pub(crate) fn parse_rule_body_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    // Whole-line macro pairs belong to the interior sweep one level
    // down (it declined whole sections containing these, but the shape
    // is still its domain — re-grouping here would double-claim).
    if super::macro_parts::parse_macro_interior_pair(old_line, new_line).is_some() {
        return None;
    }
    let old_macros = scan_line_macros(old_line);
    let new_macros = scan_line_macros(new_line);
    if old_macros.len() != new_macros.len() || old_macros.is_empty() {
        return None;
    }
    let mut delta: Option<usize> = None;
    for (i, ((op, oo, ot), (np, no, nt))) in old_macros.iter().zip(new_macros.iter()).enumerate() {
        if op != np || oo != no {
            return None;
        }
        if ot == nt {
            continue;
        }
        if delta.is_some() {
            return None;
        }
        delta = Some(i);
    }
    let i = delta?;
    let (_, _, minus) = &old_macros[i];
    let (_, _, plus) = &new_macros[i];
    if minus == plus {
        return None;
    }
    if old_line.match_indices(minus.as_str()).count() != 1
        || new_line.match_indices(plus.as_str()).count() != 1
    {
        return None;
    }
    if *new_line != old_line.replacen(minus.as_str(), plus.as_str(), 1) {
        return None;
    }
    Some((minus.clone(), plus.clone()))
}

struct RuleBodyGroup {
    minus: String,
    plus: String,
    members: Vec<usize>,
    sites: Vec<usize>,
}

/// Whole-section rule-body interior sweep over balanced pairs.
/// Returns the rewritten section, or `None` when no group fires.
pub(crate) fn try_macro_rule_body_section(
    section: &str,
    path: &str,
    used_rules: &mut Vec<String>,
    file_old: Option<&str>,
    fired: &mut Vec<(String, &'static str)>,
) -> Option<String> {
    let (prelude, hunks) = super::peel::split_section_hunks(section);
    if hunks.is_empty() {
        return None;
    }
    let hunk_changed: Vec<Vec<usize>> = hunks
        .iter()
        .map(|h| crate::totality::changed_old_lines(h))
        .collect();
    let mut pairs: Vec<(usize, String, String)> = Vec::new();
    let mut pair_old: Vec<usize> = Vec::new();
    let mut hunk_balanced = vec![false; hunks.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        if hunk.lines().next().is_some_and(|h| h.contains("rule=")) {
            continue;
        }
        let mut minuses = Vec::new();
        let mut pluses = Vec::new();
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            }
        }
        if minuses.len() != pluses.len() || minuses.is_empty() {
            continue;
        }
        hunk_balanced[i] = true;
        for (k, (m, p)) in minuses.into_iter().zip(pluses).enumerate() {
            if m.trim() == p.trim() {
                continue;
            }
            let old_line = hunk_changed[i].get(k).copied().unwrap_or(0);
            pairs.push((i, m, p));
            pair_old.push(old_line);
        }
    }
    if pairs.is_empty() {
        return None;
    }
    let mut classes: Vec<((String, String), Vec<usize>)> = Vec::new();
    for (idx, (.., m, p)) in pairs.iter().enumerate() {
        let Some(key) = parse_rule_body_pair(m, p) else {
            continue;
        };
        match classes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(idx),
            None => classes.push((key, vec![idx])),
        }
    }
    let checkpoint = used_rules.len();
    let mut passing: Vec<RuleBodyGroup> = Vec::new();
    for ((minus, plus), members) in &classes {
        if members.len() < 2 {
            continue;
        }
        if !crate::totality::template_pair_holds(path, file_old, minus, plus, members.len()) {
            continue;
        }
        passing.push(RuleBodyGroup {
            minus: minus.clone(),
            plus: plus.clone(),
            members: members.clone(),
            sites: member_sites(members, &pair_old),
        });
    }
    // Hunk atomicity: a group fires only if every balanced hunk
    // holding a member pair is fully covered.
    loop {
        let covered: HashSet<usize> = passing
            .iter()
            .flat_map(|g| g.members.iter().copied())
            .collect();
        let mut victim: Option<usize> = None;
        'scan: for (gi, g) in passing.iter().enumerate() {
            for &idx in &g.members {
                let hj = pairs[idx].0;
                for (jj, (hj2, _, _)) in pairs.iter().enumerate() {
                    if *hj2 != hj {
                        continue;
                    }
                    if !covered.contains(&jj) {
                        victim = Some(gi);
                        break 'scan;
                    }
                }
            }
        }
        let Some(gi) = victim else { break };
        passing.remove(gi);
    }
    if passing.is_empty() {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut fired_groups: Vec<(RuleBodyGroup, String)> = Vec::new();
    for g in passing {
        let rule = unique_rule(used_rules, "macro_rule_body");
        fired.push((rule.clone(), "macro_rule_body"));
        fired_groups.push((g, rule));
    }
    let passing = fired_groups;
    let covered: HashSet<usize> = passing
        .iter()
        .flat_map(|(g, _)| g.members.iter().copied())
        .collect();
    let covered_hunk: Vec<bool> = hunk_balanced
        .iter()
        .enumerate()
        .map(|(i, b)| {
            *b && pairs
                .iter()
                .enumerate()
                .filter(|(_, (j, _, _))| *j == i)
                .all(|(jj, _)| covered.contains(&jj))
        })
        .collect();
    let mut spans: Vec<Option<(usize, usize)>> = Vec::new();
    for (g, _) in &passing {
        if g.sites.contains(&0) {
            spans.push(None);
            continue;
        }
        let min = g.sites.iter().min().copied().unwrap_or(0);
        let max = g.sites.iter().max().copied().unwrap_or(0);
        if min >= 1 && max.saturating_sub(min).saturating_add(1) <= 64 {
            spans.push(Some((min, max)));
        } else {
            spans.push(None);
        }
    }
    let mut overlap = vec![false; passing.len()];
    for a in 0..spans.len() {
        for b in (a + 1)..spans.len() {
            if let (Some((a0, a1)), Some((b0, b1))) = (spans[a], spans[b]) {
                if a0 <= b1 && b0 <= a1 {
                    overlap[a] = true;
                    overlap[b] = true;
                }
            }
        }
    }
    let bodies: Vec<String> = passing
        .iter()
        .enumerate()
        .map(|(gi, (g, rule))| {
            if overlap[gi] || spans[gi].is_none() {
                format!(
                    "@@ -1,1 @@ rule={rule}\n# sweep\n- {}\n+ {}\n",
                    g.minus, g.plus
                )
            } else {
                format!(
                    "{}- {}\n+ {}\n",
                    crate::emission_body(rule, &g.sites),
                    g.minus,
                    g.plus
                )
            }
        })
        .collect();
    let covered_len: usize = hunks
        .iter()
        .zip(&covered_hunk)
        .filter(|(_, b)| **b)
        .map(|(h, _)| h.len())
        .sum();
    if bodies.iter().map(|b| b.len()).sum::<usize>() >= covered_len {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut out = prelude;
    let mut emitted = vec![false; passing.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        if covered_hunk[i] {
            for (gi, g) in passing.iter().enumerate() {
                if !emitted[gi]
                    && pairs
                        .iter()
                        .enumerate()
                        .any(|(jj, (j, _, _))| *j == i && g.0.members.contains(&jj))
                {
                    out.push_str(&bodies[gi]);
                    emitted[gi] = true;
                }
            }
            continue;
        }
        out.push_str(hunk);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, unique_rule};
    use super::*;
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

    const PRE: &str = "macro_rules! foo {\n    ($x:expr) => { inner!(1); };\n    ($y:expr) => { inner!(1); };\n}\n";

    fn raw_two_arm() -> String {
        section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n macro_rules! foo {\n-    ($x:expr) => { inner!(1); };\n-    ($y:expr) => { inner!(1); };\n+    ($x:expr) => { inner!(2); };\n+    ($y:expr) => { inner!(2); };\n }\n",
        )
    }

    #[test]
    fn rule_body_pair_extracts_inner_delta() {
        let (m, p) = parse_rule_body_pair(
            "    ($x:expr) => { inner!(1); };",
            "    ($x:expr) => { inner!(2); };",
        )
        .expect("must extract");
        assert_eq!(m, "inner!(1)");
        assert_eq!(p, "inner!(2)");
    }

    #[test]
    fn rule_body_pair_rejects_whole_line() {
        assert!(parse_rule_body_pair("    log!(1);", "    log!(2);").is_none());
    }

    #[test]
    fn rule_body_pair_rejects_depth_two() {
        // Two textual deltas (outer + inner) — depth-2 stays ordinary.
        assert!(parse_rule_body_pair(
            "    ($x:expr) => { outer!(inner!(1)); };",
            "    ($x:expr) => { outer!(inner!(2)); };",
        )
        .is_none());
    }

    #[test]
    fn rule_body_section_fires_two_arms() {
        let raw = raw_two_arm();
        let pre = pre_map(&[("f.rs", PRE)]);
        let out =
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).expect("must fire");
        validated_live(&out, &pre);
        assert!(out.contains("- inner!(1)\n"), "got: {out}");
        assert!(out.contains("+ inner!(2)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(files["f.rs"].contains("inner!(2);"));
        assert!(!files["f.rs"].contains("inner!(1);"));
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn rule_body_twin_vetoes() {
        let raw = raw_two_arm();
        let twin = "macro_rules! foo {\n    ($x:expr) => { inner!(1); };\n    ($y:expr) => { inner!(1); };\n    ($z:expr) => { inner!(1); };\n}\n";
        let pre = pre_map(&[("f.rs", twin)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none()
                || !infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                    .unwrap()
                    .contains("- inner!(1)\n")
        );
    }

    #[test]
    fn rule_body_single_site_stays_ordinary() {
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n macro_rules! foo {\n-    ($x:expr) => { inner!(1); };\n+    ($x:expr) => { inner!(2); };\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "macro_rules! foo {\n    ($x:expr) => { inner!(1); };\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre);
        assert!(out.is_none() || !out.unwrap().contains("- inner!(1)\n"));
    }
}
