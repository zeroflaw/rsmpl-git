//! WI-4c workspace fn-joint candidate (Joint-A cross-file only) plus
//! WI-4f Joint-B cross-file firing (def hunk at the def file +
//! call-template hunk at the single call file, one shared rule).
//!
//! Split from `fn_joint` (D5: no file exceeds 2000 lines): the
//! whole-patch workspace classifier. Single-file groups stay in
//! `fn_joint`; this module owns the cross-file Joint-A fire (def in one
//! file, callee-only calls across files, one follow hunk at the def),
//! the cross-file Joint-B1/B2 fires (rename+arg-add / rename+attr with
//! calls in exactly one other file, two-section shared-rule emission),
//! plus the claim-then-veto suppression that keeps vetoed shapes wholly
//! ordinary instead of fragmenting into an ordinary def plus a
//! rename-claiming call sweep.

use std::collections::{HashMap, HashSet};

use super::fn_joint::{is_call_site_of, is_callee_only};
use super::fn_joint_b2::{attr_def_shape, attr_joint_fire, probe_attr_joint_ws};
use super::fn_joint_emit::{call_hunk_text, def_hunk_text};
use super::fn_joint_sig::{
    arg_add_template, header_argadd_gates, header_fire_gates, header_name_delta,
    slots_equal_except_name,
};
use super::fn_joint_types::{Fire, FlatPair, HunkPairs, Joint};
use super::item_intent::{count_kind_defs, probe_reproduces, Pair};

/// Workspace verdict for the whole patch (WI-4c: Joint-A only).
pub(crate) enum Workspace {
    /// No workspace joint shape (pure rename, no/twin defs, no calls,
    /// missing pre-image, …) — silent, no suppression.
    None,
    /// Joint shape present but not fireable as Joint-A (arg-add, attr,
    /// hetero, gates, probe/T375 upstream) — suppress these paths.
    Suppress(Vec<String>, String),
    /// Fireable Joint-A payload plus the def path (claims span files;
    /// `claimed_secs` holds every involved global section idx).
    Ready(Box<Fire>, String),
}

