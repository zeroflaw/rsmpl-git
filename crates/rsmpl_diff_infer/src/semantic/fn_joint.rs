//! WI-4a fn rename+slot joint intent (SPEC D.4 `fn_joint`).
//!
//! A `fn` definition renamed *while* its header evolves (params or
//! return slot — SPEC 14.7 header evolution) joined with N-1 call-callee
//! use lines of the old name compresses to the single definition hunk
//! *with* `# follow=uses` + `# sweep`, the header pair carrying both
//! deltas. The engine follow rewrites the callee names at apply time,
//! so the emission is one hunk — but only when the call pairs are
//! callee-only (arguments token-identical): the follow pass never adds
//! arguments, and a single hunk claiming otherwise would fail its own
//! probe (D13).
//!
//! Claim-then-veto suppresses instead of fragmenting: when the joint
//! candidate is present but any fire gate fails (body/attr delta,
//! arg-delta calls, vis/qual/generics/where delta, twin defs, probe or
//! T375 fail), the old callee name is suppressed for its file — the
//! per-hunk fallback leaves those call hunks ordinary instead of
//! emitting a rename-claiming sweep over a definition delta it cannot
//! see (the F4b/F4d false-completeness shape). Suppression is keyed per
//! `(path, old)`: only sections that took part in the vetoed candidate.
//!
//! Scope (WI-4b landed Joint-B1/B2; WI-4c landed Joint-A; WI-4f lands
//! workspace Joint-B): single-file groups fire A/B1/B2; workspace groups
//! fire Joint-A (def in one file, callee-only calls across files, one
//! follow hunk at the def) plus Joint-B1/B2 when the calls live in
//! exactly one other file (def hunk at the def file + call-template
//! hunk at the call file, one shared rule). Body deltas, return-plus-
//! arg-add doubles, slot-plus-attr doubles, mixed def-file calls,
//! multi-call-file groups, and hetero call shapes suppress without
//! raising (their composition has no probe-sound two-hunk form).

use std::collections::{HashMap, HashSet};

use super::fn_joint_b2::{attr_def_shape, attr_joint_fire, probe_attr_joint};
pub(crate) use super::fn_joint_calls::{
    call_parts, is_call_callee_of, is_call_site_of, is_callee_only,
};
use super::fn_joint_sig::{
    arg_add_template, header_argadd_gates, header_fire_gates, header_name_delta,
    slots_equal_except_name,
};
use super::fn_joint_ws::{joint_workspace_candidate, Workspace};
use super::item_intent::{count_kind_defs, probe_reproduces, Pair};
use super::unique_rule;

