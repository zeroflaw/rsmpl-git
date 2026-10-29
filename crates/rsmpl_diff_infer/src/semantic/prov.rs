//! WI-2 per-hunk provenance for `diffrs --format json` (SPEC D.2).
//!
//! Which detector fired, over which old-side lines, under which totality
//! evidence, and how many bytes the hunk saved vs the ordinary text it
//! replaced. Split from `mod.rs` under the 2000-line gate (D5).

/// WI-2 per-hunk provenance for `diffrs --format json` (SPEC D.2): which
/// detector fired, over which old-side lines, under which totality
/// evidence, and how many bytes the hunk saved vs the ordinary text it
/// replaced. `sites` are the section's changed old-side lines covered by
/// the hunk's emitted range (a superset only when leftover ordinary
/// hunks share the span); anchor-only hunks (move pastes) report `[]`.
/// `saved_bytes` is the exact section delta for single-hunk firings and
/// the per-hunk `ordinary_len - body_len` delta inside the per-hunk
/// fallback; multi-hunk section rewrites split the section total equally
/// (first hunk takes the remainder) so entries sum to the section delta.
#[derive(Debug, Clone, Default)]
pub struct HunkProvenance {
    pub rule: String,
    pub file: String,
    pub detector: &'static str,
    pub sites: Vec<usize>,
    pub totality: &'static str,
    pub saved_bytes: usize,
}

/// WI-2 totality label (SPEC D.2): `"pre-image"` when the section had
/// pre-image text to count against, else `"section-visible"`.
pub(crate) fn totality_label(file_old: Option<&str>) -> &'static str {
    if file_old.is_some() {
        "pre-image"
    } else {
        "section-visible"
    }
}

/// WI-2: parse `(rule, old_start, old_len)` triples from the RSmPL hunk
/// headers of one emitted section or body. Handles one-sided RSmPL
/// headers (`@@ -min,span @@ rule=rN`) and two-sided move headers
/// (`@@ -a,b +c,d @@ rule=rN`); ordinary hunks (no `rule=`) are skipped.
pub(crate) fn emitted_hunk_ranges(emitted: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    for line in emitted.lines() {
        if !line.starts_with("@@") {
            continue;
        }
        let Some(rule) = line.split("rule=").nth(1) else {
            continue;
        };
        let rule: String = rule.split_whitespace().next().unwrap_or("").to_string();
        if rule.is_empty() {
            continue;
        }
        let rest = line.strip_prefix("@@").unwrap_or("");
        let ranges = rest.split("@@").next().unwrap_or("").trim();
        let old = ranges.split_whitespace().next().unwrap_or("");
        let old = old.strip_prefix('-').unwrap_or(old);
        let (start, len) = match old.split_once(',') {
            Some((s, l)) => (s.parse().unwrap_or(0), l.parse().unwrap_or(0)),
            None => (old.parse().unwrap_or(0), 1),
        };
        out.push((rule, start, len));
    }
    out
}

/// WI-2: sites for one emitted hunk — the section's changed old-side
/// lines covered by the hunk's emitted old-side range (`changed` whole
/// when the hunk is a sweep dummy or its header is missing). Covering,
/// never understating: the emission range always spans the detector's
/// own sites, so every true site survives the intersection.
pub(crate) fn covered_sites(
    ranges: &[(String, usize, usize)],
    rule: &str,
    changed: &[usize],
) -> Vec<usize> {
    match ranges.iter().find(|(r, _, _)| r == rule) {
        Some((_, start, len)) if *start >= 1 => {
            let end = start.saturating_add(len.saturating_sub(1));
            changed
                .iter()
                .copied()
                .filter(|l| *start <= *l && *l <= end)
                .collect()
        }
        _ => changed.to_vec(),
    }
}