/// Classify the whole patch as one workspace group (WI-4c). Only
/// positional balanced pairs participate: the def is the single global
/// `fn`-header name delta in a single-pair hunk; calls are call-callee
/// sites of old→new in any file. Uninvolved sections (no def/call pairs)
/// pass through untouched. Any veto from the uniqueness guard on
/// suppresses with the involved-path set (the fragmented ordinary-def +
/// rename-sweep fallback must not stand in for it); Joint-B shapes
/// (arg-add/attr/unbalanced) suppress the same way — B cross-file stays
/// ordinary in this slice, never a fragment.
pub(crate) fn joint_workspace_candidate(
    parsed: &[(usize, String, Vec<HunkPairs>)],
    pre: &HashMap<String, String>,
    hunk_texts: &HashMap<(usize, usize), String>,
) -> Workspace {
    // Path-aware flat pairs plus per-section unbalanced flags.
    struct WPair {
        sec: usize,
        path: String,
        hunk: usize,
        minus: String,
        plus: String,
        site: usize,
    }
    let mut flat: Vec<WPair> = Vec::new();
    // WI-4f-next2: unbalanced hunks become leftovers unless they carry a
    // header delta (ambiguity → suppress). Tracked per (sec, hunk).
    let mut unbalanced_hunks: Vec<(usize, usize)> = Vec::new();
    for (sec, path, hunks) in parsed {
        for (h, hp) in hunks.iter().enumerate() {
            if !hp.balanced {
                unbalanced_hunks.push((*sec, h));
                continue;
            }
            for ((m, p), s) in hp.minuses.iter().zip(hp.pluses.iter()).zip(hp.sites.iter()) {
                flat.push(WPair {
                    sec: *sec,
                    path: path.clone(),
                    hunk: h,
                    minus: m.clone(),
                    plus: p.clone(),
                    site: *s,
                });
            }
        }
    }
    // Exactly one global `fn`-header name delta (column-0 only).
    let mut def: Option<usize> = None;
    for (i, fp) in flat.iter().enumerate() {
        if header_name_delta(&fp.minus, &fp.plus).is_some() {
            if def.is_some() {
                return Workspace::None;
            }
            def = Some(i);
        }
    }
    let def_idx = match def {
        Some(d) => d,
        // WI-4f B2 has no positional def: exactly one attr-shaped
        // unbalanced def hunk plus balanced calls elsewhere composes to
        // the shared-rule two-hunk form (single-file B2 precedent).
        None => return workspace_b2_candidate(parsed, pre, hunk_texts),
    };
    let (old, new) = match header_name_delta(&flat[def_idx].minus, &flat[def_idx].plus) {
        Some(v) => v,
        None => return Workspace::None,
    };
    // Single-file groups already own single-file shapes: a workspace
    // fire needs calls in at least two files or a def file different
    // from a call file (else the single-file loop already fired or
    // suppressed on exactly this shape — running again would double-claim
    // or re-suppress identically).
    let def_path = flat[def_idx].path.clone();
    let mut call_files: HashSet<&str> = HashSet::new();
    let mut call_count = 0usize;
    for (i, fp) in flat.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        if is_call_site_of(&fp.minus, &fp.plus, &old, &new) {
            call_count += 1;
            call_files.insert(fp.path.as_str());
        }
    }
    if call_count == 0 {
        return Workspace::None;
    }
    let cross_file = call_files.len() > 1 || call_files.iter().any(|p| *p != def_path);
    if !cross_file {
        return Workspace::None;
    }
    // Pure renames belong to item_intent — never a joint candidate.
    // The def hunk holding exactly the header pair is the only Joint-A
    // shape; extra pairs in the same hunk are body/attr lines.
    let (dh, ds) = (flat[def_idx].hunk, flat[def_idx].sec);
    let def_hunk_len = flat
        .iter()
        .filter(|fp| fp.sec == ds && fp.hunk == dh)
        .count();
    let def_single = def_hunk_len == 1;
    if def_single && slots_equal_except_name(&flat[def_idx].minus, &flat[def_idx].plus) {
        return Workspace::None;
    }
    // Unique-definition guard across the involved pre-images (silent).
    // The guard counts across every pair's file; build the pair list
    // from the workspace flat (def file + every call file involved).
    let guard_pairs: Vec<Pair> = flat
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            *i == def_idx || is_call_site_of(&flat[*i].minus, &flat[*i].plus, &old, &new)
        })
        .map(|(_, fp)| Pair {
            sec: 0,
            path: fp.path.clone(),
            minus: String::new(),
            plus: String::new(),
            site: 0,
        })
        .collect();
    if count_kind_defs(pre, &guard_pairs, "fn", &old) != 1 {
        return Workspace::None;
    }
    for fp in &flat {
        if !pre.contains_key(&fp.path) {
            return Workspace::None;
        }
    }
    // Involved sections: secs holding the def or at least one call site.
    let mut involved: HashSet<usize> = HashSet::new();
    involved.insert(ds);
    for fp in &flat {
        if is_call_site_of(&fp.minus, &fp.plus, &old, &new) {
            involved.insert(fp.sec);
        }
    }
    let involved_paths: Vec<String> = parsed
        .iter()
        .filter(|(sec, _, _)| involved.contains(sec))
        .map(|(_, p, _)| p.clone())
        .collect();
    // Any veto from here on suppresses the involved paths.
    if !def_single {
        return Workspace::Suppress(involved_paths, old);
    }
    // WI-4f-next2 hunk-level partition: the def hunk is claimed; other
    // hunks in involved sections are claimed iff every pair is a call
    // site, left over iff no pair is a call site, and suppress on a
    // mixed hunk (half a hunk cannot be claimed). Unbalanced hunks in
    // involved sections are leftovers unless they carry a header delta.
    let mut hunk_members: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, fp) in flat.iter().enumerate() {
        hunk_members.entry((fp.sec, fp.hunk)).or_default().push(i);
    }
    let mut claimed_hunks: HashSet<(usize, usize)> = HashSet::new();
    claimed_hunks.insert((ds, dh));
    let mut leftover: HashMap<usize, Vec<String>> = HashMap::new();
    for ((sec, h), members) in &hunk_members {
        if *sec == ds && *h == dh {
            continue;
        }
        if !involved.contains(sec) {
            continue;
        }
        let n_call = members
            .iter()
            .filter(|i| is_call_site_of(&flat[**i].minus, &flat[**i].plus, &old, &new))
            .count();
        if n_call == members.len() {
            claimed_hunks.insert((*sec, *h));
        } else if n_call == 0 {
            if let Some(t) = hunk_texts.get(&(*sec, *h)) {
                leftover.entry(*sec).or_default().push(t.clone());
            }
        } else {
            return Workspace::Suppress(involved_paths, old);
        }
    }
    for (sec, h) in &unbalanced_hunks {
        if !involved.contains(sec) {
            continue;
        }
        let mut has_header = false;
        if let Some(hp) = parsed
            .iter()
            .find(|(s, _, _)| s == sec)
            .and_then(|(_, _, hs)| hs.get(*h))
        {
            for m in &hp.minuses {
                for p in &hp.pluses {
                    if header_name_delta(m, p).is_some() {
                        has_header = true;
                    }
                }
            }
        }
        if has_header {
            return Workspace::Suppress(involved_paths, old);
        }
        if let Some(t) = hunk_texts.get(&(*sec, *h)) {
            leftover.entry(*sec).or_default().push(t.clone());
        }
    }
    // No claimable call hunk beyond the def → nothing to join (raw
    // calls existed only inside mixed/leftover hunks → suppress so the
    // fallback stays wholly ordinary).
    if claimed_hunks.len() < 2 {
        let raw_calls = flat
            .iter()
            .enumerate()
            .filter(|(i, fp)| *i != def_idx && is_call_site_of(&fp.minus, &fp.plus, &old, &new))
            .count();
        if raw_calls > 0 {
            return Workspace::Suppress(involved_paths, old);
        }
        return Workspace::None;
    }
    // WI-4f B1 shares the Joint-A partition above (def hunk claimed,
    // other hunks claimed iff all-call, leftovers allowed in BOTH files
    // — no follow rewrites the call file, so an explicit call-template
    // hunk composes soundly beside ordinary leftovers there; the
    // call-file-leftover suppress law below stays Joint-A-only).
    let claimed_idx: HashSet<usize> = hunk_members
        .iter()
        .filter(|(k, _)| claimed_hunks.contains(*k))
        .flat_map(|(_, v)| v.iter().copied())
        .collect();
    // Claimed pairs as `FlatPair`s (def position included — the B1
    // template skips it, like the single-file classifier).
    let mut claimed: Vec<FlatPair> = Vec::new();
    let mut def_pos: Option<usize> = None;
    for (i, fp) in flat.iter().enumerate() {
        if !claimed_idx.contains(&i) {
            continue;
        }
        if i == def_idx {
            def_pos = Some(claimed.len());
        }
        claimed.push(FlatPair {
            sec: fp.sec,
            hunk: fp.hunk,
            minus: fp.minus.clone(),
            plus: fp.plus.clone(),
            site: fp.site,
        });
    }
    let Some(def_pos) = def_pos else {
        return Workspace::Suppress(involved_paths, old);
    };
    let sec_path: HashMap<usize, &str> = parsed.iter().map(|(s, p, _)| (*s, p.as_str())).collect();
    let joint = if claimed
        .iter()
        .enumerate()
        .all(|(i, fp)| i == def_pos || is_callee_only(&fp.minus, &fp.plus, &old, &new))
    {
        if !header_fire_gates(&flat[def_idx].minus, &flat[def_idx].plus) {
            return Workspace::Suppress(involved_paths, old);
        }
        // Joint-A only beyond here: a leftover in any CALL file
        // suppresses (the engine follow does not rewrite a call file
        // the patch also edits ordinarily — WI-4f-next2 soundness).
        if leftover.keys().any(|s| *s != ds) {
            return Workspace::Suppress(involved_paths, old);
        }
        Joint::A
    } else {
        // WI-4f B1: the claimed calls add the same concrete trailing
        // args or retype the trailing args uniformly (template law);
        // the header evolves params only with the return preserved —
        // the single-file B1 laws lifted workspace-wide. Any hetero
        // shape suppresses (B stays ordinary, never a fragment).
        if !header_argadd_gates(&flat[def_idx].minus, &flat[def_idx].plus) {
            return Workspace::Suppress(involved_paths, old);
        }
        let (call_minus, call_plus) = match arg_add_template(&claimed, def_pos, &old, &new) {
            Some(t) => t,
            None => return Workspace::Suppress(involved_paths, old),
        };
        // Exactly one call file beyond the def file, holding every
        // claimed call (mixed def-file calls and multi-call-file groups
        // stay suppressed-ordinary: their three-hunk composition is
        // deferred). N=1 never pays the second hunk (single-file law).
        let mut call_files: HashSet<&str> = HashSet::new();
        for (i, fp) in claimed.iter().enumerate() {
            if i == def_pos {
                continue;
            }
            call_files.insert(sec_path.get(&fp.sec).copied().unwrap_or(""));
        }
        if call_files.len() != 1 || call_files.contains(def_path.as_str()) {
            return Workspace::Suppress(involved_paths, old);
        }
        if claimed.len() < 3 {
            return Workspace::Suppress(involved_paths, old);
        }
        Joint::B1 {
            call_minus,
            call_plus,
        }
    };
    let claims: Vec<FlatPair> = flat
        .into_iter()
        .enumerate()
        .filter(|(i, _)| claimed_idx.contains(i))
        .map(|(_, fp)| FlatPair {
            sec: fp.sec,
            hunk: fp.hunk,
            minus: fp.minus,
            plus: fp.plus,
            site: fp.site,
        })
        .collect();
    let claimed_secs: Vec<usize> = claimed_hunks
        .iter()
        .map(|(s, _)| *s)
        .collect::<HashSet<usize>>()
        .into_iter()
        .collect();
    let claimed_hunk_bytes: usize = claimed_hunks
        .iter()
        .map(|k| hunk_texts.get(k).map(|t| t.len()).unwrap_or(0))
        .sum();
    Workspace::Ready(
        Box::new(Fire {
            old,
            new,
            def_minus: claims
                .iter()
                .find(|c| c.sec == ds && c.hunk == dh)
                .map(|c| c.minus.clone())
                .unwrap_or_default(),
            def_plus: claims
                .iter()
                .find(|c| c.sec == ds && c.hunk == dh)
                .map(|c| c.plus.clone())
                .unwrap_or_default(),
            claims,
            claimed_secs,
            leftover,
            claimed_hunk_bytes,
            split_extra: 0,
            joint,
        }),
        def_path,
    )
}

