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
/// when `totality` is `"pre-image"`. WI-15c: wholly-ordinary sections
/// carry `detector: "ordinary"` with an empty rule name and the
/// chain-level `declined` list (which detector families were tried and
/// why they declined); semantic hunks never carry `declined` (a firing
/// section explains itself).
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
    /// WI-15c declined-detector evidence for wholly-ordinary sections:
    /// one entry per attempted detector family that declined, in chain
    /// order (deduped). Empty on every semantic hunk.
    pub declined: Vec<HunkDeclined>,
}

/// WI-15c one declined detector attempt: the family name the chain
/// observed (pair-chain families individually; composite drivers by
/// driver name; section gates by gate name) plus the chain-level reason
/// (`no-match` shape decline, `byte-guard` strict-T375 veto after a
/// candidate, `precision-guard` precision-allowance veto after a
/// candidate, or a section-gate reason like `crlf-content`,
/// `unbalanced-hunks`, `stranded-joint`). Detector-internal guard
/// vetoes surface as `no-match` — reasons are chain-level, never
/// detector-inferred (SPEC D.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HunkDeclined {
    pub detector: String,
    pub reason: String,
}

/// WI-15c chain attempt recorder: `Some` passes through untouched;
/// `None` records `(detector, "no-match")` and passes the `None` on.
/// The name lives at the call site, so the list cannot drift from the
/// chain order (no separate registry to sync).
pub(crate) fn attempt<T>(
    declined: &mut Vec<HunkDeclined>,
    detector: &'static str,
    r: Option<T>,
) -> Option<T> {
    if r.is_none() {
        declined.push(HunkDeclined {
            detector: detector.to_string(),
            reason: "no-match".to_string(),
        });
    }
    r
}