/// WI-2: commit entries for a composed section return. Only rules present
/// in the final text get entries (an earlier stage's hunks may have been
/// consumed by a later stage); the exact section total is split equally
/// across the present rules (first takes the remainder) so entries sum to
/// the section delta. Detector identity travels per rule in `detector_of`.
pub(crate) fn commit_prov(
    prov: &mut Vec<HunkProvenance>,
    final_text: &str,
    section_len: usize,
    path: &str,
    detector_of: &[(String, &'static str)],
    changed: &[usize],
    file_old: Option<&str>,
) {
    let ranges = emitted_hunk_ranges(final_text);
    let present: Vec<(String, &'static str)> = detector_of
        .iter()
        .filter(|(r, _)| ranges.iter().any(|(hr, _, _)| hr == r))
        .cloned()
        .collect();
    if present.is_empty() {
        return;
    }
    let total_saved = section_len.saturating_sub(final_text.len());
    let n = present.len();
    for (i, (rule, detector)) in present.into_iter().enumerate() {
        let saved = if i == 0 {
            total_saved - (total_saved / n) * (n - 1)
        } else {
            total_saved / n
        };
        prov.push(HunkProvenance {
            rule: rule.clone(),
            file: path.to_string(),
            detector,
            sites: covered_sites(&ranges, &rule, changed),
            totality: totality_label(file_old),
            saved_bytes: saved,
        });
    }
}

/// WI-2: provenance for the cross-section move pre-pass. Sections are
/// matched to their originals by `diff --git` path; per-section deltas
/// are split across that section's new move rules. Cut hunks cite real
/// ranges (sites via parse-back); paste anchors report `[]`.
pub(crate) fn push_move_prov(
    prov: &mut Vec<HunkProvenance>,
    original: &[String],
    rewritten: &[String],
    pre: &std::collections::HashMap<String, String>,
    rules_before: usize,
    used_rules: &[String],
) {
    let new_rules = &used_rules[rules_before..];
    if new_rules.is_empty() {
        return;
    }
    let mut orig_lens: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut orig_changed: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for sec in original {
        let header = sec.lines().next().unwrap_or("");
        if let Some(rest) = header.strip_prefix("diff --git ") {
            if let Some(path) = crate::b_side_path(rest) {
                orig_lens.insert(path.clone(), sec.len());
                orig_changed.insert(path, crate::totality::changed_old_lines(sec));
            }
        }
    }
    for sec in rewritten {
        let header = sec.lines().next().unwrap_or("");
        let path = header
            .strip_prefix("diff --git ")
            .and_then(crate::b_side_path)
            .unwrap_or_default();
        let ranges = emitted_hunk_ranges(sec);
        let section_rules: Vec<&String> = new_rules
            .iter()
            .filter(|r| ranges.iter().any(|(hr, _, _)| hr == *r))
            .collect();
        if section_rules.is_empty() {
            continue;
        }
        let detector_of: Vec<(String, &'static str)> = section_rules
            .iter()
            .map(|r| ((*r).clone(), "move"))
            .collect();
        let orig_len = orig_lens.get(&path).copied().unwrap_or(sec.len());
        let empty: Vec<usize> = Vec::new();
        let changed = orig_changed.get(&path).unwrap_or(&empty);
        commit_prov(
            prov,
            sec,
            orig_len,
            &path,
            &detector_of,
            changed,
            pre.get(&path).map(String::as_str),
        );
    }
}

/// WI-2: push one provenance entry per new rule of a section-level
/// detector firing. `emitted` is the returned semantic section,
/// `section_len` the ordinary section length it beat (T375 guard), so
/// `section_len - emitted.len()` is the exact section saving, split
/// equally across the firing's new rules (single-hunk firings — the
/// common case — keep the exact total on their one entry).
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_section_prov(
    prov: &mut Vec<HunkProvenance>,
    emitted: &str,
    section_len: usize,
    path: &str,
    detector: &'static str,
    changed: &[usize],
    file_old: Option<&str>,
    rules_before: usize,
    used_rules: &[String],
) {
    let detector_of: Vec<(String, &'static str)> = used_rules[rules_before..]
        .iter()
        .map(|r| (r.clone(), detector))
        .collect();
    commit_prov(
        prov,
        emitted,
        section_len,
        path,
        &detector_of,
        changed,
        file_old,
    );
}