/// WI-4f workspace Joint-B2 candidate (no positional def): exactly one
/// attr-shaped unbalanced def hunk (rename header + one `#[…]` line) in
/// the def file plus uniform callee-only calls in exactly one other
/// file composes to the shared-rule two-hunk form (single-file B2
/// precedent). Any veto suppresses the involved paths so the fallback
/// stays wholly ordinary (claim-then-veto, F4b posture).
pub(crate) fn workspace_b2_candidate(
    parsed: &[(usize, String, Vec<HunkPairs>)],
    pre: &HashMap<String, String>,
    hunk_texts: &HashMap<(usize, usize), String>,
) -> Workspace {
    // Balanced positional pairs; a positional `fn`-header name delta
    // anywhere belongs to the A/B1 path (unreachable here — the caller
    // only delegates when no positional def exists — but the gate keeps
    // the two classifiers disjoint by construction, not by call order).
    struct BPair {
        sec: usize,
        path: String,
        hunk: usize,
        minus: String,
        plus: String,
        site: usize,
    }
    let mut flat: Vec<BPair> = Vec::new();
    // Unbalanced hunks as `(sec, hunk, minuses, pluses)` (4-tuple —
    // paths resolve via `parsed`, keeping the clippy type-complexity
    // lint quiet).
    let mut unbalanced: Vec<(usize, usize, Vec<String>, Vec<String>)> = Vec::new();
    for (sec, path, hunks) in parsed {
        for (h, hp) in hunks.iter().enumerate() {
            if !hp.balanced {
                unbalanced.push((*sec, h, hp.minuses.clone(), hp.pluses.clone()));
                continue;
            }
            for ((m, p), s) in hp.minuses.iter().zip(hp.pluses.iter()).zip(hp.sites.iter()) {
                if header_name_delta(m, p).is_some() {
                    return Workspace::None;
                }
                flat.push(BPair {
                    sec: *sec,
                    path: path.clone(),
                    hunk: h,
                    minus: m.clone(),
                    plus: p.clone(),
                    site: *s,
                });
            }
        }
    }
    // Exactly one attr-shaped def hunk (rename header + one `#[…]`
    // line); zero is not a joint shape, two is ambiguity (silent, the
    // single-file twin-def precedent).
    let sec_path: HashMap<usize, &str> = parsed.iter().map(|(s, p, _)| (*s, p.as_str())).collect();
    let mut attr_hunks: Vec<(usize, usize)> = Vec::new();
    for (sec, h, m, p) in &unbalanced {
        if attr_def_shape(m, p).is_some() {
            attr_hunks.push((*sec, *h));
        }
    }
    if attr_hunks.is_empty() {
        return Workspace::None;
    }
    if attr_hunks.len() != 1 {
        return Workspace::None;
    }
    let (def_sec, def_h) = attr_hunks[0];
    let Some(def_path) = sec_path.get(&def_sec).map(|s| s.to_string()) else {
        return Workspace::None;
    };
    let (def_minus, def_plus) = {
        let (_, _, m, p) = unbalanced
            .iter()
            .find(|(s, h, _, _)| *s == def_sec && *h == def_h)
            .expect("attr hunk re-found");
        match attr_def_shape(m, p) {
            Some((_, _, dm, dp)) => (dm, dp),
            None => return Workspace::None,
        }
    };
    let (old, new) = match header_name_delta(&def_minus, &def_plus) {
        Some(v) => v,
        None => return Workspace::None,
    };
    // At least one balanced call-callee pair of old→new must exist.
    let mut call_count = 0usize;
    for fp in &flat {
        if is_call_site_of(&fp.minus, &fp.plus, &old, &new) {
            call_count += 1;
        }
    }
    if call_count == 0 {
        return Workspace::None;
    }
    // Unique-definition guard over the def file plus every call file
    // (silent, the Joint-A posture).
    let mut guard_paths: HashSet<&str> = HashSet::new();
    guard_paths.insert(def_path.as_str());
    for fp in &flat {
        if is_call_site_of(&fp.minus, &fp.plus, &old, &new) {
            guard_paths.insert(fp.path.as_str());
        }
    }
    let guard_pairs: Vec<Pair> = guard_paths
        .iter()
        .map(|p| Pair {
            sec: 0,
            path: p.to_string(),
            minus: String::new(),
            plus: String::new(),
            site: 0,
        })
        .collect();
    if count_kind_defs(pre, &guard_pairs, "fn", &old) != 1 {
        return Workspace::None;
    }
    for p in &guard_paths {
        if !pre.contains_key(*p) {
            return Workspace::None;
        }
    }
    // Involved sections: the def hunk's section plus every section
    // holding at least one call site.
    let mut involved: HashSet<usize> = HashSet::new();
    involved.insert(def_sec);
    for fp in &flat {
        if is_call_site_of(&fp.minus, &fp.plus, &old, &new) {
            involved.insert(fp.sec);
        }
    }
    let involved_paths: Vec<String> = parsed
        .iter()
        .filter(|(sec, _, _)| involved.contains(sec))
        .map(|(_, p, _)| p.clone())
        .collect();
    // Partition balanced hunks: all-call hunks claimed, no-call hunks
    // leftover, mixed hunks suppress. Non-attr unbalanced hunks with a
    // header delta suppress; without one they are leftovers.
    let mut hunk_members: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, fp) in flat.iter().enumerate() {
        hunk_members.entry((fp.sec, fp.hunk)).or_default().push(i);
    }
    let mut claimed_hunks: HashSet<(usize, usize)> = HashSet::new();
    let mut leftover: HashMap<usize, Vec<String>> = HashMap::new();
    for ((sec, h), members) in &hunk_members {
        if !involved.contains(sec) {
            continue;
        }
        let n_call = members
            .iter()
            .filter(|i| is_call_site_of(&flat[**i].minus, &flat[**i].plus, &old, &new))
            .count();
        if n_call == members.len() {
            claimed_hunks.insert((*sec, *h));
        } else if n_call == 0 {
            if let Some(t) = hunk_texts.get(&(*sec, *h)) {
                leftover.entry(*sec).or_default().push(t.clone());
            }
        } else {
            return Workspace::Suppress(involved_paths, old);
        }
    }
    for (sec, h, m, p) in &unbalanced {
        if !involved.contains(sec) {
            continue;
        }
        if attr_def_shape(m, p).is_some() {
            continue;
        }
        let mut has_header = false;
        for mm in m {
            for pp in p {
                if header_name_delta(mm, pp).is_some() {
                    has_header = true;
                }
            }
        }
        if has_header {
            return Workspace::Suppress(involved_paths, old);
        }
        if let Some(t) = hunk_texts.get(&(*sec, *h)) {
            leftover.entry(*sec).or_default().push(t.clone());
        }
    }
    // Exactly one call file beyond the def file (mixed def-file calls
    // and multi-call-file groups stay suppressed-ordinary: their
    // three-hunk composition is deferred). N=1 never pays the second
    // hunk (single-file B2 law).
    let claimed_idx: HashSet<usize> = hunk_members
        .iter()
        .filter(|(k, _)| claimed_hunks.contains(*k))
        .flat_map(|(_, v)| v.iter().copied())
        .collect();
    let mut claimed_flat: Vec<FlatPair> = Vec::new();
    for (i, fp) in flat.iter().enumerate() {
        if !claimed_idx.contains(&i) {
            continue;
        }
        claimed_flat.push(FlatPair {
            sec: fp.sec,
            hunk: fp.hunk,
            minus: fp.minus.clone(),
            plus: fp.plus.clone(),
            site: fp.site,
        });
    }
    claimed_flat.sort_by_key(|fp| (fp.sec, fp.hunk, fp.site));
    let mut call_files: HashSet<&str> = HashSet::new();
    for fp in &claimed_flat {
        call_files.insert(sec_path.get(&fp.sec).copied().unwrap_or(""));
    }
    if call_files.len() != 1 || call_files.contains(def_path.as_str()) {
        return Workspace::Suppress(involved_paths, old);
    }
    if claimed_flat.len() < 2 {
        return Workspace::Suppress(involved_paths, old);
    }
    let single_attr = vec![unbalanced
        .iter()
        .find(|(s, h, _, _)| *s == def_sec && *h == def_h)
        .map(|(s, h, m, p)| (*s, *h, m.clone(), p.clone()))
        .expect("attr hunk re-found")];
    let Some(mut joint_fire) =
        attr_joint_fire(&def_path, pre, &single_attr, &claimed_flat, &old, &new)
    else {
        return Workspace::Suppress(involved_paths, old);
    };
    let mut claimed_secs: Vec<usize> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    for fp in &claimed_flat {
        if seen.insert(fp.sec) {
            claimed_secs.push(fp.sec);
        }
    }
    if seen.insert(def_sec) {
        claimed_secs.push(def_sec);
    }
    joint_fire.claimed_secs = claimed_secs;
    joint_fire.leftover = leftover;
    joint_fire.claimed_hunk_bytes = claimed_flat
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
    Workspace::Ready(Box::new(joint_fire), def_path)
}

