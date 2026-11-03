//! WI-15 slice 1 (D5): fn-joint whole-patch pre-pass driver.
//!
//! Split from `fn_joint.rs` under the 2000-line gate (D5): pure move, no
//! behavior change. The driver parses sections, runs `joint_candidate`
//! per single-file group (plus the workspace fallback), probes, gates,
//! and rebuilds; the classifier stays in `fn_joint.rs`. Tests stay
//! inline in their owning files per WI-22.

use std::collections::{HashMap, HashSet};

use super::fn_joint::{joint_candidate, Candidate};
use super::fn_joint_b2::probe_attr_joint;
use super::fn_joint_split::split_fire_gate;
use super::fn_joint_types::{Fire, HunkPairs, Joint, JointOutcome};
use super::fn_joint_ws::{joint_workspace_candidate, Workspace};
use super::item_intent::{probe_reproduces, Pair};
use super::unique_rule;

/// Whole-patch fn-joint pre-pass: returns the rebuilt section list plus
/// the definition path and rule for provenance, and the suppressed
/// `(path, old-name)` set. `None` when no file holds a joint
/// candidate. A suppress-only outcome returns the sections
/// byte-identical with an empty rule — the caller keeps the sections
/// and threads only the suppression set. Reserved rule names are
/// released on decline.
pub(crate) fn try_fn_joint_intent(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<JointOutcome> {
    let checkpoint = used_rules.len();
    // Parse eligible sections (`.rs` non-rename sections only).
    // WI-4e: hunk-level inputs — skip `rule=` hunks, not whole sections.
    let mut parsed: Vec<(usize, String, Vec<HunkPairs>)> = Vec::new();
    // WI-4e: preserved semantic hunks + native lens per claimed section.
    let mut preserved: HashMap<usize, (String, Vec<String>)> = HashMap::new();
    let mut native_lens: HashMap<usize, usize> = HashMap::new();
    // WI-4f-next: verbatim native hunk texts per (sec, hunk-idx) for
    // leftover preservation + claimed-bytes T375.
    let mut hunk_texts: HashMap<(usize, usize), String> = HashMap::new();
    for (i, sec) in sections.iter().enumerate() {
        let header = sec.lines().next().unwrap_or("");
        if !header.starts_with("diff --git ") || sec.contains('\r') {
            continue;
        }
        if super::is_rename_header(header) {
            continue;
        }
        let (prelude, semantic_hunks, native_view) = super::split_semantic_native(sec);
        let Some(native_sec) = native_view else {
            continue;
        };
        let rest = header.strip_prefix("diff --git ")?;
        let path = crate::b_side_path(rest)?;
        if !path.ends_with(".rs") {
            continue;
        }
        preserved.insert(i, (prelude, semantic_hunks));
        native_lens.insert(i, native_sec.len());
        parsed.push((i, path, split_hunks(&native_sec)));
        for (h, nh) in super::split_native_hunks(&native_sec)
            .into_iter()
            .enumerate()
        {
            hunk_texts.insert((i, h), nh.text);
        }
    }
    if parsed.is_empty() {
        return None;
    }
    // Group parsed-section keys by path (single-file groups only).
    let mut by_path: HashMap<&str, Vec<usize>> = HashMap::new();
    for (k, (_, path, _)) in parsed.iter().enumerate() {
        by_path.entry(path.as_str()).or_default().push(k);
    }
    let mut suppressed: HashSet<(String, String)> = HashSet::new();
    let mut fire: Option<(Box<Fire>, String, String)> = None;
    for (path, keys) in &by_path {
        if pre.get(*path).is_none() {
            continue;
        }
        match joint_candidate(&parsed, keys, path, pre, &hunk_texts) {
            Candidate::None => {}
            Candidate::Suppress(old) => {
                suppressed.insert((path.to_string(), old));
            }
            Candidate::Ready(f) => {
                if fire.is_some() {
                    // Two fireable joints in one patch: the second is
                    // suppressed instead (one joint intent per pre-pass
                    // keeps rule provenance unambiguous).
                    suppressed.insert((path.to_string(), f.old));
                    continue;
                }
                let rule = unique_rule(used_rules, &format!("rename_{}_to_{}", f.old, f.new));
                let emission = super::fn_joint_emit::emit(path, &rule, &f);
                let probed = match &f.joint {
                    Joint::A | Joint::B1 { .. } | Joint::Body { .. } => {
                        let pairs: Vec<Pair> = f
                            .claims
                            .iter()
                            .map(|c| Pair {
                                sec: 0,
                                path: path.to_string(),
                                minus: c.minus.clone(),
                                plus: c.plus.clone(),
                                site: c.site,
                            })
                            .collect();
                        probe_reproduces(pre, &emission, &pairs)
                    }
                    Joint::B2 { .. } => {
                        let calls: Vec<(usize, String)> =
                            f.claims.iter().map(|c| (c.site, c.plus.clone())).collect();
                        probe_attr_joint(pre, &emission, path, &f, &calls)
                    }
                };
                if !probed {
                    used_rules.truncate(checkpoint);
                    suppressed.insert((path.to_string(), f.old));
                    continue;
                }
                // WI-4e: native-view bytes so preserved hunks never tax the win.
                // WI-4f-next: mixed sections compare emission hunks vs
                // claimed hunk bytes (shared header stays in place).
                let (claimed_bytes, emission_bytes) = if f.leftover.is_empty() {
                    let claimed: usize = keys
                        .iter()
                        .map(|k| native_lens.get(&parsed[*k].0).copied().unwrap_or(0))
                        .sum();
                    (claimed, emission.len())
                } else {
                    let eh: usize = super::emission_hunks(&emission)
                        .iter()
                        .map(|h| h.len())
                        .sum();
                    (f.claimed_hunk_bytes, eh)
                };
                // WI-8 slice 2: B1 two-hunk emission prices the WI-7 precision
                // allowance over the proved site set; a split fire prices
                // the same allowance with its header overhead emission-side
                // (helper in the split sibling); every other joint keeps
                // strict T375 (precision is a superset of strict, so prior
                // N=2 fires are byte-identical).
                let wins = match &f.joint {
                    Joint::B1 { .. } => super::guard::within_precision(
                        emission_bytes,
                        claimed_bytes,
                        f.claims.len(),
                        0,
                    ),
                    _ if f.split_extra > 0 => split_fire_gate(
                        emission_bytes,
                        claimed_bytes,
                        f.claims.len(),
                        f.split_extra,
                    ),
                    _ => emission_bytes < claimed_bytes,
                };
                if !wins {
                    used_rules.truncate(checkpoint);
                    suppressed.insert((path.to_string(), f.old));
                    continue;
                }
                fire = Some((f, path.to_string(), rule));
            }
        }
    }
    match fire {
        Some((f, def_path, rule)) => {
            let emission = super::fn_joint_emit::emit(&def_path, &rule, &f);
            let claimed: HashSet<usize> = f.claimed_secs.iter().copied().collect();
            let first = *claimed.iter().min().unwrap_or(&0);
            let new_hunks = super::emission_hunks(&emission);
            let mut out: Vec<String> = Vec::new();
            for (i, sec) in sections.iter().enumerate() {
                if !claimed.contains(&i) {
                    out.push(sec.clone());
                    continue;
                }
                let (prelude, keep) = preserved.get(&i).cloned().unwrap_or_default();
                let rest = f.leftover.get(&i).cloned().unwrap_or_default();
                if i == first {
                    if keep.is_empty() && rest.is_empty() {
                        out.push(emission.clone());
                    } else {
                        let mut rebuilt = prelude;
                        for h in &keep {
                            rebuilt.push_str(h);
                        }
                        for h in &new_hunks {
                            rebuilt.push_str(h);
                        }
                        for h in &rest {
                            rebuilt.push_str(h);
                        }
                        out.push(rebuilt);
                    }
                } else if keep.is_empty() && rest.is_empty() {
                    // Fully claimed non-def section collapses away.
                } else {
                    let mut rebuilt = prelude;
                    for h in &keep {
                        rebuilt.push_str(h);
                    }
                    for h in &rest {
                        rebuilt.push_str(h);
                    }
                    out.push(rebuilt);
                }
            }
            Some((out, def_path, rule, suppressed))
        }
        None => {
            // WI-4c: Joint-A workspace fallback (def in one file,
            // callee-only calls across files). Single-file groups each
            // saw only their own file, so this shape declined silently
            // above; the per-hunk fallback would otherwise fragment it
            // into an ordinary def plus a rename-claiming call sweep.
            match joint_workspace_candidate(&parsed, pre, &hunk_texts) {
                Workspace::None => {}
                Workspace::Suppress(paths, old) => {
                    for p in paths {
                        suppressed.insert((p, old.clone()));
                    }
                }
                Workspace::Ready(f, def_path) => {
                    // WI-4f: Joint-B rides two sections (def hunk at the
                    // def file, call template at the single call file,
                    // one rule) — the single-section emission below only
                    // serves Joint-A.
                    if !matches!(f.joint, Joint::A | Joint::Body { .. }) {
                        return match super::fn_joint_ws::rebuild_workspace_b(
                            super::fn_joint_ws::WsBRebuild {
                                sections,
                                parsed: &parsed,
                                preserved: &preserved,
                                native_lens: &native_lens,
                                pre,
                                fire: &f,
                                def_path: &def_path,
                                used_rules,
                                checkpoint,
                                suppressed: &mut suppressed,
                            },
                        ) {
                            Some((out, dp, rule)) => Some((out, dp, rule, suppressed)),
                            None => {
                                if suppressed.is_empty() {
                                    used_rules.truncate(checkpoint);
                                    None
                                } else {
                                    Some((
                                        sections.to_vec(),
                                        String::new(),
                                        String::new(),
                                        suppressed,
                                    ))
                                }
                            }
                        };
                    }
                    let rule = unique_rule(used_rules, &format!("rename_{}_to_{}", f.old, f.new));
                    let emission = super::fn_joint_emit::emit(&def_path, &rule, &f);
                    let pairs: Vec<Pair> = f
                        .claims
                        .iter()
                        .map(|c| {
                            let path = parsed
                                .iter()
                                .find(|(sec, _, _)| *sec == c.sec)
                                .map(|(_, p, _)| p.clone())
                                .unwrap_or_else(|| def_path.clone());
                            Pair {
                                sec: 0,
                                path,
                                minus: c.minus.clone(),
                                plus: c.plus.clone(),
                                site: c.site,
                            }
                        })
                        .collect();
                    let probed = probe_reproduces(pre, &emission, &pairs);
                    // WI-4e native-view bytes; WI-4f-next2 subset bytes
                    // when leftovers are nonempty (shared header stays).
                    let (claimed_bytes, emission_bytes) = if f.leftover.is_empty() {
                        let claimed: usize = f
                            .claimed_secs
                            .iter()
                            .map(|s| native_lens.get(s).copied().unwrap_or(0))
                            .sum();
                        (claimed, emission.len())
                    } else {
                        let eh: usize = super::emission_hunks(&emission)
                            .iter()
                            .map(|h| h.len())
                            .sum();
                        (f.claimed_hunk_bytes, eh)
                    };
                    if probed && emission_bytes < claimed_bytes {
                        let claimed: HashSet<usize> = f.claimed_secs.iter().copied().collect();
                        let first = *claimed.iter().min().unwrap_or(&0);
                        let new_hunks = super::emission_hunks(&emission);
                        let mut out: Vec<String> = Vec::new();
                        for (i, sec) in sections.iter().enumerate() {
                            if !claimed.contains(&i) {
                                out.push(sec.clone());
                                continue;
                            }
                            let rest = f.leftover.get(&i).cloned().unwrap_or_default();
                            if i == first {
                                let keep = preserved
                                    .get(&i)
                                    .map(|(_, h)| h.clone())
                                    .unwrap_or_default();
                                if keep.is_empty() && rest.is_empty() {
                                    out.push(emission.clone());
                                } else {
                                    let (prelude, _) =
                                        preserved.get(&i).cloned().unwrap_or_default();
                                    let mut rebuilt = prelude;
                                    for h in &keep {
                                        rebuilt.push_str(h);
                                    }
                                    for h in &new_hunks {
                                        rebuilt.push_str(h);
                                    }
                                    for h in &rest {
                                        rebuilt.push_str(h);
                                    }
                                    out.push(rebuilt);
                                }
                            } else {
                                let keep = preserved
                                    .get(&i)
                                    .map(|(_, h)| h.clone())
                                    .unwrap_or_default();
                                if keep.is_empty() && rest.is_empty() {
                                    continue;
                                }
                                let (prelude, _) = preserved.get(&i).cloned().unwrap_or_default();
                                let mut rebuilt = prelude;
                                for h in &keep {
                                    rebuilt.push_str(h);
                                }
                                for h in &rest {
                                    rebuilt.push_str(h);
                                }
                                out.push(rebuilt);
                            }
                        }
                        return Some((out, def_path, rule, suppressed));
                    }
                    used_rules.truncate(checkpoint);
                    // Probe/T375 fail: suppress the involved files so the
                    // fallback stays wholly ordinary (same posture as the
                    // single-file veto).
                    let involved: HashSet<usize> = f.claimed_secs.iter().copied().collect();
                    for (sec, path, _) in &parsed {
                        if involved.contains(sec) {
                            suppressed.insert((path.clone(), f.old.clone()));
                        }
                    }
                }
            }
            if suppressed.is_empty() {
                used_rules.truncate(checkpoint);
                None
            } else {
                Some((sections.to_vec(), String::new(), String::new(), suppressed))
            }
        }
    }
}

/// Split one native section into per-hunk minus/plus runs with their
/// old-side sites (0-context `-U0` hunks hold pure `-`/`+` runs, but
/// context lines are skipped soundly when present).
pub(crate) fn split_hunks(section: &str) -> Vec<HunkPairs> {
    let mut out: Vec<HunkPairs> = Vec::new();
    let mut current: Option<(Vec<String>, Vec<String>, Vec<String>)> = None;
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            if let Some((raw, minuses, pluses)) = current.take() {
                out.push(finish_hunk(raw, minuses, pluses));
            }
            current = Some((vec![line.to_string()], Vec::new(), Vec::new()));
            continue;
        }
        if let Some((raw, minuses, pluses)) = current.as_mut() {
            raw.push(line.to_string());
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            }
        }
    }
    if let Some((raw, minuses, pluses)) = current.take() {
        out.push(finish_hunk(raw, minuses, pluses));
    }
    out
}

/// Pair one hunk positionally (the whole-section balanced gate in
/// `section_pairs` style, per hunk): balanced iff every `-` has its
/// `+` in global order.
fn finish_hunk(raw: Vec<String>, minuses: Vec<String>, pluses: Vec<String>) -> HunkPairs {
    let balanced = minuses.len() == pluses.len() && !minuses.is_empty();
    let sites = if balanced {
        crate::totality::changed_old_lines(&raw.join("\n"))
    } else {
        Vec::new()
    };
    HunkPairs {
        minuses,
        pluses,
        balanced,
        sites,
    }
}