use super::fn_joint_types::{Fire, FlatPair, HunkPairs, Joint, JointOutcome};

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
                // allowance over the proved site set; every other joint keeps
                // strict T375 (precision is a superset of strict, so prior
                // N=2 fires are byte-identical).
                let wins = match &f.joint {
                    Joint::B1 { .. } => super::guard::within_precision(
                        emission_bytes,
                        claimed_bytes,
                        f.claims.len(),
                        0,
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

/// Candidate verdict for one single-file group.
enum Candidate {
    /// No joint shape here (pure rename, no def pair, twin defs, no
    /// pre-image, …) — existing behavior untouched, no suppression.
    None,
    /// Joint shape present but vetoed — suppress the old callee name.
    Suppress(String),
    /// Fireable joint payload (boxed: the B2 shape carries two
    /// templates plus the attr line).
    Ready(Box<Fire>),
}

/// Classify one single-file group (sections `keys` of `parsed`, all the
/// same `path`).
fn joint_candidate(
    parsed: &[(usize, String, Vec<HunkPairs>)],
    keys: &[usize],
    path: &str,
    pre: &HashMap<String, String>,
    hunk_texts: &HashMap<(usize, usize), String>,
) -> Candidate {
    // Positional pairs from balanced hunks, plus header-pair candidates
    // scanned from unbalanced hunks (an unbalanced attr-shaped def hunk
    // fires as Joint-B2; any other unbalanced def hunk is a joint shape
    // that must suppress the fragmented fallback).
    let mut flat: Vec<FlatPair> = Vec::new();
    // (sec, hunk, minus, plus) header pairs found only via the
    // unbalanced scan (no sites — never fireable as Joint-A/B1).
    // Full minus/plus runs of every unbalanced hunk (Joint-B2 shapes).
    let mut combo_defs: Vec<(usize, usize, String, String)> = Vec::new();
    let mut unbalanced_hunks: Vec<(usize, usize, Vec<String>, Vec<String>)> = Vec::new();
    for k in keys {
        let (sec, _, hunks) = &parsed[*k];
        for (h, hp) in hunks.iter().enumerate() {
            if !hp.balanced {
                unbalanced_hunks.push((*sec, h, hp.minuses.clone(), hp.pluses.clone()));
                for m in &hp.minuses {
                    for p in &hp.pluses {
                        if header_name_delta(m, p).is_some() {
                            combo_defs.push((*sec, h, m.clone(), p.clone()));
                        }
                    }
                }
                continue;
            }
            for ((m, p), s) in hp.minuses.iter().zip(hp.pluses.iter()).zip(hp.sites.iter()) {
                flat.push(FlatPair {
                    sec: *sec,
                    hunk: h,
                    minus: m.clone(),
                    plus: p.clone(),
                    site: *s,
                });
            }
        }
    }
    // Exactly one `fn`-header pair with a name delta (column-0 only —
    // indented headers are methods, WI-13 residue).
    let mut def: Option<usize> = None;
    for (i, fp) in flat.iter().enumerate() {
        if header_name_delta(&fp.minus, &fp.plus).is_some() {
            if def.is_some() {
                return Candidate::None;
            }
            def = Some(i);
        }
    }
    // A combo-scanned def plus a positional one (or two) is ambiguity,
    // never a joint.
    if def.is_some() && !combo_defs.is_empty() {
        return Candidate::None;
    }
    if def.is_none() && combo_defs.len() != 1 {
        return Candidate::None;
    }
    let (old, new, def_single) = match def {
        Some(def_idx) => {
            let (old, new) = header_name_delta(&flat[def_idx].minus, &flat[def_idx].plus)
                .expect("def pair re-parses");
            // The def hunk holding exactly the header pair is the only
            // fireable shape; extra pairs in the same hunk are body/attr
            // lines of the definition (still a joint shape, suppress).
            let h = flat[def_idx].hunk;
            let s = flat[def_idx].sec;
            let len = flat.iter().filter(|fp| fp.sec == s && fp.hunk == h).count();
            (old, new, len == 1)
        }
        None => {
            let (_, _, m, p) = &combo_defs[0];
            let (old, new) = header_name_delta(m, p).expect("combo def re-parses");
            (old, new, false)
        }
    };
    // A pure rename (header slots equal in a single-pair def hunk) is
    // item_intent's business — never a joint candidate, never
    // suppressed. A header-identical pair with extra def-hunk lines
    // (F4b body-literal shape) IS a joint shape: the definition delta
    // lives outside the header pair.
    if def_single && def.is_some_and(|d| slots_equal_except_name(&flat[d].minus, &flat[d].plus)) {
        return Candidate::None;
    }
    // At least one call-callee pair of old→new must exist.
    let mut call_count = 0usize;
    for (i, fp) in flat.iter().enumerate() {
        if def.is_some_and(|d| d == i) {
            continue;
        }
        if is_call_site_of(&fp.minus, &fp.plus, &old, &new) {
            call_count += 1;
        }
    }
    if call_count == 0 {
        return Candidate::None;
    }
    // Unique-definition guard (same posture as item_intent): twin `fn
    // old` defs decline silently (no suppression — a twin file may
    // hold a legitimate independent sweep).
    let probe_pair = Pair {
        sec: 0,
        path: path.to_string(),
        minus: String::new(),
        plus: String::new(),
        site: 0,
    };
    if count_kind_defs(pre, std::slice::from_ref(&probe_pair), "fn", &old) != 1 {
        return Candidate::None;
    }
    // WI-20 slice 1: rename+single-expr-body joint fires here; any other
    // shape falls through to the Joint-A/B path (which suppresses the
    // fragmented fallback as today).
    if let Some(def_idx) = def {
        if let Some(fire) = super::fn_joint_body::try_body_joint(
            &flat,
            def_idx,
            hunk_texts,
            !unbalanced_hunks.is_empty(),
            &old,
            &new,
        ) {
            return Candidate::Ready(fire);
        }
    }
    // Any veto from here on suppresses: the file holds a joint shape
    // the raiser cannot prove, and the fragmented fallback (ordinary
    // Any veto from here on suppresses: the file holds a joint shape
    // the raiser cannot prove, and the fragmented fallback (ordinary
    // def + rename-claiming sweep) must not stand in for it.
    //
    // WI-4f-next: hunk-level claiming — group balanced pairs per
    // (sec, hunk); hunks whose pairs are not all call sites become
    // leftovers instead of vetoing. Unbalanced hunks without a header
    // delta are leftovers too (B2 attr hunk stays claimed).
    let mut hunk_members: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, fp) in flat.iter().enumerate() {
        hunk_members.entry((fp.sec, fp.hunk)).or_default().push(i);
    }
    // WI-4b Joint-B2 (no positional def): an unbalanced attr-shaped
    // def hunk plus uniform callee-only calls composes to the
    // shared-rule two-hunk form.
    if !def_single {
        // Partition balanced hunks: call-site hunks claimed, others
        // leftover. Other unbalanced hunks with a header delta are
        // ambiguity (suppress); without one they are leftovers.
        let mut claimed_flat_idx: Vec<usize> = Vec::new();
        let mut leftover: HashMap<usize, Vec<String>> = HashMap::new();
        for ((sec, h), members) in &hunk_members {
            let all_call = members
                .iter()
                .all(|i| is_call_site_of(&flat[*i].minus, &flat[*i].plus, &old, &new));
            if all_call {
                claimed_flat_idx.extend(members.iter().copied());
            } else if let Some(t) = hunk_texts.get(&(*sec, *h)) {
                leftover.entry(*sec).or_default().push(t.clone());
            }
        }
        // Attr-shaped unbalanced hunks: exactly one must match (the
        // combo def); the rest are leftovers unless they carry a header
        // delta (ambiguity → suppress).
        let mut attr_hunks: Vec<(usize, usize)> = Vec::new();
        for (sec, h, m, p) in &unbalanced_hunks {
            if attr_def_shape(m, p).is_some() {
                attr_hunks.push((*sec, *h));
            } else {
                let mut has_header = false;
                for mm in m {
                    for pp in p {
                        if header_name_delta(mm, pp).is_some() {
                            has_header = true;
                        }
                    }
                }
                if has_header {
                    return Candidate::Suppress(old);
                }
                if let Some(t) = hunk_texts.get(&(*sec, *h)) {
                    leftover.entry(*sec).or_default().push(t.clone());
                }
            }
        }
        if attr_hunks.len() != 1 || unbalanced_hunks.is_empty() {
            return Candidate::Suppress(old);
        }
        // Lone call sites (N=1) never pay the second hunk: the
        // unique-single detector would re-harvest the call-template
        // hunk on the emission's second pass and fragment the
        // intent (pinned) — suppress like every other veto.
        let claimed_calls: Vec<FlatPair> = {
            let mut v: Vec<FlatPair> = claimed_flat_idx.iter().map(|i| flat[*i].clone()).collect();
            v.sort_by_key(|fp| (fp.sec, fp.hunk, fp.site));
            v
        };
        if claimed_calls.len() < 2 {
            return Candidate::Suppress(old);
        }
        let def_hunk = unbalanced_hunks
            .iter()
            .find(|(s, h, _, _)| attr_hunks.contains(&(*s, *h)))
            .map(|(_, _, m, p)| (m.clone(), p.clone()));
        let Some((dm, dp)) = def_hunk else {
            return Candidate::Suppress(old);
        };
        let single_attr: Vec<(usize, usize, Vec<String>, Vec<String>)> = unbalanced_hunks
            .iter()
            .filter(|(s, h, _, _)| attr_hunks.contains(&(*s, *h)))
            .cloned()
            .collect();
        if let Some(mut joint_fire) =
            attr_joint_fire(path, pre, &single_attr, &claimed_calls, &old, &new)
        {
            let mut claimed_secs: Vec<usize> = Vec::new();
            let mut seen: HashSet<usize> = HashSet::new();
            for fp in &claimed_calls {
                if seen.insert(fp.sec) {
                    claimed_secs.push(fp.sec);
                }
            }
            if let Some((s, _, _, _)) = single_attr.first() {
                if seen.insert(*s) {
                    claimed_secs.push(*s);
                }
            }
            joint_fire.claimed_secs = claimed_secs;
            joint_fire.leftover = leftover;
            joint_fire.claimed_hunk_bytes = claimed_calls
                .iter()
                .map(|fp| {
                    hunk_texts
                        .get(&(fp.sec, fp.hunk))
                        .map(|t| t.len())
                        .unwrap_or(0)
                })
                .sum::<usize>()
                + single_attr
                    .first()
                    .and_then(|(s, h, _, _)| hunk_texts.get(&(*s, *h)))
                    .map(|t| t.len())
                    .unwrap_or(0);
            // Claims for B2 are the call pairs; the attr def rides the
            // def hunk (kept in `def_minus/plus`).
            let _ = (dm, dp);
            return Candidate::Ready(Box::new(joint_fire));
        }
        return Candidate::Suppress(old);
    }
    let def_idx = def.expect("def_single implies a positional def");
    // WI-4f-next partition: def hunk claimed; every other balanced hunk
    // claimed iff all its pairs are call-callee sites. Unbalanced hunks
    // without a header delta are leftovers (combo_defs already vetoed
    // the header-delta case above).
    let mut claimed_flat_idx: Vec<usize> = Vec::new();
    let mut leftover: HashMap<usize, Vec<String>> = HashMap::new();
    for ((sec, h), members) in &hunk_members {
        if members.contains(&def_idx) {
            claimed_flat_idx.extend(members.iter().copied());
            continue;
        }
        let all_call = members
            .iter()
            .all(|i| is_call_site_of(&flat[*i].minus, &flat[*i].plus, &old, &new));
        if all_call {
            claimed_flat_idx.extend(members.iter().copied());
        } else if let Some(t) = hunk_texts.get(&(*sec, *h)) {
            leftover.entry(*sec).or_default().push(t.clone());
        }
    }
    for (sec, h, _, _) in &unbalanced_hunks {
        if let Some(t) = hunk_texts.get(&(*sec, *h)) {
            leftover.entry(*sec).or_default().push(t.clone());
        }
    }
    claimed_flat_idx.sort_unstable();
    let claimed_flat: Vec<FlatPair> = claimed_flat_idx.iter().map(|i| flat[*i].clone()).collect();
    if claimed_flat.len() < 2 {
        return Candidate::None;
    }
    let claimed_def_pos = claimed_flat
        .iter()
        .position(|fp| {
            fp.sec == flat[def_idx].sec
                && fp.hunk == flat[def_idx].hunk
                && fp.site == flat[def_idx].site
        })
        .expect("def in claimed set");
    let callee_only = claimed_flat
        .iter()
        .enumerate()
        .all(|(i, fp)| i == claimed_def_pos || is_callee_only(&fp.minus, &fp.plus, &old, &new));
    let joint = if callee_only {
        // Joint-A fire gates on the header (header_fire_gates): vis in
        // the WI-1 closed set; generics/where preserved; evolving ⊆
        // {vis, qual, params, return} with params or return evolving,
        // at most two evolve (WI-8 slices 1+4).
        if !header_fire_gates(&flat[def_idx].minus, &flat[def_idx].plus) {
            return Candidate::Suppress(old);
        }
        Joint::A
    } else {
        // WI-4b Joint-B1: every call adds the same concrete trailing
        // args or retypes the trailing args uniformly (uniform
        // template below); the header must evolve params only with
        // the return preserved — a return delta plus a call evolution
        // is two evolutions the shared-rule pair cannot restate. N=1
        // (def plus one call) pays the second hunk under the
        // precision allowance at the fire gate (WI-8 slice 2); the len
        // gate above already guarantees at least one call.
        if !header_argadd_gates(&flat[def_idx].minus, &flat[def_idx].plus) {
            return Candidate::Suppress(old);
        }
        match arg_add_template(&claimed_flat, claimed_def_pos, &old, &new) {
            Some((call_minus, call_plus)) => Joint::B1 {
                call_minus,
                call_plus,
            },
            None => return Candidate::Suppress(old),
        }
    };
    let def_minus = flat[def_idx].minus.clone();
    let def_plus = flat[def_idx].plus.clone();
    let mut claimed_secs: Vec<usize> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    for fp in &claimed_flat {
        if seen.insert(fp.sec) {
            claimed_secs.push(fp.sec);
        }
    }
    let claimed_hunk_bytes: usize = {
        let mut seen_h: HashSet<(usize, usize)> = HashSet::new();
        let mut sum = 0usize;
        for fp in &claimed_flat {
            if seen_h.insert((fp.sec, fp.hunk)) {
                sum += hunk_texts
                    .get(&(fp.sec, fp.hunk))
                    .map(|t| t.len())
                    .unwrap_or(0);
            }
        }
        sum
    };
    Candidate::Ready(Box::new(Fire {
        old,
        new,
        def_minus,
        def_plus,
        claims: claimed_flat,
        claimed_secs,
        joint,
        leftover,
        claimed_hunk_bytes,
    }))
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

#[cfg(test)]
mod tests {
    use super::super::fn_joint_emit::with_body_hole;
    use super::super::{infer_semantic_with_pre_images, split_item};
    use super::*;
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
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

    const PRE: &str = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";

    fn params_sections() -> Vec<String> {
        vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )]
    }

    #[test]
    fn joint_params_fires_with_follow() {
        let pre = pre_map(&[("src.rs", PRE)]);
        let out =
            infer_semantic_with_pre_images(&params_sections(), &pre).expect("joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("# sweep"), "got:\n{out}");
        assert!(out.contains("- fn foo(x: i32) { ... }"), "got:\n{out}");
        assert!(out.contains("+ fn bar(x: u32) { ... }"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"], "fn bar(x: u32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "joint must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_return_fires_with_follow() {
        let pre = "fn foo(x: i32) -> i32 {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> u32 {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("return joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(
            out.contains("+ fn bar(x: i32) -> u32 { ... }"),
            "got:\n{out}"
        );
        validated_live(&out, &pre);
    }

    #[test]
    fn joint_qualified_and_let_calls_fire() {
        let pre =
            "fn foo(x: i32) {\n    x\n}\nfn m() {\n    crate::m::foo(1);\n    let y = foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,2 +5,2 @@\n-    crate::m::foo(1);\n-    let y = foo(2);\n+    crate::m::bar(1);\n+    let y = bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("qualified/let joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["src.rs"].contains("crate::m::bar(1);"),
            "got: {}",
            files["src.rs"]
        );
        assert!(
            files["src.rs"].contains("let y = bar(2);"),
            "got: {}",
            files["src.rs"]
        );
    }

    #[test]
    fn joint_single_call_site_fires() {
        // N=1 call site still joins (the probe is the uniqueness guard).
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,1 +5,1 @@\n-    foo(1);\n+    bar(1);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("N=1 joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        validated_live(&out, &pre);
    }

    #[test]
    fn joint_b2_attr_add_single_call_stays_ordinary() {
        // N=1 never pays the second hunk (the unique-single detector
        // would re-harvest the call template and fragment the intent)
        // — the vetoed shape stays wholly ordinary, pinned.
        // (WI-6 slice 1 changes nothing here: the suppressed-callee
        // call hunk never reaches the single core — the WI-4a
        // suppression gate in the per-hunk path owns it first.)
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,1 +6,1 @@\n-    foo(1);\n+    bar(1);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "B2 N=1 must stay wholly ordinary, got: {full:?}"
        );
    }
    #[test]
    fn body_delta_fires_as_single_expr_joint() {
        // WI-20 slice 1: rename + single general-expr body line +
        // callee-only calls raises to the single follow hunk carrying
        // the header `...` pair plus the body pair lines (was F4b
        // wholly-ordinary veto before the body slot joined the intent).
        let pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: i32) -> i32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("body joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(
            out.contains("- fn foo(x: i32) -> i32 { ... }"),
            "got:\n{out}"
        );
        assert!(
            out.contains("+ fn bar(x: i32) -> i32 { ... }"),
            "got:\n{out}"
        );
        assert!(out.contains("- x + 1"), "got:\n{out}");
        assert!(out.contains("+ x + 2"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    x + 2\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "body joint must round-trip byte-identical"
        );
    }

    #[test]
    fn attr_change_vetoes_to_wholly_ordinary() {
        // WI-4 residue (WI-5 owns the composition): rename+attr-*change*
        // (balanced `#[inline]` → `#[inline(never)]` hunk) must not
        // fragment into ordinary def + rename-claiming sweep — the whole
        // section stays ordinary (the old F4b body-delta veto fired as a
        // joint since WI-20 slice 1; attr-change still has no joint).
        let pre =
            "#[inline]\nfn foo(x: i32) -> i32 {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-#[inline]\n-fn foo(x: i32) -> i32 {\n+#[inline(never)]\n+fn bar(x: i32) -> i32 {\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let mut used = Vec::new();
        let out = try_fn_joint_intent(&sections, &pre, &mut used);
        let Some((rewritten, _, rule, suppressed)) = out else {
            panic!("attr-change joint must suppress, not decline silently");
        };
        assert!(rule.is_empty(), "suppress-only carries no rule");
        assert_eq!(rewritten, sections, "suppress-only leaves bytes identical");
        assert_eq!(
            suppressed,
            HashSet::from([("src.rs".to_string(), "foo".to_string())])
        );
        // End to end: no semantic hunk at all (the sweep is suppressed).
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "attr-change veto must stay wholly ordinary"
        );
    }

    #[test]
    fn joint_b2_attr_add_fires_shared_rule() {
        // WI-4b B2 (F4d): rename+attr-add with callee-only calls raises
        // to the shared-rule two-hunk composition (def hunk carrying the
        // attr line + call-template hunk, no follow). Real-apply is
        // byte-identical.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B2 attr joint must fire");
        assert!(
            out.matches("rule=r1").count() >= 2,
            "both hunks share one rule, got:\n{out}"
        );
        assert!(
            !out.contains("# follow=uses"),
            "no follow in a joint, got:\n{out}"
        );
        assert!(
            out.contains("+ #[must_use]\n"),
            "attr line must raise, got:\n{out}"
        );
        assert!(
            out.contains("- foo(E0)\n"),
            "call template minus, got:\n{out}"
        );
        assert!(
            out.contains("+ bar(E0)\n"),
            "call template plus, got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "#[must_use]\nfn bar(x: i32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "B2 attr-add must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_b2_attr_remove_fires() {
        // Attr-remove mirror: the minus-side attr line drops, the header
        // renames, calls ride the template.
        let pre =
            "#[must_use]\nfn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,1 @@\n-#[must_use]\n-fn foo(x: i32) {\n+fn bar(x: i32) {\n@@ -6,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B2 remove must fire");
        assert!(out.matches("rule=r1").count() >= 2, "got:\n{out}");
        assert!(
            out.contains("- #[must_use]\n"),
            "minus attr must raise, got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"], "fn bar(x: i32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "B2 attr-remove must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_b2_double_evolution_vetoes_stay_ordinary() {
        // Two evolutions in one commit stay wholly ordinary: params
        // delta under the attr, double attr, arg-delta calls under an
        // attr def, and hetero-arity calls under an attr def.
        for body in [
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: u32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
            "@@ -1,1 +1,3 @@\n-fn foo(x: i32) {\n+#[must_use]\n+#[inline]\n+fn bar(x: i32) {\n@@ -5,2 +7,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(1, 2);\n+    bar(1);\n+    bar(1, 2);\n",
        ] {
            let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
            let sections = vec![section("src.rs", body)];
            let pre = pre_map(&[("src.rs", pre)]);
            let full = infer_semantic_with_pre_images(&sections, &pre);
            assert!(
                full.as_ref().is_none_or(|s| !s.contains("rule=")),
                "B2 double-evolution must stay wholly ordinary for {body}, got: {full:?}"
            );
        }
    }

    #[test]
    fn joint_b2_without_calls_is_not_a_candidate() {
        // Attr+rename with no call sites: no joint shape, no suppression.
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n",
        )];
        let pre = pre_map(&[("src.rs", PRE)]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn joint_b1_arg_add_fires_shared_rule() {
        // WI-4b B1 (F4a): rename+param-add with uniform arg-adding calls
        // raises to the shared-rule two-hunk composition (def hunk +
        // call-template hunk, no follow — a follow would overlap the
        // call hunk's sites). Real-apply is byte-identical.
        let pre = PRE;
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B1 arg-add must fire");
        assert!(
            out.matches("rule=r1").count() >= 2,
            "both hunks share one rule, got:\n{out}"
        );
        assert!(
            !out.contains("# follow=uses"),
            "no follow in a joint, got:\n{out}"
        );
        assert!(
            out.contains("- fn foo(x: i32) { ... }"),
            "def minus must raise, got:\n{out}"
        );
        assert!(
            out.contains("+ fn bar(x: i32, y: i32) { ... }"),
            "def plus must raise, got:\n{out}"
        );
        assert!(
            out.contains("- foo(E0)\n"),
            "call template minus, got:\n{out}"
        );
        assert!(
            out.contains("+ bar(E0, 0)\n"),
            "call template plus, got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32, y: i32) {\n    x\n}\nfn m() {\n    bar(1, 0);\n    bar(2, 0);\n}\n",
            "B1 must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_b1_zero_prefix_and_qualified_fire() {
        // Zero-prefix addition (`foo()` → `bar(false)`) and qualified
        // uniform calls (`crate::m::foo(1)` → `crate::m::bar(1, 0)`)
        // each compose the same way.
        for (pre_text, body, minus_tpl, plus_tpl) in [
            (
                "fn foo() {\n    1\n}\nfn m() {\n    foo();\n    foo();\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo() {\n+fn bar(x: i32) {\n@@ -5,2 +5,2 @@\n-    foo();\n-    foo();\n+    bar(false);\n+    bar(false);\n",
                "- foo()\n",
                "+ bar(false)\n",
            ),
            (
                "fn foo(x: i32) {\n    x\n}\nfn m() {\n    crate::m::foo(1);\n    crate::m::foo(2);\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    crate::m::foo(1);\n-    crate::m::foo(2);\n+    crate::m::bar(1, 0);\n+    crate::m::bar(2, 0);\n",
                "- crate::m::foo(E0)\n",
                "+ crate::m::bar(E0, 0)\n",
            ),
            (
                "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo::<u8>(1);\n    foo::<u8>(2);\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo::<u8>(1);\n-    foo::<u8>(2);\n+    bar::<u8>(1, 0);\n+    bar::<u8>(2, 0);\n",
                "- foo::<u8>(E0)\n",
                "+ bar::<u8>(E0, 0)\n",
            ),
            (
                "fn foo(a: i32, b: i32) {\n    a\n}\nfn m() {\n    foo(1, 2);\n    foo(3, 2);\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo(a: i32, b: i32) {\n+fn bar(a: i32, b: u32) {\n@@ -5,2 +5,2 @@\n-    foo(1, 2);\n-    foo(3, 2);\n+    bar(1, 2u32);\n+    bar(3, 2u32);\n",
                "- foo(E0, 2)\n",
                "+ bar(E0, 2u32)\n",
            ),
        ] {
            let sections = vec![section("src.rs", body)];
            let pre = pre_map(&[("src.rs", pre_text)]);
            let out = infer_semantic_with_pre_images(&sections, &pre)
                .unwrap_or_else(|| panic!("B1 twin must fire for {body}"));
            assert!(
                out.matches("rule=r1").count() >= 2,
                "shared rule for {body}, got:\n{out}"
            );
            assert!(out.contains(minus_tpl), "template minus for {body}, got:\n{out}");
            assert!(out.contains(plus_tpl), "template plus for {body}, got:\n{out}");
            validated_live(&out, &pre);
        }
    }

    #[test]
    fn joint_b1_single_call_fires_shared_rule() {
        // WI-8 slice 2: rename plus param-add with a single uniform
        // arg-adding call (N=1) raises to the shared-rule two-hunk B1
        // under the precision allowance — the probe proves the site set.
        let one = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,1 +5,1 @@\n-    foo(1);\n+    bar(1, 0);\n",
        )];
        let pre = pre_map(&[("src.rs", one)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B1 N=1 must fire");
        assert!(
            out.matches("rule=r1").count() >= 2,
            "both hunks share one rule"
        );
        assert!(!out.contains("follow=uses"), "no follow in a joint");
        assert!(out.contains("- foo(E0)"), "call template minus");
        assert!(out.contains("+ bar(E0, 0)"), "call template plus");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"], "fn bar(x: i32, y: i32) {\n    x\n}\nfn m() {\n    bar(1, 0);\n}\n",
            "B1 N=1 must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_b1_qual_argadd_fires() {
        // WI-8 slice 6: rename + qual-run + param-add with uniform
        // arg-adding calls raises to the shared-rule two-hunk B1 at
        // N=2 and N=1 (single-file; workspace inherits via the shared
        // `header_argadd_gates`). Vis + arg-add stays vetoed (pinned
        // in the hetero test); qual + return + arg-add triples veto.
        for (pre_text, body, post) in [
            (
                "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+async fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
                "async fn bar(x: i32, y: i32) {\n    x\n}\nfn m() {\n    bar(1, 0);\n    bar(2, 0);\n}\n",
            ),
            (
                "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+async fn bar(x: i32, y: i32) {\n@@ -5,1 +5,1 @@\n-    foo(1);\n+    bar(1, 0);\n",
                "async fn bar(x: i32, y: i32) {\n    x\n}\nfn m() {\n    bar(1, 0);\n}\n",
            ),
        ] {
            let sections = vec![section("src.rs", body)];
            let pre = pre_map(&[("src.rs", pre_text)]);
            let out = infer_semantic_with_pre_images(&sections, &pre)
                .unwrap_or_else(|| panic!("B1+qual must fire for {body}"));
            assert!(
                out.matches("rule=r1").count() >= 2,
                "both hunks share one rule for {body}, got:\n{out}"
            );
            assert!(!out.contains("follow=uses"), "no follow in a joint");
            assert!(out.contains("- foo(E0)"), "call template minus");
            assert!(out.contains("+ bar(E0, 0)"), "call template plus");
            assert!(
                out.contains("+ async fn bar(x: i32, y: i32) { ... }"),
                "def plus carries qual, got:\n{out}"
            );
            validated_live(&out, &pre);
            let parsed = rsmpl_patch::parse_patch(&out).unwrap();
            let mut files = pre.clone();
            rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
            assert_eq!(files["src.rs"], post, "B1+qual must round-trip");
        }
    }

    #[test]
    fn joint_b1_hetero_vetoes_stay_ordinary() {
        // Hetero suffixes, prefix rewrites, hetero arities, return deltas,
        // and vis deltas each stay wholly ordinary — no uniform
        // template states them. (WI-8 slice 2: lone N=1 additions now fire
        // as B1, pinned by joint_b1_single_call_fires_shared_rule.)
        let pre = PRE;
        for body in [
            // Hetero trailing suffix.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 1);\n",
            // Leading-arg rewrite is not an addition.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(9, 0);\n+    bar(2, 0);\n",
            // Hetero minus arity.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2, 3);\n+    bar(1, 0);\n+    bar(2, 3, 0);\n",
            // Return delta plus arg-add is two evolutions.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32, y: i32) -> u32 {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            // Vis delta plus arg-add stays vetoed (qual alone fires slice 6).
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+pub fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            // Qualifier plus return delta plus arg-add is a triple.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+async fn bar(x: i32, y: i32) -> u32 {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            // Non-uniform trailing retype (WI-8 slice 3).
            "@@ -1,1 +1,1 @@\n-fn foo(a: i32, b: i32) {\n+fn bar(a: i32, b: u32) {\n@@ -5,2 +5,2 @@\n-    foo(1, 2);\n-    foo(3, 2);\n+    bar(1, 2u32);\n+    bar(3, 3u32);\n",
            // Arg deletion is never a B1 evolution.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32, y: i32) {\n+fn bar(x: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1, 0);\n-    foo(2, 0);\n+    bar(1);\n+    bar(2);\n",
        ] {
            let sections = vec![section("src.rs", body)];
            let pre = pre_map(&[("src.rs", pre)]);
            let full = infer_semantic_with_pre_images(&sections, &pre);
            assert!(
                full.as_ref().is_none_or(|s| !s.contains("rule=")),
                "B1 hetero shape must stay wholly ordinary for {body}, got: {full:?}"
            );
        }
    }

    #[test]
    fn pure_rename_is_not_a_candidate() {
        // Suffix-equal renames belong to item_intent: no suppression, and
        // the terse follow emission is untouched.
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", PRE)]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pure rename fires");
        assert!(
            out.contains("- fn foo\n+ fn bar\n"),
            "terse intent kept, got:\n{out}"
        );
    }

    #[test]
    fn twin_defs_decline_silently() {
        // Two `fn foo` defs in one file: the unique-definition guard
        // declines with no suppression (a twin file may hold a
        // legitimate independent sweep).
        let pre =
            "fn foo(x: i32) {\n    x\n}\nfn foo(y: i32) {\n    y\n}\nfn m() {\n    foo(1);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -8,1 +8,1 @@\n-    foo(1);\n+    bar(1);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn def_without_calls_is_not_a_candidate() {
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n",
        )];
        let pre = pre_map(&[("src.rs", PRE)]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
    }

    #[test]
    fn joint_a_qual_params_two_slot_fires() {
        // WI-8 slice 4: name + qual + params (two slots) raises to one
        // follow hunk — the old `qual_delta_suppresses` pin now fires.
        // Qual-only stays the terse WI-5 intent (gate needs
        // params/return); qual+arg-add stays B1-vetoed (pinned below).
        // REVIEW: `async` and `unsafe` spellings ride the same
        // token-compare arm — both pinned here.
        for (plus_hdr, post) in [
            (
                "async fn bar(x: u32) {",
                "async fn bar(x: u32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            ),
            (
                "unsafe fn bar(x: u32) {",
                "unsafe fn bar(x: u32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            ),
        ] {
            let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
            let sections = vec![section(
                "src.rs",
                &format!(
                    "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {{\n+{plus_hdr}\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n"
                ),
            )];
            let pre = pre_map(&[("src.rs", pre)]);
            let out = infer_semantic_with_pre_images(&sections, &pre)
                .unwrap_or_else(|| panic!("qual+params joint must fire for {plus_hdr}"));
            assert!(out.contains("# follow=uses"), "got:\n{out}");
            assert!(out.contains(&format!("+ {plus_hdr} ... }}")), "got:\n{out}");
            validated_live(&out, &pre);
            let parsed = rsmpl_patch::parse_patch(&out).unwrap();
            let mut files = pre.clone();
            rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
            assert_eq!(
                files["src.rs"], post,
                "joint must round-trip byte-identical"
            );
        }
    }

    #[test]
    fn joint_a_vis_params_two_slot_fires() {
        // WI-8 slice 1: name + vis + params (two slots) raises to one
        // follow hunk — the `pub → pub(crate)` half of the old
        // `vis_and_qual_deltas_suppress` pin now fires.
        let pre = "pub fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-pub fn foo(x: i32) {\n+pub(crate) fn bar(x: u32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("vis+params joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(
            out.contains("+ pub(crate) fn bar(x: u32) { ... }"),
            "got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "pub(crate) fn bar(x: u32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "joint must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_a_vis_return_two_slot_fires_at_n1() {
        // WI-8 slice 1 head probe: name + vis + return at a single call
        // site raises to one follow hunk.
        let pre = "fn foo() -> i32 {\n    1\n}\nfn m() {\n    let v = foo();\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo() -> i32 {\n+pub fn bar() -> i64 {\n@@ -5,1 +5,1 @@\n-    let v = foo();\n+    let v = bar();\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("vis+return joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("+ pub fn bar() -> i64 { ... }"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"], "pub fn bar() -> i64 {\n    1\n}\nfn m() {\n    let v = bar();\n}\n",
            "joint must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_a_vis_params_return_triple_suppresses() {
        // Three evolving slots stay wholly ordinary — no joint states
        // them (WI-8 slice 4: qual triples join the vis triple here).
        for body in [
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+pub fn bar(x: u32) -> u64 {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+async fn bar(x: u32) -> u64 {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        ] {
            let pre = "fn foo(x: i32) -> i32 {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
            let sections = vec![section("src.rs", body)];
            let pre = pre_map(&[("src.rs", pre)]);
            let full = infer_semantic_with_pre_images(&sections, &pre);
            assert!(
                full.as_ref().is_none_or(|s| !s.contains("rule=")),
                "triple-slot joint must stay wholly ordinary, got: {full:?}"
            );
        }
    }

    #[test]
    fn joint_a_qual_params_two_slot_fires_at_n1() {
        // WI-8 slice 4 head probe: name + qual + params at a single
        // call site raises to one follow hunk under precision.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+async fn bar(x: u32) {\n@@ -5,1 +5,1 @@\n-    foo(1);\n+    bar(1);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("qual+params N=1 must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(
            out.contains("+ async fn bar(x: u32) { ... }"),
            "got:\n{out}"
        );
        validated_live(&out, &pre);
    }

    #[test]
    fn qual_only_rename_stays_terse() {
        // Qual-only + rename is the WI-5 name-only arm, never Joint-A:
        // the joint gate needs params/return evolution, so the terse
        // emission is intact.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+async fn bar(x: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("terse qual intent must fire");
        assert!(
            out.contains("- fn foo\n+ async fn bar\n"),
            "terse intent kept, got:\n{out}"
        );
    }

    #[test]
    fn vis_only_rename_declines_joint() {
        // Vis-only rename is the terse intent's domain, never Joint-A:
        // the joint pre-pass declines and the terse emission is intact.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+pub fn bar(x: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("terse vis intent must fire");
        assert!(
            out.contains("- fn foo\n+ pub fn bar\n"),
            "terse intent kept, got:\n{out}"
        );
    }

    #[test]
    fn self_referential_vis_suppresses() {
        // `pub(in foo)` renaming `foo` is an evolution, never a joint.
        let pre = "pub(in foo) fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-pub(in foo) fn foo(x: i32) {\n+pub(in foo) fn bar(x: u32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "self-referential vis must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn method_calls_are_not_joint_sites() {
        // `s.foo()` is a different namespace: not a joint use line, and
        // the joint shape around it suppresses rather than firing.
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,1 +5,1 @@\n-    s.foo();\n+    s.bar();\n",
        )];
        let pre = pre_map(&[("src.rs", PRE)]);
        let mut used = Vec::new();
        let out = try_fn_joint_intent(&sections, &pre, &mut used);
        // `s.foo()` is not a call-callee site of `foo`, so the joint
        // shape is absent entirely (no def+call group): silent decline.
        assert!(out.is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn joint_cross_file_fires_with_follow() {
        // WI-4c: def in a.rs, callee-only calls in a.rs + b.rs raise to
        // one follow hunk at the def, real-apply byte-identical in both
        // files (the single-file loop alone would fragment this).
        let a_pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let b_pre = "fn n() {\n    foo(2);\n    foo(3);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,1 +5,1 @@\n-    foo(1);\n+    bar(1);\n",
            ),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(2);\n-    foo(3);\n+    bar(2);\n+    bar(3);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("cross-file joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- fn foo(x: i32) { ... }"), "got:\n{out}");
        assert!(out.contains("+ fn bar(x: u32) { ... }"), "got:\n{out}");
        assert_eq!(
            out.matches("rule=").count(),
            1,
            "one follow hunk, got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["a.rs"], "fn bar(x: u32) {\n    x\n}\nfn m() {\n    bar(1);\n}\n",
            "a.rs must round-trip byte-identical"
        );
        assert_eq!(
            files["b.rs"], "fn n() {\n    bar(2);\n    bar(3);\n}\n",
            "b.rs must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_cross_file_b1_stays_ordinary() {
        // WI-4f: rename+arg-add across files FIRES as a shared-rule
        // two-section emission (def hunk at the def file, call template
        // at the call file) — the WI-4c stays-ordinary posture is
        // superseded; veto shapes still suppress (next test), never a
        // fragment. (Pre-pass level; the infer-level round-trip lives in
        // `fn_joint_ws`.)
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n",
            ),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let mut used = Vec::new();
        let Some((out, def_path, rule, _)) = try_fn_joint_intent(&sections, &pre, &mut used) else {
            panic!("B1 cross-file must fire");
        };
        assert_eq!(def_path, "a.rs");
        assert!(!rule.is_empty());
        assert_eq!(out.len(), 2, "two sections, got: {out:?}");
        assert!(
            out[0].contains(&format!("rule={rule}")) && out[1].contains(&format!("rule={rule}")),
            "one rule across both sections, got: {out:?}"
        );
        validated_live(&out.join(""), &pre);
    }

    #[test]
    fn joint_cross_file_vis_params_two_slot_fires() {
        // WI-8 slice 1: the shared `header_fire_gates` widening rides the
        // workspace Joint-A path too — vis+params across files fires as a
        // probe-verified follow hunk at the def file, never a fragment.
        // (The WI-4c suppress-only posture for vis-changing defs is
        // superseded for the two-slot shapes the gate now admits.)
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+pub(crate) fn bar(x: u32) {\n",
            ),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let mut used = Vec::new();
        let Some((out, def_path, rule, _)) = try_fn_joint_intent(&sections, &pre, &mut used) else {
            panic!("cross-file vis+params joint must fire");
        };
        assert_eq!(def_path, "a.rs");
        assert!(!rule.is_empty());
        validated_live(&out.join(""), &pre);
        let full = infer_semantic_with_pre_images(&sections, &pre).expect("joint must raise");
        assert!(full.contains("# follow=uses"), "got:\n{full}");
        assert!(
            full.contains("+ pub(crate) fn bar(x: u32) { ... }"),
            "got:\n{full}"
        );
    }

    #[test]
    fn joint_cross_file_twin_defs_decline_silent() {
        // Two `fn foo` defs across files: unique-definition guard declines
        // silently (no suppression — an unrelated sweep elsewhere stays).
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n",
            ),
            section(
                "b.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n",
            ),
        ];
        let pre = pre_map(&[
            ("a.rs", "fn foo(x: i32) {\n    x\n}\n"),
            ("b.rs", "fn foo(x: i32) {\n    x\n}\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn joint_hybrid_section_native_pairs_still_fire() {
        // WI-4e: pre-existing `rule=` hunk in the def section does not
        // block the Joint-A fire on the native hunks; the preserved hunk
        // rides alongside the new follow hunk.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let hybrid = "diff --git a/src.rs b/src.rs\n--- a/src.rs\n+++ b/src.rs\n@@ -10,0 +10,0 @@ rule=r1\n# bind=M1\n- I\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n"
            .to_string();
        let sections = vec![hybrid];
        let pre_map = pre_map(&[("src.rs", pre)]);
        let mut used = Vec::new();
        let Some((out, def_path, rule, _)) = try_fn_joint_intent(&sections, &pre_map, &mut used)
        else {
            panic!("native joint pairs in hybrid must fire");
        };
        assert_eq!(def_path, "src.rs");
        assert!(!rule.is_empty());
        assert_eq!(out.len(), 1);
        assert!(
            out[0].contains("rule=r1"),
            "preserved hunk kept:\n{}",
            out[0]
        );
        assert!(
            out[0].contains(&format!("rule={rule}")),
            "new hunk present:\n{}",
            out[0]
        );
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
    }

    #[test]
    fn joint_mixed_section_claims_subset_leaves_leftover() {
        // WI-4f-next: Joint-A def+calls plus an unrelated hunk in one
        // section fires, leftover stays verbatim in place.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\nfn u() {\n    let z = 1;\n}\n";
        let section = "diff --git a/src.rs b/src.rs\n--- a/src.rs\n+++ b/src.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n@@ -9,1 +9,1 @@\n-    let z = 1;\n+    let z = 2;\n"
            .to_string();
        let sections = vec![section];
        let pre_map = pre_map(&[("src.rs", pre)]);
        let mut used = Vec::new();
        let Some((out, def_path, rule, _)) = try_fn_joint_intent(&sections, &pre_map, &mut used)
        else {
            panic!("mixed joint subset must fire");
        };
        assert_eq!(def_path, "src.rs");
        assert!(!rule.is_empty());
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(
            out[0].contains("-    let z = 1;"),
            "leftover verbatim:\n{}",
            out[0]
        );
        validated_live(&out[0], &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["src.rs"].contains("fn bar(x: u32)"),
            "got: {}",
            files["src.rs"]
        );
        assert!(
            files["src.rs"].contains("let z = 2;"),
            "got: {}",
            files["src.rs"]
        );
    }

    #[test]
    fn joint_workspace_mixed_def_section_claims_subset_leaves_leftover() {
        // WI-4f-next2: workspace Joint-A with an unrelated hunk in the
        // DEF file fires on the claimed subset (same-file ordinary+follow
        // composition), leftover stays verbatim, both files round-trip.
        let a_pre = "fn foo(x: i32) {\n    x\n}\nfn z() {\n    let q = 1;\n}\n";
        let b_pre = "fn n() {\n    foo(2);\n    foo(3);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,1 +5,1 @@\n-    let q = 1;\n+    let q = 2;\n",
            ),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(2);\n-    foo(3);\n+    bar(2);\n+    bar(3);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let mut used = Vec::new();
        let Some((out, def_path, rule, _)) = try_fn_joint_intent(&sections, &pre, &mut used) else {
            panic!("workspace mixed subset must fire");
        };
        assert_eq!(def_path, "a.rs");
        assert!(!rule.is_empty());
        assert!(
            out.iter().any(|s| s.contains("# follow=uses")),
            "got: {out:?}"
        );
        assert!(
            out.iter().any(|s| s.contains("-    let q = 1;")),
            "leftover verbatim, got: {out:?}"
        );
        validated_live(&out.join(""), &pre);
        let parsed = rsmpl_patch::parse_patch(&out.join("")).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["a.rs"].contains("fn bar(x: u32)"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("let q = 2;"),
            "got: {}",
            files["a.rs"]
        );
        assert!(files["b.rs"].contains("bar(2)"), "got: {}", files["b.rs"]);
        assert!(!files["b.rs"].contains("foo("), "got: {}", files["b.rs"]);
    }

    #[test]
    fn joint_workspace_mixed_call_section_suppresses() {
        // WI-4f-next2 soundness: an unrelated hunk in a CALL file
        // suppresses the workspace group (wholly ordinary) — the engine
        // follow does not rewrite a call file the patch also edits
        // ordinarily, so firing would silently lose the call rewrites.
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn n() {\n    foo(2);\n    foo(3);\n    let x = 1;\n}\n";
        let sections = vec![
            section("a.rs", "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n"),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(2);\n-    foo(3);\n+    bar(2);\n+    bar(3);\n@@ -4,1 +4,1 @@\n-    let x = 1;\n+    let x = 2;\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let mut used = Vec::new();
        let out = try_fn_joint_intent(&sections, &pre, &mut used);
        let Some((rewritten, rule, _, suppressed)) = out else {
            panic!("call-file mixed group must suppress, not decline silently");
        };
        assert!(rule.is_empty(), "suppress-only carries no rule");
        assert_eq!(rewritten, sections, "suppress-only leaves bytes identical");
        assert!(suppressed.contains(&("a.rs".to_string(), "foo".to_string())));
        assert!(suppressed.contains(&("b.rs".to_string(), "foo".to_string())));
        let full = infer_semantic_with_pre_images(&sections, &pre);
        // The joint stays suppressed (def/call hunks ordinary, no
        // `# follow=uses` — the suppression's purpose); the unrelated
        // `let x` line raises as its own WI-6-slice-1 literal verbatim
        // single (unique site, byte-probe verified), partitioning
        // beside the ordinary remainder instead of stranding it.
        let out = full.expect("partition must emit");
        assert!(
            !out.contains("# follow=uses"),
            "joint must stay suppressed, got: {out}"
        );
        assert!(
            out.contains("-    foo(2);"),
            "call lines must stay ordinary, got: {out}"
        );
        assert!(
            out.contains("- let x = 1;"),
            "unrelated line raises literally, got: {out}"
        );
        assert!(
            out.contains("+ let x = 2;"),
            "unrelated line raises literally, got: {out}"
        );
    }

    #[test]
    fn header_helpers_behave() {
        assert_eq!(
            header_name_delta("fn foo(x: i32) {", "fn bar(x: u32) {"),
            Some(("foo".to_string(), "bar".to_string()))
        );
        assert!(header_name_delta("fn foo(x: i32) {", "fn foo(x: u32) {").is_none());
        assert!(header_name_delta("    fn foo() {", "    fn bar() {").is_none());
        assert!(header_name_delta("struct Old {", "struct New {").is_none());
        assert!(slots_equal_except_name(
            "fn foo(x: i32) {",
            "fn bar(x: i32) {"
        ));
        assert!(!slots_equal_except_name(
            "fn foo(x: i32) {",
            "fn bar(x: u32) {"
        ));
        assert!(is_call_site_of("    foo(1);", "    bar(1);", "foo", "bar"));
        assert!(is_call_site_of(
            "    let y = foo(2);",
            "    let y = bar(2);",
            "foo",
            "bar"
        ));
        assert!(!is_call_site_of("    foo(1);", "    baz(1);", "foo", "bar"));
        assert!(!is_call_site_of(
            "    s.foo();",
            "    s.bar();",
            "foo",
            "bar"
        ));
        assert!(is_callee_only("    foo(1);", "    bar(1);", "foo", "bar"));
        assert!(!is_callee_only(
            "    foo(1);",
            "    bar(1, 0);",
            "foo",
            "bar"
        ));
        assert_eq!(with_body_hole("fn foo(x: i32) {"), "fn foo(x: i32) { ... }");
        // `split_item` still sees the joint def as a plain `fn` item.
        assert!(split_item("fn foo(x: i32) {").is_some());
    }
}
