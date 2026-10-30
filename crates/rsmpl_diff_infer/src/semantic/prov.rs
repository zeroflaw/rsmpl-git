//! WI-2d honest per-hunk provenance for `diffrs --format json` (SPEC D.2).
//!
//! Which detector fired, over which old-side lines, under which totality
//! evidence, and how many bytes the hunk saved vs the ordinary text it
//! replaced. Split from `mod.rs` under the 2000-line gate (D5).
//!
//! Honesty rules (WI-2d): single-hunk firings keep the exact section
//! saving (the one hunk is the whole win). Multi-hunk rewrites measure
//! each hunk — the ordinary native hunk bytes its old-range covers
//! minus its own emitted block bytes — never an equal split of a
//! section total, so those entries need not sum to the section delta.
//! `sites` stays covering and says so via `sites_exact`. `proven` is
//! true exactly when `totality` is `"pre-image"`.

/// WI-2d per-hunk provenance for `diffrs --format json` (SPEC D.2): which
/// detector fired, over which old-side lines, under which totality
/// evidence, and how many bytes the hunk saved vs the ordinary text it
/// replaced. `sites` are the section's changed old-side lines covered by
/// the hunk's emitted range (a superset only when leftover ordinary
/// hunks share the span; `sites_exact` says whether the list is proven
/// exact). Anchor-only move pastes report `[]` (exactly: there is
/// nothing to cover). `saved_bytes` is the exact section saving for
/// single-hunk firings and the measured per-hunk win (covered ordinary
/// hunk bytes minus emitted block bytes, saturating; anchor-only
/// pastes report `0`) for multi-hunk rewrites. `proven` is true exactly
/// when `totality` is `"pre-image"`.
#[derive(Debug, Clone, Default)]
pub struct HunkProvenance {
    pub rule: String,
    pub file: String,
    pub detector: &'static str,
    pub sites: Vec<usize>,
    pub sites_exact: bool,
    pub totality: &'static str,
    pub proven: bool,
    pub saved_bytes: usize,
    /// WI-7a cross-file intent label: `""` when ungrouped, else
    /// `rename_Old_to_New` shared with every hunk of the same
    /// single-delta intent across files. Provenance-only (patch bytes
    /// stay canonical `r1…`); WI-7b owns the shared-`rule=` re-emission.
    pub group: String,
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

/// One `@@` hunk block: the `rule=` name when the header carries one
/// (`None` for ordinary native hunks), the old-side range, and the
/// block's exact byte length (header line plus body lines).
struct HunkBlock {
    rule: Option<String>,
    old_start: usize,
    old_len: usize,
    bytes: usize,
}

/// WI-2d: parse an `@@` header line into (`rule=`, old-start, old-len).
/// Handles one-sided RSmPL headers (`@@ -min,span @@ rule=rN`),
/// two-sided move headers (`@@ -a,b +c,d @@ rule=rN`), and plain native
/// headers (`@@ -a,b +c,d @@`, no `rule=`).
fn parse_hunk_header(line: &str) -> (Option<String>, usize, usize) {
    let rule = line
        .split("rule=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .filter(|r| !r.is_empty())
        .map(str::to_string);
    let ranges = line
        .strip_prefix("@@")
        .unwrap_or("")
        .split("@@")
        .next()
        .unwrap_or("")
        .trim();
    let old = ranges.split_whitespace().next().unwrap_or("");
    let old = old.strip_prefix('-').unwrap_or(old);
    let (start, len) = match old.split_once(',') {
        Some((s, l)) => (s.parse().unwrap_or(0), l.parse().unwrap_or(0)),
        None => (old.parse().unwrap_or(0), 1),
    };
    (rule, start, len)
}

/// WI-2d: split emitted or native section text into hunk blocks.
/// Prelude lines before the first `@@` are ignored (they belong to no
/// hunk). Byte lengths use `split_inclusive` so they are exact.
fn hunk_blocks(text: &str) -> Vec<HunkBlock> {
    let mut out = Vec::new();
    let mut cur: Option<HunkBlock> = None;
    for line in text.split_inclusive('\n') {
        if line.starts_with("@@") {
            if let Some(b) = cur.take() {
                out.push(b);
            }
            let (rule, old_start, old_len) = parse_hunk_header(line);
            cur = Some(HunkBlock {
                rule,
                old_start,
                old_len,
                bytes: line.len(),
            });
        } else if let Some(b) = cur.as_mut() {
            b.bytes += line.len();
        }
    }
    if let Some(b) = cur.take() {
        out.push(b);
    }
    out
}

/// WI-2d: measured per-rule win — the ordinary native hunk bytes whose
/// old-range the rule's emitted block(s) cover, minus the emitted
/// bytes (saturating). A file-scope sweep dummy (`@@ -1,1 @@`)
/// covers every native hunk by construction; ranged hunks cover the
/// native hunks their old-range overlaps. A rule with no emitted
/// block, or one covering nothing (anchor-only move pastes), saves 0.
fn measured_saved(
    emitted: &[HunkBlock],
    native: &[HunkBlock],
    rule: &str,
    is_dummy: bool,
) -> usize {
    let mut emitted_bytes = 0usize;
    let mut covered_native: Vec<bool> = vec![false; native.len()];
    let mut saw_block = false;
    for e in emitted.iter().filter(|b| b.rule.as_deref() == Some(rule)) {
        saw_block = true;
        emitted_bytes += e.bytes;
        if e.old_start == 0 {
            continue;
        }
        let end = e.old_start.saturating_add(e.old_len.saturating_sub(1));
        for (i, n) in native.iter().enumerate() {
            if is_dummy {
                covered_native[i] = true;
                continue;
            }
            if n.old_start == 0 {
                continue;
            }
            let n_end = n.old_start.saturating_add(n.old_len.saturating_sub(1));
            if e.old_start <= n_end && n.old_start <= end {
                covered_native[i] = true;
            }
        }
    }
    if !saw_block {
        return 0;
    }
    let covered: usize = native
        .iter()
        .enumerate()
        .filter(|(i, _)| covered_native[*i])
        .map(|(_, n)| n.bytes)
        .sum();
    covered.saturating_sub(emitted_bytes)
}

/// WI-2: parse `(rule, old_start, old_len)` triples from the RSmPL hunk
/// headers of one emitted section or body. Handles one-sided RSmPL
/// headers (`@@ -min,span @@ rule=rN`) and two-sided move headers
/// (`@@ -a,b +c,d @@ rule=rN`); ordinary hunks (no `rule=`) are skipped.
pub(crate) fn emitted_hunk_ranges(emitted: &str) -> Vec<(String, usize, usize)> {
    hunk_blocks(emitted)
        .into_iter()
        .filter_map(|b| b.rule.map(|r| (r, b.old_start, b.old_len)))
        .collect()
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
        // File-scope sweep dummy (`@@ -1,1 @@` + `# sweep`, SPEC 17.3):
        // the emitted range covers the whole file by construction, so
        // every changed line is a site. A ranged hunk always spans its
        // own sites, so an exact (1,1) range over changed lines outside
        // [1,1] can only be a dummy — never a ranged under-report
        // (WI-2 sweep triage: verbatim r7 cited zero sites).
        Some((_, 1, 1)) => changed.to_vec(),
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

/// WI-2d: commit entries for a composed section return. Only rules present
/// in the final text get entries (an earlier stage's hunks may have been
/// consumed by a later stage). Single-hunk firings keep the exact
/// section saving (the one hunk is the whole win, prelude stripping
/// included — the T375 guard compared exactly this). Multi-hunk
/// rewrites measure each hunk against the original section's native
/// hunks (never an equal split), so those entries need not sum to the
/// section delta. `sites_exact`/`proven` state the evidence grade per
/// entry either way.
pub(crate) fn commit_prov(
    prov: &mut Vec<HunkProvenance>,
    final_text: &str,
    original: &str,
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
    let single_total = if present.len() == 1 {
        Some(original.len().saturating_sub(final_text.len()))
    } else {
        None
    };
    // Multi-hunk measurement inputs, parsed lazily (single-hunk
    // firings never need them).
    let emitted = hunk_blocks(final_text);
    let native = hunk_blocks(original);
    let proven = file_old.is_some();
    for (rule, detector) in present.into_iter() {
        let parsed = ranges.iter().any(|(hr, _, _)| hr == &rule);
        let is_dummy = ranges
            .iter()
            .any(|(hr, s, l)| hr == &rule && *s == 1 && *l == 1);
        let sites = covered_sites(&ranges, &rule, changed);
        // Exact when the emitted range parsed and either pre-image
        // totality backs the coverage or there is nothing covered to
        // overstate (anchor-only move pastes report `[]`).
        let sites_exact = parsed && (proven || sites.is_empty());
        let saved_bytes = match single_total {
            Some(total) => total,
            None => measured_saved(&emitted, &native, &rule, is_dummy),
        };
        prov.push(HunkProvenance {
            rule: rule.clone(),
            file: path.to_string(),
            detector,
            sites,
            sites_exact,
            totality: totality_label(file_old),
            proven,
            saved_bytes,
            group: String::new(),
        });
    }
}

/// WI-2: provenance for the cross-section move pre-pass. Sections are
/// matched to their originals by `diff --git` path; per-section deltas
/// are measured per hunk against that section's original. Cut hunks cite real
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
    let mut orig_secs: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    let mut orig_changed: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for sec in original {
        let header = sec.lines().next().unwrap_or("");
        if let Some(rest) = header.strip_prefix("diff --git ") {
            if let Some(path) = crate::b_side_path(rest) {
                orig_secs.insert(path.clone(), sec.as_str());
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
        let empty = String::new();
        let orig = orig_secs.get(&path).copied().unwrap_or(&empty);
        let no_changed: Vec<usize> = Vec::new();
        let changed = orig_changed.get(&path).unwrap_or(&no_changed);
        commit_prov(
            prov,
            sec,
            orig,
            &path,
            &detector_of,
            changed,
            pre.get(&path).map(String::as_str),
        );
    }
}

/// WI-2d: push one provenance entry per new rule of a section-level
/// detector firing. `original` is the ordinary section (or hunk) the
/// emission beat (T375 guard): single-hunk firings keep its exact
/// saving, multi-hunk rewrites measure each hunk against it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_section_prov(
    prov: &mut Vec<HunkProvenance>,
    emitted: &str,
    original: &str,
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
        original,
        path,
        &detector_of,
        changed,
        file_old,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_dummy_covers_whole_changed() {
        // WI-2 sweep triage: a file-scope sweep dummy (`@@ -1,1 @@`)
        // cites every changed line (verbatim r7 cited none).
        let ranges = vec![("r7".to_string(), 1, 1)];
        assert_eq!(
            covered_sites(&ranges, "r7", &[297, 342, 396]),
            vec![297, 342, 396]
        );
        // Ranged hunks still intersect.
        let ranges = vec![("r1".to_string(), 290, 10)];
        assert_eq!(covered_sites(&ranges, "r1", &[297, 342, 396]), vec![297]);
        // Missing headers fall back to whole changed.
        let ranges: Vec<(String, usize, usize)> = vec![];
        assert_eq!(covered_sites(&ranges, "r9", &[5]), vec![5]);
    }

    #[test]
    fn measured_saved_is_per_hunk_not_split() {
        // WI-2d: two emitted hunks over disjoint native hunks of
        // different sizes measure their own wins — unequal, so never
        // an equal split (which would differ by at most one).
        let original = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,2 +2,2 @@\n-    old(a1);\n-    old(a2);\n+    new(a1);\n+    new(a2);\n@@ -9,5 +10,5 @@\n-    old(b1);\n-    old(b2);\n-    old(b3);\n-    old(b4);\n-    old(b5);\n+    new(b1);\n+    new(b2);\n+    new(b3);\n+    new(b4);\n+    new(b5);\n";
        let final_text = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,2 +2,2 @@ rule=r1\n-    old(E);\n+    new(E);\n@@ -9,5 +10,5 @@ rule=r2\n-    old(E);\n+    new(E);\n";
        let emitted = hunk_blocks(final_text);
        let native = hunk_blocks(original);
        let s1 = measured_saved(&emitted, &native, "r1", false);
        let s2 = measured_saved(&emitted, &native, "r2", false);
        assert!(s1 > 0 && s2 > 0, "s1={s1} s2={s2}");
        // Each hunk's win equals its own delta; the bigger span wins
        // bigger (an equal split would report near-equal shares).
        assert_eq!(s1, native[0].bytes.saturating_sub(emitted[0].bytes));
        assert_eq!(s2, native[1].bytes.saturating_sub(emitted[1].bytes));
        assert!(s2 > s1 + 1, "s1={s1} s2={s2}");
    }

    #[test]
    fn anchor_paste_saves_zero_and_stays_exact() {
        // WI-2d: a rule with no emitted block (or covering nothing)
        // saves 0 rather than a split share.
        let emitted =
            hunk_blocks("diff --git a/b.rs b/b.rs\n@@ -1,0 +2,7 @@ rule=r2\n+fn big() {}\n");
        let native = hunk_blocks("diff --git a/b.rs b/b.rs\n@@ -1,0 +2,7 @@\n+fn big() {}\n");
        assert_eq!(measured_saved(&emitted, &native, "r9", false), 0);
    }

    #[test]
    fn header_parse_handles_all_three_shapes() {
        assert_eq!(
            parse_hunk_header("@@ -1,1 @@ rule=r1\n"),
            (Some("r1".to_string()), 1, 1)
        );
        assert_eq!(
            parse_hunk_header("@@ -2,7 +1,0 @@ rule=r3\n"),
            (Some("r3".to_string()), 2, 7)
        );
        assert_eq!(parse_hunk_header("@@ -2,7 +1,0 @@\n"), (None, 2, 7));
    }
}