/// WI-4f two-section rebuild for a workspace Joint-B fire: the def hunk
/// lands at the def file, the call-template hunk at the single call
/// file, both under one rule. The scratch-apply probe (D13) runs over
/// the composed two-section emission with per-file claims (B1 via
/// `probe_reproduces`, B2 via `probe_attr_joint_ws`); T375 compares
/// emission-hunk bytes against claimed-hunk bytes (hunk-level law, the
/// shared header stays). `None` fills `suppressed` with the involved
/// paths (wholly-ordinary fallback) and truncates the reserved rule.
///
/// Inputs ride one struct (clippy 7-arg lint).
pub(crate) struct WsBRebuild<'a> {
    pub sections: &'a [String],
    pub parsed: &'a [(usize, String, Vec<HunkPairs>)],
    pub preserved: &'a HashMap<usize, (String, Vec<String>)>,
    pub native_lens: &'a HashMap<usize, usize>,
    pub pre: &'a HashMap<String, String>,
    pub fire: &'a Fire,
    pub def_path: &'a str,
    pub used_rules: &'a mut Vec<String>,
    pub checkpoint: usize,
    pub suppressed: &'a mut HashSet<(String, String)>,
}

/// Two-section rebuild; see [`WsBRebuild`].
pub(crate) fn rebuild_workspace_b(input: WsBRebuild<'_>) -> Option<(Vec<String>, String, String)> {
    let WsBRebuild {
        sections,
        parsed,
        preserved,
        native_lens,
        pre,
        fire,
        def_path,
        used_rules,
        checkpoint,
        suppressed,
    } = input;
    let sec_path: HashMap<usize, &str> = parsed.iter().map(|(s, p, _)| (*s, p.as_str())).collect();
    let fail = |suppressed: &mut HashSet<(String, String)>, used_rules: &mut Vec<String>| {
        used_rules.truncate(checkpoint);
        for s in &fire.claimed_secs {
            if let Some(p) = sec_path.get(s) {
                suppressed.insert((p.to_string(), fire.old.clone()));
            }
        }
    };
    // The def file contributes exactly the def hunk's section; every
    // other claimed section shares the single call file (classifier
    // gate — re-verified here, never trusted blindly).
    let Some(def_sec) = fire
        .claimed_secs
        .iter()
        .find(|s| sec_path.get(s).copied().unwrap_or("") == def_path)
        .copied()
    else {
        fail(suppressed, used_rules);
        return None;
    };
    let call_secs: Vec<usize> = fire
        .claimed_secs
        .iter()
        .filter(|s| **s != def_sec)
        .copied()
        .collect();
    let mut call_paths: HashSet<&str> = HashSet::new();
    for s in &call_secs {
        call_paths.insert(sec_path.get(s).copied().unwrap_or(""));
    }
    if call_paths.len() != 1 || call_secs.is_empty() {
        fail(suppressed, used_rules);
        return None;
    }
    let call_path = call_paths.into_iter().next().expect("exactly one");
    if call_path == def_path {
        fail(suppressed, used_rules);
        return None;
    }
    let rule = super::unique_rule(used_rules, &format!("rename_{}_to_{}", fire.old, fire.new));
    let def_hunk = def_hunk_text(&rule, fire);
    let Some(call_hunk) = call_hunk_text(&rule, fire) else {
        fail(suppressed, used_rules);
        return None;
    };
    let mut def_section = crate::git_header(def_path);
    def_section.push('\n');
    def_section.push_str(&def_hunk);
    let mut call_section = crate::git_header(call_path);
    call_section.push('\n');
    call_section.push_str(&call_hunk);
    let emission = format!("{def_section}{call_section}");
    // Scratch-apply probe over the composed emission (D13): per-file
    // claims, every other line byte-identical.
    let pairs: Vec<Pair> = fire
        .claims
        .iter()
        .map(|c| Pair {
            sec: 0,
            path: if c.sec == def_sec {
                def_path.to_string()
            } else {
                call_path.to_string()
            },
            minus: c.minus.clone(),
            plus: c.plus.clone(),
            site: c.site,
        })
        .collect();
    let probed = match &fire.joint {
        Joint::A | Joint::Body { .. } => {
            fail(suppressed, used_rules);
            return None;
        }
        Joint::B1 { .. } => probe_reproduces(pre, &emission, &pairs),
        Joint::B2 { .. } => {
            let calls: Vec<(usize, String)> = fire
                .claims
                .iter()
                .map(|c| (c.site, c.plus.clone()))
                .collect();
            probe_attr_joint_ws(pre, &emission, def_path, call_path, fire, &calls)
        }
    };
    // WI-4e native-view bytes when pure, hunk-level bytes when leftovers
    // are nonempty (shared header stays in place).
    let (claimed_bytes, emission_bytes) = if fire.leftover.is_empty() {
        let claimed: usize = fire
            .claimed_secs
            .iter()
            .map(|s| native_lens.get(s).copied().unwrap_or(0))
            .sum();
        (claimed, def_section.len() + call_section.len())
    } else {
        let eh: usize = super::emission_hunks(&emission)
            .iter()
            .map(|h| h.len())
            .sum();
        (fire.claimed_hunk_bytes, eh)
    };
    if !(probed && emission_bytes < claimed_bytes) {
        fail(suppressed, used_rules);
        return None;
    }
    let claimed: HashSet<usize> = fire.claimed_secs.iter().copied().collect();
    let def_new_hunks = super::emission_hunks(&def_section);
    let call_new_hunks = super::emission_hunks(&call_section);
    let mut out: Vec<String> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        if !claimed.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        let (prelude, keep) = preserved.get(&i).cloned().unwrap_or_default();
        let rest = fire.leftover.get(&i).cloned().unwrap_or_default();
        let is_def = i == def_sec;
        if keep.is_empty() && rest.is_empty() {
            out.push(if is_def {
                def_section.clone()
            } else {
                call_section.clone()
            });
            continue;
        }
        let mut rebuilt = prelude;
        for h in &keep {
            rebuilt.push_str(h);
        }
        for h in if is_def {
            &def_new_hunks
        } else {
            &call_new_hunks
        } {
            rebuilt.push_str(h);
        }
        for h in &rest {
            rebuilt.push_str(h);
        }
        out.push(rebuilt);
    }
    Some((out, def_path.to_string(), rule))
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, split_semantic_native};
    use super::*;

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

    fn apply_to_tree(patch: &str, pre: &HashMap<String, String>) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        files
    }

    /// Both native sections are fully claimed (no leftovers): the
    /// rebuild splices whole emitted sections.
    fn b1_sections() -> (Vec<String>, HashMap<String, String>) {
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections =
            vec![
            section("a.rs", "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n"),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            ),
        ];
        (sections, pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]))
    }

    #[test]
    fn ws_b1_fires_two_sections() {
        // WI-4f: rename+arg-add with the def in one file and uniform
        // arg-add calls in exactly one other file raises as a
        // shared-rule two-section emission (never a fragment).
        let (sections, pre) = b1_sections();
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B1 must fire");
        assert!(out.contains("diff --git a.rs"), "got:\n{out}");
        assert!(out.contains("diff --git b.rs"), "got:\n{out}");
        assert!(
            !out.contains("# follow=uses"),
            "B1 rides no follow, got:\n{out}"
        );
        assert!(out.contains("- fn foo(x: i32) { ... }"), "got:\n{out}");
        assert!(
            out.contains("+ fn bar(x: i32, y: i32) { ... }"),
            "got:\n{out}"
        );
        assert!(out.contains("- foo(E0)"), "got:\n{out}");
        assert!(out.contains("+ bar(E0, 0)"), "got:\n{out}");
        // One rule across both sections.
        let mut rules: Vec<&str> = Vec::new();
        for line in out.lines() {
            if let Some(at) = line.find("rule=") {
                rules.push(&line[at..]);
            }
        }
        assert_eq!(rules.len(), 2, "two hunks share one rule, got:\n{out}");
        assert_eq!(rules[0], rules[1], "got:\n{out}");
        validated_live(&out, &pre);
        let files = apply_to_tree(&out, &pre);
        assert!(
            files["a.rs"].contains("fn bar(x: i32, y: i32) {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(!files["a.rs"].contains("foo("), "got: {}", files["a.rs"]);
        assert!(
            files["b.rs"].contains("bar(1, 0);"),
            "got: {}",
            files["b.rs"]
        );
        assert!(
            files["b.rs"].contains("bar(2, 0);"),
            "got: {}",
            files["b.rs"]
        );
        assert!(!files["b.rs"].contains("foo("), "got: {}", files["b.rs"]);
    }

    #[test]
    fn ws_b2_fires_two_sections() {
        // WI-4f: rename+attr-add with uniform callee-only calls in one
        // other file raises as the shared-rule two-section form.
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[inline]\n+fn bar(x: i32) {\n",
            ),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B2 must fire");
        assert!(out.contains("diff --git a.rs"), "got:\n{out}");
        assert!(out.contains("diff --git b.rs"), "got:\n{out}");
        assert!(
            out.contains("- #[inline]") || out.contains("+ #[inline]"),
            "got:\n{out}"
        );
        assert!(out.contains("- foo(E0)"), "got:\n{out}");
        assert!(out.contains("+ bar(E0)"), "got:\n{out}");
        let mut rules: Vec<&str> = Vec::new();
        for line in out.lines() {
            if let Some(at) = line.find("rule=") {
                rules.push(&line[at..]);
            }
        }
        assert_eq!(rules.len(), 2, "two hunks share one rule, got:\n{out}");
        assert_eq!(rules[0], rules[1], "got:\n{out}");
        validated_live(&out, &pre);
        let files = apply_to_tree(&out, &pre);
        assert!(
            files["a.rs"].contains("#[inline]"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("fn bar(x: i32) {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(files["b.rs"].contains("bar(1);"), "got: {}", files["b.rs"]);
        assert!(!files["b.rs"].contains("foo("), "got: {}", files["b.rs"]);
    }

    #[test]
    fn ws_b2_remove_fires_two_sections() {
        // B2 attr-REMOVE cross-file: the `head_site - 1` splice path,
        // proved against the def file while calls live elsewhere.
        let a_pre = "#[inline]\nfn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,2 +1,1 @@\n-#[inline]\n-fn foo(x: i32) {\n+fn bar(x: i32) {\n",
            ),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B2 remove must fire");
        assert!(out.contains("diff --git a.rs"), "got:\n{out}");
        assert!(out.contains("diff --git b.rs"), "got:\n{out}");
        let mut rules: Vec<&str> = Vec::new();
        for line in out.lines() {
            if let Some(at) = line.find("rule=") {
                rules.push(&line[at..]);
            }
        }
        assert_eq!(rules.len(), 2, "two hunks share one rule, got:\n{out}");
        assert_eq!(rules[0], rules[1], "got:\n{out}");
        validated_live(&out, &pre);
        let files = apply_to_tree(&out, &pre);
        assert!(
            !files["a.rs"].contains("#[inline]"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("fn bar(x: i32) {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(!files["b.rs"].contains("foo("), "got: {}", files["b.rs"]);
    }

    #[test]
    fn ws_b1_def_file_leftover_composes() {
        // REVIEW gap 2 twin: an unrelated hunk in the DEF file composes
        // the same way (N=3 for the hunk-level T375 law — see the
        // call-file twin above for the N=2 parity note).
        let a_pre = "fn foo(x: i32) {\n    x\n}\nfn z() {\n    let q = 1;\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,1 +5,1 @@\n-    let q = 1;\n+    let q = 2;\n",
            ),
            section(
                "b.rs",
                "@@ -2,3 +2,3 @@\n-    foo(1);\n-    foo(2);\n-    foo(3);\n+    bar(1, 0);\n+    bar(2, 0);\n+    bar(3, 0);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B1 must fire");
        assert!(
            out.contains("let q = 2;"),
            "leftover stays verbatim, got:\n{out}"
        );
        validated_live(&out, &pre);
        let files = apply_to_tree(&out, &pre);
        assert!(
            files["a.rs"].contains("fn bar(x: i32, y: i32) {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("let q = 2;"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["b.rs"].contains("bar(3, 0);"),
            "got: {}",
            files["b.rs"]
        );
        assert!(!files["b.rs"].contains("foo("), "got: {}", files["b.rs"]);
    }

    #[test]
    fn ws_b_mixed_def_calls_suppress() {
        // WI-4f deferred: a call in the def file too (three-hunk
        // composition) suppresses the workspace group — wholly ordinary,
        // never a fragment sweep over the def delta.
        let a_pre = "fn foo(x: i32) {\n    x\n}\nfn n() {\n    foo(9);\n    foo(8);\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -4,2 +4,2 @@\n-    foo(9);\n-    foo(8);\n+    bar(9, 0);\n+    bar(8, 0);\n",
            ),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            out.as_ref().is_none_or(|s| !s.contains("rule=")),
            "mixed def-file calls suppress, got: {out:?}"
        );
    }

    #[test]
    fn ws_b_multi_call_files_suppress() {
        // WI-4f deferred: calls spread over two call files suppress.
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let c_pre = "fn n() {\n    foo(3);\n    foo(4);\n}\n";
        let sections =
            vec![
            section("a.rs", "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n"),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            ),
            section(
                "c.rs",
                "@@ -2,2 +2,2 @@\n-    foo(3);\n-    foo(4);\n+    bar(3, 0);\n+    bar(4, 0);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre), ("c.rs", c_pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            out.as_ref().is_none_or(|s| !s.contains("rule=")),
            "multi-call-file group suppresses, got: {out:?}"
        );
    }

    #[test]
    fn ws_b_hetero_suffix_suppress() {
        // Hetero arg-add tails suppress without a fragment: no
        // rename-claiming call sweep may stand in for the joint.
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections =
            vec![
            section("a.rs", "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n"),
            section(
                "b.rs",
                "@@ -2,2 +2,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 1);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            out.as_ref().is_none_or(|s| !s.contains("rule=")),
            "hetero suffix suppresses, got: {out:?}"
        );
    }

    #[test]
    fn ws_b1_lone_call_suppresses() {
        // N=1 never pays the second hunk (single-file B1 law lifted
        // workspace-wide): wholly ordinary, no fragment.
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n",
            ),
            section("b.rs", "@@ -2,1 +2,1 @@\n-    foo(1);\n+    bar(1, 0);\n"),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            out.as_ref().is_none_or(|s| !s.contains("rule=")),
            "lone call suppresses, got: {out:?}"
        );
    }

    #[test]
    fn ws_b1_call_file_leftover_composes() {
        // Joint-B (unlike Joint-A) allows ordinary leftovers in the CALL
        // file: no follow rewrites that file, so the explicit call hunk
        // composes soundly beside them. N=3: at N=2 the `{ ... }` def
        // hunk outweighs the two call lines under the hunk-level T375
        // law (single-file B1 parity — pinned by
        // `ws_b1_lone_call_suppresses` for N=1 and the hetero suite).
        let a_pre = "fn foo(x: i32) {\n    x\n}\n";
        let b_pre = "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n    let q = 1;\n}\n";
        let sections = vec![
            section("a.rs", "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n"),
            section(
                "b.rs",
                "@@ -2,3 +2,3 @@\n-    foo(1);\n-    foo(2);\n-    foo(3);\n+    bar(1, 0);\n+    bar(2, 0);\n+    bar(3, 0);\n@@ -5,1 +5,1 @@\n-    let q = 1;\n+    let q = 2;\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B1 must fire");
        // The def section is fully claimed (collapsed header); the call
        // section keeps its native prelude around the spliced hunks
        // (hybrid-section precedent, same as single-file B1).
        assert!(out.contains("diff --git a.rs"), "got:\n{out}");
        assert!(out.contains("a/b.rs"), "got:\n{out}");
        // WI-6 slice 1: the leftover raises as its own literal verbatim
        // single (unique site, byte-probe verified) — the call section
        // is fully semantic, so there is no ordinary remainder view.
        assert!(
            out.contains("- let q = 1;"),
            "leftover raises literally, got:\n{out}"
        );
        assert!(
            out.contains("+ let q = 2;"),
            "leftover raises literally, got:\n{out}"
        );
        validated_live(&out, &pre);
        let files = apply_to_tree(&out, &pre);
        assert!(
            files["b.rs"].contains("bar(1, 0);"),
            "got: {}",
            files["b.rs"]
        );
        assert!(
            files["b.rs"].contains("bar(3, 0);"),
            "got: {}",
            files["b.rs"]
        );
        assert!(
            files["b.rs"].contains("let q = 2;"),
            "got: {}",
            files["b.rs"]
        );
        // Leftover survival is in-place: with the WI-6-slice-1 single
        // the call section is fully semantic (no ordinary remainder),
        // so there is no native view beside the semantic hunks.
        let (_, _, native_view) =
            split_semantic_native(&out[out.find("a/b.rs").expect("call section")..]);
        assert!(
            native_view.is_none(),
            "fully semantic call section keeps no native view, got:\n{out}"
        );
    }
}
