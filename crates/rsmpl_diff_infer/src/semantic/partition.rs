//! WI-8 pair partition section driver (WI-15 D5 split).
//!
//! Split from `chain.rs` under the 2000-line gate (D5): pure move, no
//! behavior change. `chain.rs` re-exports the driver so call sites and
//! tests keep resolving with no changes.

/// WI-8 pair partition: a whole balanced section with N ≥ 2 pairs whose
/// pair chain declined compresses pair by pair when every pair is a
/// single-pair item rename (including the plain-`fn` arm, whose
/// `{`/no-return/no-`where` gate applies per pair). All-or-nothing:
/// one unclaimable pair vetoes the whole section (a mixed hunk stays
/// ordinary whole, never half-semantic). Duplicate minus lines veto
/// (identical pairs belong to the verbatim sweep); duplicate emitted
/// minus patterns veto too (terse `fn` shapes drop params, so distinct
/// raw lines can share one pattern and would overlap-match). Each pair
/// also clears pre-image totality (`template_pair_holds` at n = 1, the
/// `slot_holds` posture lifted to single pairs): an untouched twin
/// anywhere in the file vetoes, `None` pre-image keeps the
/// section-visible legacy allow. Emission is one file
/// section with one hunk per pair (per-pair covering ranges from that
/// pair's changed line, positional `rN` rules — the shared counter is
/// already positional per WI-1e diet; distinct deltas never group so
/// `group` stays `""`). Section-level T375 lives with the caller (same
/// shape as the `run_pair_chain` block, rename prelude included).
/// Detector `"item_rename"` so cross-file same-delta grouping keeps
/// working with no `group.rs` change.
pub(crate) fn try_partition_section(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if minuses.len() < 2 || minuses.len() != pluses.len() {
        return None;
    }
    // Pair-to-site mapping for the per-pair covering ranges below.
    if changed.len() != minuses.len() {
        return None;
    }
    // Identical pairs belong to the verbatim sweep (and their patterns
    // would overlap-match under `# allow=many`).
    let mut ordered: Vec<&&str> = minuses.iter().collect();
    ordered.sort();
    ordered.dedup();
    if ordered.len() != minuses.len() {
        return None;
    }
    let checkpoint = used_rules.len();
    let mut bodies: Vec<String> = Vec::new();
    // WI-19c: per-pair gate — `try_item_rename` (now indent-tolerant)
    // first, then the self-ty `impl` single. Impl pairs carry their own
    // scratch-apply totality (a same-name self-ty elsewhere is orphan
    // evidence, never a veto), so they skip the sweep-count check below
    // that item pairs keep.
    let mut sweep_check: Vec<bool> = Vec::new();
    for i in 0..minuses.len() {
        let probe_before = used_rules.len();
        if let Some(full) = super::try_item_rename(
            minuses[i],
            pluses[i],
            path,
            file_old,
            &changed[i..i + 1],
            used_rules,
        ) {
            let Some((_, body)) = full.split_once('\n') else {
                used_rules.truncate(checkpoint);
                return None;
            };
            bodies.push(body.to_string());
            sweep_check.push(true);
            continue;
        }
        used_rules.truncate(probe_before);
        match super::try_impl_header_single(
            &minuses[i..i + 1],
            &pluses[i..i + 1],
            path,
            file_old,
            &changed[i..i + 1],
            used_rules,
        ) {
            super::prov::Counted::Fire((full, _)) => {
                let Some((_, body)) = full.split_once('\n') else {
                    used_rules.truncate(checkpoint);
                    return None;
                };
                bodies.push(body.to_string());
                sweep_check.push(false);
            }
            _ => {
                used_rules.truncate(checkpoint);
                return None;
            }
        }
    }
    // One minus pattern per pair; identical emitted patterns would
    // overlap-match under `# allow=many` (terse `fn` shapes drop params,
    // so distinct raw lines can share one pattern — those classes stay
    // with the slot/verbatim sweeps).
    let mut pats: Vec<(&str, &str)> = Vec::new();
    for body in &bodies {
        let mut minus = None;
        let mut plus = None;
        for line in body.lines() {
            if let Some(pat) = line.strip_prefix("- ") {
                if minus.is_none() {
                    minus = Some(pat);
                }
            } else if let Some(pat) = line.strip_prefix("+ ") {
                if plus.is_none() {
                    plus = Some(pat);
                }
            }
        }
        let (Some(minus), Some(plus)) = (minus, plus) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        pats.push((minus, plus));
    }
    let mut by_pat: Vec<&&str> = pats.iter().map(|(m, _)| m).collect();
    by_pat.sort();
    by_pat.dedup();
    if by_pat.len() != pats.len() {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Pre-image totality per item pair (the `slot_holds` posture lifted to
    // single pairs): each minus pattern must match exactly its own site.
    // An untouched twin anywhere in the file vetoes; `None` pre-image
    // keeps the section-visible legacy allow (provenance marks it).
    // WI-19c impl pairs skip this: their scratch-apply probe already
    // passed, and a same-name self-ty elsewhere is orphan evidence.
    for (idx, &(minus, plus)) in pats.iter().enumerate() {
        if !sweep_check[idx] {
            continue;
        }
        if !crate::totality::template_pair_holds(path, file_old, minus, plus, 1) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    let mut out = crate::git_header(path);
    out.push('\n');
    for body in &bodies {
        out.push_str(body);
    }
    Some((out, "item_rename"))
}