/// WI-15c chain guard-veto recorder: the detector fired but the
/// section/hunk guard priced it out. Call at the veto branch only.
pub(crate) fn declined_guard(
    declined: &mut Vec<HunkDeclined>,
    detector: &str,
    reason: &'static str,
) {
    declined.push(HunkDeclined {
        detector: detector.to_string(),
        reason: reason.to_string(),
    });
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

/// WI-15 partial: true when `emitted` carries a file-scope directive for
/// `rule` (`# sweep` or `# scope=file`, SPEC 17.3 / SPEC 9.3). The `(1,1)`
/// old range is a dummy iff file scope is present — a ranged single-line
/// claim at line 1 shares the `(1,1)` shape but carries no file-scope
/// directive, so range alone cannot tell them apart. `# allow=many`
/// alone is never file scope.
pub(crate) fn rule_is_file_scope(emitted: &str, rule: &str) -> bool {
    let mut cur_rule: Option<String> = None;
    let mut cur_scope = false;
    let mut scoped: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    for line in emitted.lines() {
        if line.starts_with("@@") {
            if let Some(r) = cur_rule.take() {
                scoped.insert(r, cur_scope);
            }
            cur_scope = false;
            cur_rule = line
                .split("rule=")
                .nth(1)
                .and_then(|r| r.split_whitespace().next())
                .filter(|r| !r.is_empty())
                .map(str::to_string);
        } else if cur_rule.is_some() {
            let t = line.trim();
            if t == "# sweep"
                || t.starts_with("# sweep ")
                || t.starts_with("# sweep\t")
                || t == "# scope=file"
                || t.starts_with("# scope=file ")
                || t.starts_with("# scope=file\t")
            {
                cur_scope = true;
            }
        }
    }
    if let Some(r) = cur_rule.take() {
        scoped.insert(r, cur_scope);
    }
    scoped.get(rule).copied().unwrap_or(false)
}
/// WI-2: sites for one emitted hunk — the section's changed old-side
/// lines covered by the hunk's emitted old-side range (`changed` whole
/// when the hunk is a file-scope sweep dummy or its header is missing).
/// Covering, never understating: the emission range always spans the
/// detector's own sites, so every true site survives the intersection.
/// A `(1,1)` range covers everything only when `is_sweep` proves the
/// block carries `# sweep` / `# scope=file` (SPEC 17.3); a ranged
/// single-line claim at line 1 shares the shape and intersects.
pub(crate) fn covered_sites(
    ranges: &[(String, usize, usize)],
    rule: &str,
    changed: &[usize],
    is_sweep: bool,
) -> Vec<usize> {
    match ranges.iter().find(|(r, _, _)| r == rule) {
        // File-scope sweep dummy (`@@ -1,1 @@` + file scope, SPEC 17.3):
        // the emitted range covers the whole file by construction, so
        // every changed line is a site (WI-2 sweep triage: verbatim r7
        // cited zero sites).
        Some((_, 1, 1)) if is_sweep => changed.to_vec(),
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
        let is_sweep = rule_is_file_scope(final_text, &rule);
        let is_dummy = ranges
            .iter()
            .any(|(hr, s, l)| hr == &rule && *s == 1 && *l == 1)
            && is_sweep;
        let sites = covered_sites(&ranges, &rule, changed, is_sweep);
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
            declined: Vec::new(),
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

/// WI-15c: one provenance entry for a wholly-ordinary section. The
/// caller passes the chain-attempt `declined` list (empty means nothing
/// was tried — headers-only or non-`.rs` — so no entry is pushed).
/// `changed` are the section's changed old-side lines (exact,
/// header-derived, hence `sites_exact: true` either way); `saved_bytes`
/// is `0` (nothing won); `totality`/`proven` follow the pre-image rule.
/// Entries dedupe preserving first-seen chain order (one hunk's singles
/// declining the same family five times reports it once). Sections with
/// no native (`rule=`-free) hunk get no entry: pre-pass emissions (move
/// / joint intents) are already semantic, and attributing chain declines
/// to them would mislabel a compressed section as ordinary.
pub(crate) fn push_ordinary_prov(
    prov: &mut Vec<HunkProvenance>,
    path: &str,
    declined: &[HunkDeclined],
    changed: &[usize],
    file_old: Option<&str>,
    section: &str,
) {
    if declined.is_empty() {
        return;
    }
    if !has_native_hunk(section) {
        return;
    }
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for d in declined {
        if seen.insert((d.detector.clone(), d.reason.clone())) {
            deduped.push(d.clone());
        }
    }
    prov.push(HunkProvenance {
        rule: String::new(),
        file: path.to_string(),
        detector: "ordinary",
        sites: changed.to_vec(),
        sites_exact: true,
        totality: totality_label(file_old),
        proven: file_old.is_some(),
        saved_bytes: 0,
        group: String::new(),
        declined: deduped,
    });
}

/// WI-15c: true when `section` holds at least one native hunk (an `@@`
/// header without `rule=`). Pre-pass emissions carry only `rule=` hunks;
/// they never re-enter the chain, so chain declines misattribute them.
pub(crate) fn has_native_hunk(section: &str) -> bool {
    section
        .lines()
        .any(|l| l.starts_with("@@") && !l.contains("rule="))
}

/// WI-15c: per-section ordinary-entry driver for the `mod.rs` loop.
/// Extracts the b-side path, skips headers that do not parse, and
/// delegates to [`push_ordinary_prov`] (which owns the empty-list and
/// no-native-hunk silences). Keeps the `mod.rs` loop delta to the call.
pub(crate) fn push_ordinary_for_section(
    prov: &mut Vec<HunkProvenance>,
    section: &str,
    declined: &[HunkDeclined],
    pre: &std::collections::HashMap<String, String>,
) {
    if declined.is_empty() {
        return;
    }
    let header = match section.lines().next() {
        Some(h) => h,
        None => return,
    };
    let rest = match header.strip_prefix("diff --git ") {
        Some(r) => r,
        None => return,
    };
    let path = match crate::b_side_path(rest) {
        Some(p) => p,
        None => return,
    };
    push_ordinary_prov(
        prov,
        &path,
        declined,
        &crate::totality::changed_old_lines(section),
        pre.get(&path).map(String::as_str),
        section,
    );
}

/// WI-15 (partially-compressed sections): one remainder ordinary entry
/// for a mixed emission (semantic `rule=` hunks plus verbatim ordinary
/// remainder hunks in one file section — per-hunk fallback mixes, WI-19
/// partial-partition mixes, subhunk mixes). `sites` are the original
/// section's changed old-side lines minus the union of the already-
/// committed semantic prov sites for the emitted rules (run-scoped, so
/// context-carrying bodies with `...` spans never over-claim remainder
/// lines; anchor-only move pastes cover `[]`, so they never shrink the
/// remainder; rules with no committed entry fall back to `covered_sites`
/// per rule). Empty remainder, wholly-semantic emissions (no native
/// hunk), rule-free emissions (unreachable on the `Some` path —
/// guarded), or empty `declined` (nothing tried) all stay silent — the
/// same silences as [`push_ordinary_prov`]. Chain-level `declined`
/// (deduped) is carried as-is; inner subhunk/verbatim classification
/// probes stay on a throwaway by design (the whole-hunk chain attempt
/// already records per-family evidence).
pub(crate) fn push_partial_ordinary_prov(
    prov: &mut Vec<HunkProvenance>,
    semantic: &str,
    original: &str,
    path: &str,
    declined: &[HunkDeclined],
    file_old: Option<&str>,
) {
    if declined.is_empty() {
        return;
    }
    if !has_native_hunk(semantic) || !has_semantic_hunk(semantic) {
        return;
    }
    let changed = crate::totality::changed_old_lines(original);
    if changed.is_empty() {
        return;
    }
    let ranges = emitted_hunk_ranges(semantic);
    // Claimed sites come from the already-committed semantic prov entries
    // (run-scoped `changed`), never from emitted-range intersection:
    // context-carrying bodies (`...` spans, e.g. the method `sig_evolution`
    // hunk) would otherwise over-claim remainder lines as covered. Inner
    // subhunk/verbatim classification probes stay on a throwaway by design
    // (the whole-hunk chain attempt already records per-family evidence).
    // Rules with no committed entry (unit-called helper) fall back to
    // `covered_sites`, preserving today's behavior.
    let mut claimed: Vec<usize> = Vec::new();
    for (rule, _, _) in &ranges {
        let mut from_prov: Vec<usize> = Vec::new();
        for p in prov.iter() {
            if p.rule == *rule && p.file == path {
                for s in &p.sites {
                    if !from_prov.contains(s) {
                        from_prov.push(*s);
                    }
                }
            }
        }
        let sites = if from_prov.is_empty() {
            covered_sites(&ranges, rule, &changed, rule_is_file_scope(semantic, rule))
        } else {
            from_prov
        };
        for site in sites {
            if !claimed.contains(&site) {
                claimed.push(site);
            }
        }
    }
    let remainder: Vec<usize> = changed
        .into_iter()
        .filter(|l| !claimed.contains(l))
        .collect();
    if remainder.is_empty() {
        return;
    }
    push_ordinary_prov(prov, path, declined, &remainder, file_old, semantic);
}

/// WI-15 (partial): true when `section` holds at least one semantic hunk
/// (an `@@` header carrying `rule=`). Mixed emissions hold both this and
/// [`has_native_hunk`]; wholly-semantic emissions hold only this.
pub(crate) fn has_semantic_hunk(section: &str) -> bool {
    section
        .lines()
        .any(|l| l.starts_with("@@") && l.contains("rule="))
}

/// WI-7a: swap a whole-section detector's collapsed `diff --git` header
/// for the original rename prelude (byte-exact: `similarity index`,
/// `rename from/to`, modes, `---`/`+++`). The detector body (from the
/// first `@@`) is kept verbatim; `None` when the emission holds no
/// hunk (defensive — detectors always emit one). Moved from `chain.rs`
/// under the 2000-line gate (D5): pure move, no behavior change.
pub(crate) fn reheader_rename(semantic: &str, prelude: &str) -> Option<String> {
    let at = semantic.find("@@")?;
    let mut out = prelude.to_string();
    out.push_str(&semantic[at..]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// WI-3: shared whole-patch pre-pass epilogue for the single-def-shape
/// intent sweeps (`item_intent`, `item_joint_impl`, `member_rename`,
/// `mod_rename` — all return the rebuilt sections plus the definition
/// path). Finds the emitted definition section, rebuilds the replaced
/// original bytes plus their changed lines, commits provenance under
/// `detector`, flips `compressed_any`, and returns the rewritten
/// sections for the caller's owned binding. The `fn_joint` pre-pass
/// keeps its own epilogue (4-tuple return with the suppression set).
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_pre_pass(
    prov: &mut Vec<HunkProvenance>,
    sections: &[String],
    rewritten: Vec<String>,
    def_path: &str,
    detector: &'static str,
    rules_before: usize,
    used_rules: &[String],
    pre: &std::collections::HashMap<String, String>,
    compressed_any: &mut bool,
) -> Vec<String> {
    *compressed_any = true;
    let emitted = rewritten
        .iter()
        .find(|s| {
            s.lines().next().is_some_and(|h| {
                h.starts_with("diff --git ")
                    && crate::b_side_path(&h["diff --git ".len()..]).is_some_and(|p| p == def_path)
            })
        })
        .cloned()
        .unwrap_or_default();
    let original: String = sections
        .iter()
        .filter(|s| !rewritten.iter().any(|r| r == *s))
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("");
    let changed: Vec<usize> = sections
        .iter()
        .filter(|s| !rewritten.iter().any(|r| r == *s))
        .flat_map(|s| crate::totality::changed_old_lines(s.as_str()))
        .collect();
    if !emitted.is_empty() && !original.is_empty() {
        let detector_of: Vec<(String, &'static str)> = used_rules[rules_before..]
            .iter()
            .map(|r| (r.clone(), detector))
            .collect();
        commit_prov(
            prov,
            &emitted,
            &original,
            def_path,
            &detector_of,
            &changed,
            pre.get(def_path).map(String::as_str),
        );
    }
    rewritten
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_dummy_covers_whole_changed() {
        // WI-2 sweep triage: a file-scope sweep dummy (`@@ -1,1 @@` +
        // file scope) cites every changed line (verbatim r7 cited none).
        let ranges = vec![("r7".to_string(), 1, 1)];
        assert_eq!(
            covered_sites(&ranges, "r7", &[297, 342, 396], true),
            vec![297, 342, 396]
        );
        // Ranged hunks still intersect.
        let ranges = vec![("r1".to_string(), 290, 10)];
        assert_eq!(
            covered_sites(&ranges, "r1", &[297, 342, 396], false),
            vec![297]
        );
        // Missing headers fall back to whole changed.
        let ranges: Vec<(String, usize, usize)> = vec![];
        assert_eq!(covered_sites(&ranges, "r9", &[5], false), vec![5]);
        assert_eq!(covered_sites(&ranges, "r9", &[5], true), vec![5]);
    }
    #[test]
    fn ranged_one_one_claims_line_one_only() {
        // WI-15 partial residue: a `(1,1)` range without file scope is a
        // ranged single-line claim at line 1, not a dummy — it intersects
        // instead of covering the whole changed set.
        let ranges = vec![("r1".to_string(), 1, 1)];
        assert_eq!(covered_sites(&ranges, "r1", &[1, 4], false), vec![1]);
        assert_eq!(
            covered_sites(&ranges, "r1", &[4], false),
            Vec::<usize>::new()
        );
        assert_eq!(covered_sites(&ranges, "r1", &[1, 4], true), vec![1, 4]);
    }
    #[test]
    fn rule_is_file_scope_reads_body_not_range() {
        // Body-aware dummy detection: the range alone never proves file
        // scope — only `# sweep` / `# scope=file` in the rule's block.
        let sweep = "diff --git a/f.rs b/f.rs\n@@ -1,1 @@ rule=r1\n# sweep\n- a\n+ b\n";
        let scope =
            "diff --git a/f.rs b/f.rs\n@@ -1,1 @@ rule=r1\n# allow=many\n# scope=file\n- a\n+ b\n";
        let ranged = "diff --git a/f.rs b/f.rs\n@@ -1,1 @@ rule=r1\n# allow=many\n- a\n+ b\n";
        let plain = "diff --git a/f.rs b/f.rs\n@@ -1,1 @@ rule=r1\n- a\n+ b\n";
        assert!(rule_is_file_scope(sweep, "r1"));
        assert!(rule_is_file_scope(scope, "r1"));
        assert!(!rule_is_file_scope(ranged, "r1"));
        assert!(!rule_is_file_scope(plain, "r1"));
        assert!(!rule_is_file_scope(sweep, "r9"));
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

    fn declined(names: &[(&str, &str)]) -> Vec<HunkDeclined> {
        names
            .iter()
            .map(|(d, r)| HunkDeclined {
                detector: d.to_string(),
                reason: r.to_string(),
            })
            .collect()
    }

    #[test]
    fn attempt_records_only_on_none() {
        // WI-15c: the name lives at the call site — `Some` passes
        // through untouched, `None` records `(detector, "no-match")`.
        let mut v = Vec::new();
        assert_eq!(attempt(&mut v, "call_sweep", Some(1)), Some(1));
        assert!(v.is_empty());
        assert_eq!(attempt(&mut v, "call_sweep", None::<i32>), None);
        assert_eq!(v, declined(&[("call_sweep", "no-match")]), "got: {v:?}");
    }

    #[test]
    fn ordinary_entry_dedupes_preserving_chain_order() {
        // WI-15c: one hunk's singles declining the same family five
        // times reports it once, first-seen order kept.
        let mut prov = Vec::new();
        push_ordinary_prov(
            &mut prov,
            "f.rs",
            &declined(&[
                ("call_sweep", "no-match"),
                ("verbatim", "no-match"),
                ("call_sweep", "no-match"),
                ("call_sweep", "byte-guard"),
            ]),
            &[2],
            Some("pre"),
            "diff --git f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2 +2 @@\n-    // hi\n+    // bye\n",
        );
        assert_eq!(prov.len(), 1, "got: {prov:?}");
        let e = &prov[0];
        assert_eq!(e.detector, "ordinary");
        assert_eq!(e.rule, "");
        assert_eq!(e.sites, vec![2]);
        assert!(e.sites_exact);
        assert!(e.proven);
        assert_eq!(e.saved_bytes, 0);
        assert_eq!(
            e.declined,
            declined(&[
                ("call_sweep", "no-match"),
                ("verbatim", "no-match"),
                ("call_sweep", "byte-guard"),
            ]),
            "got: {:?}",
            e.declined
        );
    }

    #[test]
    fn partial_mixed_emission_gains_remainder_entry() {
        // WI-15 partial: a mixed emission (one semantic hunk claiming
        // line 2, one verbatim remainder hunk on line 4) gains exactly
        // one remainder ordinary entry with the remainder site only.
        // (Line 1 is avoided: `@@ -1,1 @@` is sweep-dummy-shaped, so a
        // ranged claim there over-covers by construction — filed
        // separately, this pin stays off the ambiguity.)
        let original = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2 +2 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n@@ -4 +4 @@\n-    // hi\n+    // bye\n";
        let semantic = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 @@ rule=r1\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n@@ -4 +4 @@\n-    // hi\n+    // bye\n";
        let mut prov = Vec::new();
        push_partial_ordinary_prov(
            &mut prov,
            semantic,
            original,
            "f.rs",
            &declined(&[("call_sweep", "no-match"), ("verbatim", "no-match")]),
            Some("pre"),
        );
        assert_eq!(prov.len(), 1, "got: {prov:?}");
        let e = &prov[0];
        assert_eq!(e.detector, "ordinary");
        assert_eq!(e.rule, "");
        assert_eq!(e.sites, vec![4], "remainder only, got: {prov:?}");
        assert!(e.sites_exact);
        assert_eq!(e.saved_bytes, 0);
        assert_eq!(
            e.declined,
            declined(&[("call_sweep", "no-match"), ("verbatim", "no-match")])
        );
        assert!(has_semantic_hunk(semantic));
        assert!(has_native_hunk(semantic));
    }

    #[test]
    fn partial_helper_stays_silent_without_remainder_or_tries() {
        // Wholly-semantic emissions, wholly-ordinary callers (guarded),
        // fully-claimed sweep dummies, and empty declined lists stay
        // silent — no spurious remainder entry.
        let mut prov = Vec::new();
        push_partial_ordinary_prov(
            &mut prov,
            "diff --git a/f.rs b/f.rs\n@@ -1,1 @@ rule=r1\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n",
            "f.rs",
            &declined(&[("call_sweep", "no-match")]),
            Some("pre"),
        );
        assert!(prov.is_empty(), "wholly-semantic stays silent: {prov:?}");
        // Empty declined (nothing tried) stays silent even when mixed.
        push_partial_ordinary_prov(
            &mut prov,
            "diff --git a/f.rs b/f.rs\n@@ -1,1 @@ rule=r1\n-x\n@@ -2,1 @@\n-y\n+z\n",
            "diff --git a/f.rs b/f.rs\n@@ -1,1 @@\n-x\n+y\n@@ -2,1 @@\n-y\n+z\n",
            "f.rs",
            &[],
            Some("pre"),
        );
        assert!(prov.is_empty(), "untried stays silent: {prov:?}");
        // Sweep dummy claims everything — no remainder to explain.
        push_partial_ordinary_prov(
            &mut prov,
            "diff --git a/f.rs b/f.rs\n@@ -1,1 @@ rule=r1\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n",
            "f.rs",
            &declined(&[("call_sweep", "no-match")]),
            Some("pre"),
        );
        assert!(prov.is_empty(), "fully-claimed stays silent: {prov:?}");
        assert!(!has_semantic_hunk(
            "diff --git a/f.rs b/f.rs\n@@ -1,1 @@\n-x\n+y\n"
        ));
    }

    #[test]
    fn ordinary_entry_silences_untried_and_rule_only_sections() {
        // WI-15c: empty declined (nothing tried — headers-only,
        // non-`.rs`) and rule-only sections (pre-pass emissions,
        // already semantic) both stay silent.
        let mut prov = Vec::new();
        push_ordinary_prov(
            &mut prov,
            "f.rs",
            &[],
            &[2],
            Some("pre"),
            "diff --git f.rs\n",
        );
        assert!(prov.is_empty());
        push_ordinary_prov(
            &mut prov,
            "a.rs",
            &declined(&[("delete", "no-match")]),
            &[2, 3],
            Some("pre"),
            "diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,2 +2,2 @@ rule=r1\n-    old(E);\n+    new(E);\n",
        );
        assert!(prov.is_empty(), "got: {prov:?}");
        // `has_native_hunk` gates mixed sections back in.
        assert!(has_native_hunk(
            "diff --git f.rs\n@@ -1,1 @@ rule=r1\n-x\n@@ -2,1 @@\n-y\n+z\n"
        ));
        assert!(!has_native_hunk(
            "diff --git f.rs\nold mode 100644\nnew mode 100755\n"
        ));
    }
    #[test]
    fn partial_claims_prov_sites_not_range_span() {
        // WI-15 P3 residue: context-carrying bodies (`...` spans — the
        // method `sig_evolution` hunk) must not over-claim remainder
        // lines. Claimed sites come from the committed prov entry
        // (`[2]`), so the remainder is `[3]`. (Line 1 is avoided:
        // `@@ -1,1 @@` is sweep-dummy-shaped and over-covers by
        // construction — filed separately.)
        let original = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,2 +2,2 @@\n-    fn alpha(x: i32) {\n-        1\n+    pub fn alpha(x: i32) {\n+        2\n";
        let semantic = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,3 @@ rule=r1\n  impl Svc {\n      ...\n-     fn alpha(...) {\n+     pub fn alpha(...) {\n          ...\n      }\n  }\n@@ -3,1 +3,1 @@\n-        1\n+        2\n";
        let mut prov = vec![HunkProvenance {
            rule: "r1".to_string(),
            file: "f.rs".to_string(),
            detector: "sig_evolution",
            sites: vec![2],
            ..Default::default()
        }];
        push_partial_ordinary_prov(
            &mut prov,
            semantic,
            original,
            "f.rs",
            &declined(&[("call_sweep", "no-match")]),
            Some("pre"),
        );
        assert_eq!(prov.len(), 2, "got: {prov:?}");
        assert_eq!(prov[1].detector, "ordinary");
        assert_eq!(prov[1].sites, vec![3], "remainder only, got: {prov:?}");
    }
    #[test]
    fn partial_falls_back_without_committed_entry() {
        // Unit-called helper with an empty prov keeps today's behavior:
        // a rule with no committed entry falls back to `covered_sites`.
        let original = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2 +2 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n@@ -4 +4 @@\n-    // hi\n+    // bye\n";
        let semantic = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 @@ rule=r1\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n@@ -4 +4 @@\n-    // hi\n+    // bye\n";
        let mut prov = Vec::new();
        push_partial_ordinary_prov(
            &mut prov,
            semantic,
            original,
            "f.rs",
            &declined(&[("call_sweep", "no-match")]),
            Some("pre"),
        );
        assert_eq!(prov.len(), 1, "got: {prov:?}");
        assert_eq!(prov[0].sites, vec![4], "got: {prov:?}");
    }
    #[test]
    fn partial_line_one_ranged_claims_only_line_one() {
        // WI-15 partial residue (`covered_sites` `(1,1)` ambiguity): a
        // ranged `(1,1)` claim at line 1 (no `# sweep`) covers only line
        // 1, so a mixed section with a line-4 remainder gains exactly one
        // remainder ordinary entry; the `# sweep` twin covers everything
        // and stays silent.
        let original = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n@@ -4 +4 @@\n-    // hi\n+    // bye\n";
        let ranged = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 @@ rule=r1\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n@@ -4 +4 @@\n-    // hi\n+    // bye\n";
        let sweep = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 @@ rule=r1\n# sweep\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n@@ -4 +4 @@\n-    // hi\n+    // bye\n";
        let mut prov = Vec::new();
        push_partial_ordinary_prov(
            &mut prov,
            ranged,
            original,
            "f.rs",
            &declined(&[("call_sweep", "no-match")]),
            Some("pre"),
        );
        assert_eq!(prov.len(), 1, "ranged (1,1) leaves a remainder: {prov:?}");
        assert_eq!(prov[0].detector, "ordinary");
        assert_eq!(prov[0].sites, vec![4], "remainder only, got: {prov:?}");
        let mut prov = Vec::new();
        push_partial_ordinary_prov(
            &mut prov,
            sweep,
            original,
            "f.rs",
            &declined(&[("call_sweep", "no-match")]),
            Some("pre"),
        );
        assert!(prov.is_empty(), "sweep dummy claims all: {prov:?}");
    }
}
