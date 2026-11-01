//! WI-4c workspace fn-joint candidate (Joint-A cross-file only).
//!
//! Split from `fn_joint` (D5: no file exceeds 2000 lines): the
//! whole-patch workspace classifier. Single-file groups stay in
//! `fn_joint`; this module owns the cross-file Joint-A fire (def in one
//! file, callee-only calls across files, one follow hunk at the def)
//! plus the claim-then-veto suppression that keeps vetoed shapes wholly
//! ordinary instead of fragmenting into an ordinary def plus a
//! rename-claiming call sweep.

use std::collections::{HashMap, HashSet};

use super::fn_joint::{
    header_fire_gates, header_name_delta, is_call_site_of, is_callee_only, slots_equal_except_name,
    Fire, FlatPair, HunkPairs, Joint,
};
use super::item_intent::{count_kind_defs, Pair};

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
        None => return Workspace::None,
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
    // WI-4f-next2 soundness: leftovers are sound only in the def file
    // (same-file ordinary+follow composition, single-file precedent).
    // A leftover in any call file suppresses the whole group — the
    // engine follow does not rewrite a call file the patch also edits
    // ordinarily, so the call rewrites would be silently lost (probe
    // of the emission alone cannot see the composed loss).
    if leftover.keys().any(|s| *s != ds) {
        return Workspace::Suppress(involved_paths, old);
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
    // Joint-A only: callee-only calls + header slot gates over the
    // claimed subset. Arg-add, attr, and hetero shapes suppress (B
    // cross-file stays ordinary).
    let claimed_idx: HashSet<usize> = hunk_members
        .iter()
        .filter(|(k, _)| claimed_hunks.contains(*k))
        .flat_map(|(_, v)| v.iter().copied())
        .collect();
    let callee_only = flat.iter().enumerate().all(|(i, fp)| {
        if i == def_idx || !claimed_idx.contains(&i) {
            return true;
        }
        is_callee_only(&fp.minus, &fp.plus, &old, &new)
    });
    if !callee_only {
        return Workspace::Suppress(involved_paths, old);
    }
    if !header_fire_gates(&flat[def_idx].minus, &flat[def_idx].plus) {
        return Workspace::Suppress(involved_paths, old);
    }
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
            joint: Joint::A,
        }),
        def_path,
    )
}
