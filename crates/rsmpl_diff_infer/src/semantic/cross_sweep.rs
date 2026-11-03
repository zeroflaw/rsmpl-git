//! WI-12 cross-file assoc + trait-method + inherent-method intent
//! sweeps (SPEC D.4).
//!
//! Whole-patch pre-passes joining definition pairs spread across files
//! with same-delta use lines into one shared-rule intent. The lowering
//! is already workspace-wide; these close the one-file-boundary lag in
//! raising (F9). `member` already owns its cross-file sweep (WI-15b)
//! and is untouched here.
//!
//! - `try_assoc_intent_sweep`: K ≥ 1 assoc `type`/`const` def pairs in
//!   `trait`/`impl` blocks (any files, one per block, same kind and
//!   ident delta) plus `::Old` use lines. One section per def file
//!   (verbatim block header, shared rule), `# follow=uses` on the
//!   document-first hunk only (the single-file multi-hunk shape,
//!   spread over files).
//! - `try_trait_method_intent_sweep`: method def pairs uniform in one
//!   ident delta with ≥1 `trait`-block decl and ≥1 trait-`impl` def
//!   (same trait, last-segment discipline) plus anchored call lines.
//!   One hunk per enclosing block across files (shared rule, follow on
//!   the document-first hunk; the WI-4b K-hunk shape, spread over
//!   files). The single-hunk fast path is deliberately omitted: it only
//!   suffices when one hunk owns every def, which is the same-file
//!   detector's shape.
//! - `try_inherent_method_intent_sweep`: method def pairs uniform in
//!   one ident delta, every def in an inherent `impl` block of one
//!   self type (last-segment discipline) across files, plus anchored
//!   call lines. Single-hunk shape only (document-first impl block +
//!   `# follow=uses` claims every same-self-type def + call
//!   workspace-wide); def-only classes stay with the per-section
//!   detector.
//!
//! Totality in both is the `item_intent`-posture workspace
//! scratch-apply probe: the emission applied to a `pre` clone must show
//! exactly the native plus lines at the claimed sites with every other
//! line byte-identical (an off-diff same-name use anywhere in the
//! workspace fails the unchanged-leg and vetoes the whole intent).
//! Single-section pure shapes stay with the per-section detectors
//! (decline when the patch holds one section and no leftovers).

use std::collections::{HashMap, HashSet};

use super::{
    emission_hunks, item_emission_header, split_native_hunks, split_semantic_native, unique_rule,
};

/// One native minus/plus line pair at its old-side site.
struct Pair {
    sec: usize,
    path: String,
    minus: String,
    plus: String,
    site: usize,
}

/// One native hunk with verbatim text (unbalanced hunks are skipped by
/// both sweeps — neither intent owns insertions/deletions).
struct HunkEntry {
    sec: usize,
    text: String,
    minuses: Vec<String>,
    pluses: Vec<String>,
    sites: Vec<usize>,
    balanced: bool,
    path: String,
}

/// Shared harvest: per-hunk entries over eligible (`.rs`, non-rename)
/// sections plus one flattened pair per balanced-hunk line, in section
/// order. `None` when fewer than two pairs exist.
type Harvest = (
    Vec<HunkEntry>,
    Vec<Pair>,
    Vec<usize>,
    HashMap<usize, (String, Vec<String>)>,
    HashMap<usize, usize>,
    HashMap<usize, String>,
);

fn harvest(sections: &[String]) -> Option<Harvest> {
    let mut hunks: Vec<HunkEntry> = Vec::new();
    let mut preserved: HashMap<usize, (String, Vec<String>)> = HashMap::new();
    let mut native_lens: HashMap<usize, usize> = HashMap::new();
    let mut sec_paths: HashMap<usize, String> = HashMap::new();
    for (i, sec) in sections.iter().enumerate() {
        let header = sec.lines().next().unwrap_or("");
        if !header.starts_with("diff --git ") || sec.contains('\r') {
            continue;
        }
        if super::is_rename_header(header) {
            continue;
        }
        let (prelude, semantic_hunks, native_view) = split_semantic_native(sec);
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
        sec_paths.insert(i, path.clone());
        for nh in split_native_hunks(&native_sec).into_iter() {
            hunks.push(HunkEntry {
                sec: i,
                text: nh.text,
                minuses: nh.minuses,
                pluses: nh.pluses,
                sites: nh.sites,
                balanced: nh.balanced,
                path: path.clone(),
            });
        }
    }
    if hunks.is_empty() {
        return None;
    }
    let mut pairs: Vec<Pair> = Vec::new();
    let mut pair_hunk: Vec<usize> = Vec::new();
    for (hi, he) in hunks.iter().enumerate() {
        if !he.balanced {
            continue;
        }
        if he.minuses.len() != he.sites.len() {
            return None;
        }
        for ((m, p), s) in he.minuses.iter().zip(he.pluses.iter()).zip(he.sites.iter()) {
            pairs.push(Pair {
                sec: he.sec,
                path: he.path.clone(),
                minus: m.clone(),
                plus: p.clone(),
                site: *s,
            });
            pair_hunk.push(hi);
        }
    }
    if pairs.len() < 2 {
        return None;
    }
    Some((hunks, pairs, pair_hunk, preserved, native_lens, sec_paths))
}

/// Ident-boundary containment (never substring).
fn contains_ident(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut token = String::new();
    for c in haystack.chars().chain(std::iter::once(' ')) {
        if c == '_' || c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if token == needle {
                return true;
            }
            token.clear();
        }
    }
    false
}

/// Workspace scratch probe: the joined emission applied to a `pre`
/// clone must show exactly each claimed plus line at its site with
/// every other line in every file byte-identical.
fn probe_reproduces(pre: &HashMap<String, String>, emission: &str, pairs: &[Pair]) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(emission) else {
        return false;
    };
    let mut files = pre.clone();
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    let mut claims: HashMap<&str, HashMap<usize, &str>> = HashMap::new();
    for pair in pairs {
        claims
            .entry(pair.path.as_str())
            .or_default()
            .insert(pair.site, pair.plus.trim());
    }
    for (path, pre_text) in pre {
        let Some(post_text) = files.get(path) else {
            return false;
        };
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let post_lines: Vec<&str> = post_text.lines().collect();
        if post_lines.len() != pre_lines.len() {
            return false;
        }
        let empty: HashMap<usize, &str> = HashMap::new();
        let claim = claims.get(path.as_str()).unwrap_or(&empty);
        for (idx, (a, b)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
            if let Some(want) = claim.get(&(idx + 1)) {
                if b.trim() != *want {
                    return false;
                }
            } else if a != b {
                return false;
            }
        }
    }
    true
}

/// T375 byte-win guard on the member/item-intent basis: pure sections
/// compare whole native views, mixed sections compare claimed hunk
/// bytes vs emission hunk bytes.
fn byte_win(
    emission: &str,
    hunks: &[HunkEntry],
    claimed_hunks: &HashSet<usize>,
    native_lens: &HashMap<usize, usize>,
    preserved: &HashMap<usize, (String, Vec<String>)>,
) -> bool {
    let claimed_secs: HashSet<usize> = claimed_hunks.iter().map(|hi| hunks[*hi].sec).collect();
    let has_leftover = hunks.iter().enumerate().any(|(hi, he)| {
        if claimed_hunks.contains(&hi) {
            return false;
        }
        claimed_secs.contains(&he.sec) && preserved.contains_key(&he.sec)
    });
    let claimed_bytes: usize = if has_leftover {
        claimed_hunks.iter().map(|hi| hunks[*hi].text.len()).sum()
    } else {
        claimed_secs
            .iter()
            .map(|s| native_lens.get(s).copied().unwrap_or(0))
            .sum()
    };
    let emission_bytes: usize = if has_leftover {
        emission_hunks(emission).iter().map(|h| h.len()).sum()
    } else {
        emission.len()
    };
    emission_bytes < claimed_bytes
}

/// Rebuild with a multi-section emission (`path -> section`): def files
/// keep preserved + their emission hunks + leftovers; use-only claimed
/// files keep preserved + leftovers (collapse when both empty);
/// untouched sections pass through.
fn rebuild(
    sections: &[String],
    hunks: &[HunkEntry],
    claimed_hunks: &HashSet<usize>,
    preserved: &HashMap<usize, (String, Vec<String>)>,
    sec_paths: &HashMap<usize, String>,
    emissions: &HashMap<String, String>,
) -> Vec<String> {
    let claimed_secs: HashSet<usize> = claimed_hunks.iter().map(|hi| hunks[*hi].sec).collect();
    let mut leftovers: HashMap<usize, Vec<String>> = HashMap::new();
    for (hi, he) in hunks.iter().enumerate() {
        if claimed_hunks.contains(&hi) {
            continue;
        }
        if !claimed_secs.contains(&he.sec) {
            continue;
        }
        leftovers.entry(he.sec).or_default().push(he.text.clone());
    }
    let mut out: Vec<String> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        if !claimed_secs.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        let Some(path) = sec_paths.get(&i) else {
            out.push(sec.clone());
            continue;
        };
        let (prelude, keep) = preserved.get(&i).cloned().unwrap_or_default();
        let rest = leftovers.get(&i).cloned().unwrap_or_default();
        if let Some(em) = emissions.get(path) {
            if keep.is_empty() && rest.is_empty() {
                out.push(em.clone());
            } else {
                let mut rebuilt = prelude;
                for h in &keep {
                    rebuilt.push_str(h);
                }
                for h in &emission_hunks(em) {
                    rebuilt.push_str(h);
                }
                for h in &rest {
                    rebuilt.push_str(h);
                }
                out.push(rebuilt);
            }
        } else if keep.is_empty() && rest.is_empty() {
            continue;
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
    out
}

/// Single-section pure shapes stay with the per-section detectors:
/// decline when every eligible section is one section with no
/// leftovers (the whole-section path owns it byte-identically).
fn single_section_pure(
    hunks: &[HunkEntry],
    claimed_hunks: &HashSet<usize>,
    preserved: &HashMap<usize, (String, Vec<String>)>,
) -> bool {
    let mut all_secs: HashSet<usize> = HashSet::new();
    for he in hunks {
        if preserved.contains_key(&he.sec) {
            all_secs.insert(he.sec);
        }
    }
    if all_secs.len() != 1 {
        return false;
    }
    hunks
        .iter()
        .enumerate()
        .all(|(hi, _)| claimed_hunks.contains(&hi))
}

/// Whole-patch cross-file assoc intent: K ≥ 1 assoc def pairs across
/// files plus `::Old` use lines, one shared-rule intent. Returns the
/// rebuilt section list plus the definition path and rule, or `None`.
/// Reserved rule names are released on decline.
pub(crate) fn try_assoc_intent_sweep(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    let (hunks, pairs, pair_hunk, preserved, native_lens, sec_paths) = harvest(sections)?;
    // Pass 1: assoc def pairs (uniform kind + ident delta; a second
    // delta vetoes — two intents, never grouped).
    let mut def_idx: Vec<usize> = Vec::new();
    let mut is_type = false;
    let mut old = String::new();
    let mut new = String::new();
    let mut first = true;
    for (i, pair) in pairs.iter().enumerate() {
        if let Some(d) = super::assoc::parse_assoc_def_pair(&pair.minus, &pair.plus) {
            if first {
                is_type = d.is_type;
                old = d.old.clone();
                new = d.new.clone();
                first = false;
            } else if d.is_type != is_type || d.old != old || d.new != new {
                used_rules.truncate(checkpoint);
                return None;
            }
            def_idx.push(i);
        }
    }
    if def_idx.is_empty() || old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    // Every def site must sit in a trait/impl block; verbatim header
    // plus block span per def. The trimmed native def line must match
    // its minus side (trailing `,` tolerated — patterns omit member
    // commas).
    let mut def_blocks: Vec<(usize, String, usize, usize)> = Vec::new();
    for &pi in &def_idx {
        let pair = &pairs[pi];
        let pre_text = pre.get(&pair.path)?;
        let (header, start, end) = super::assoc::enclosing_block(pre_text, pair.site)?;
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let pre_line = pre_lines.get(pair.site.saturating_sub(1))?;
        let want = pair.minus.trim();
        let got = pre_line.trim();
        if got != want
            && got != format!("{want},").as_str()
            && got.trim_end_matches(',').trim() != want.trim_end_matches(',').trim()
        {
            return None;
        }
        def_blocks.push((pi, header, start, end));
    }
    // Two-trait veto: distinct named traits across def headers mean two
    // intents (mirror the single-file rule).
    {
        let mut traits: Vec<String> = Vec::new();
        for (_, h, _, _) in &def_blocks {
            if let Some(t) = super::assoc::block_trait_name(h) {
                if !traits.contains(&t) {
                    traits.push(t);
                }
            }
        }
        if traits.len() > 1 {
            return None;
        }
    }
    // Group defs by (path, header): one def per block.
    let mut groups: Vec<(String, String, usize, usize, Vec<usize>)> = Vec::new();
    for (pi, header, start, end) in def_blocks {
        let path = pairs[pi].path.clone();
        match groups
            .iter_mut()
            .find(|(p, h, _, _, _)| *p == path && *h == header)
        {
            Some(g) => {
                g.4.push(pi);
                // A duplicate same-name def in one block is ambiguous.
                used_rules.truncate(checkpoint);
                return None;
            }
            None => groups.push((path, header, start, end, vec![pi])),
        }
    }
    // Hunk-level claiming: def hunks hold exactly their def pair;
    // every other claimed hunk is all-claimable (ident + tail gate).
    let mut hunk_pairs: Vec<Vec<usize>> = vec![Vec::new(); hunks.len()];
    for (pi, hi) in pair_hunk.iter().enumerate() {
        hunk_pairs[*hi].push(pi);
    }
    let def_set: HashSet<usize> = def_idx.iter().copied().collect();
    let claimable = |pi: usize| -> bool {
        if def_set.contains(&pi) {
            return true;
        }
        let pair = &pairs[pi];
        if pair.minus.trim() == pair.plus.trim() {
            return false;
        }
        super::assoc::contains_path_tail(&pair.minus, &old)
            && super::assoc::contains_path_tail(&pair.plus, &new)
            && contains_ident(&pair.minus, &old)
            && contains_ident(&pair.plus, &new)
    };
    // A def hunk carrying an unclaimable pair declines the intent.
    for pis in hunk_pairs.iter() {
        if pis.iter().any(|pi| def_set.contains(pi)) {
            for pi in pis {
                if !claimable(*pi) {
                    return None;
                }
            }
        }
    }
    let mut claimed_hunks: HashSet<usize> = HashSet::new();
    for (hi, pis) in hunk_pairs.iter().enumerate() {
        if pis.iter().any(|pi| def_set.contains(pi))
            || (!pis.is_empty() && pis.iter().all(|pi| claimable(*pi)))
        {
            claimed_hunks.insert(hi);
        }
    }
    // At least one def and one use (or a second def) must be claimed.
    let claimed_pairs: usize = hunk_pairs
        .iter()
        .enumerate()
        .filter(|(hi, _)| claimed_hunks.contains(hi))
        .map(|(_, pis)| pis.len())
        .sum();
    if claimed_pairs < 2 {
        return None;
    }
    if single_section_pure(&hunks, &claimed_hunks, &preserved) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old}_to_{new}"));
    // Emission: one section per def file, one `@@` hunk per block in
    // group order, `# follow=uses` on the document-first hunk only.
    let mut emissions: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for (gi, (path, header, start, end, _)) in groups.iter().enumerate() {
        let pi = groups[gi].4[0];
        let body = format!(
            "  {header}\n- {}\n+ {}\n",
            pairs[pi].minus.trim(),
            pairs[pi].plus.trim()
        );
        if let Some(entry) = emissions.get_mut(path) {
            entry.push_str(&crate::emission_body(&rule, &[*start, *end]));
            entry.push_str(&body);
        } else {
            let mut sec = crate::emission_header(path, &rule, &[*start, *end]);
            if gi == 0 {
                sec.push_str("# follow=uses\n");
            }
            sec.push_str(&body);
            order.push(path.clone());
            emissions.insert(path.clone(), sec);
        }
    }
    // Claimed pair list for the probe (def + use pairs in claimed hunks).
    let mut claimed_idx: Vec<usize> = Vec::new();
    for hi in claimed_hunks.iter() {
        claimed_idx.extend(hunk_pairs[*hi].iter().copied());
    }
    claimed_idx.sort_unstable();
    let claimed: Vec<Pair> = claimed_idx
        .iter()
        .map(|pi| Pair {
            sec: pairs[*pi].sec,
            path: pairs[*pi].path.clone(),
            minus: pairs[*pi].minus.clone(),
            plus: pairs[*pi].plus.clone(),
            site: pairs[*pi].site,
        })
        .collect();
    for pair in &claimed {
        if !pre.contains_key(&pair.path) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    let joined: String = order.iter().map(|p| emissions[p].as_str()).collect();
    if !probe_reproduces(pre, &joined, &claimed) {
        used_rules.truncate(checkpoint);
        return None;
    }
    if !byte_win(&joined, &hunks, &claimed_hunks, &native_lens, &preserved) {
        used_rules.truncate(checkpoint);
        return None;
    }
    let def_path = groups[0].0.clone();
    let out = rebuild(
        sections,
        &hunks,
        &claimed_hunks,
        &preserved,
        &sec_paths,
        &emissions,
    );
    Some((out, def_path, rule))
}

/// Whole-patch cross-file trait-method intent: method def pairs uniform
/// in one ident delta with ≥1 `trait`-block decl and ≥1 trait-`impl`
/// def across files, plus anchored call lines. One shared-rule intent.
/// Returns the rebuilt section list plus the definition path and rule.
pub(crate) fn try_trait_method_intent_sweep(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    let (hunks, pairs, pair_hunk, preserved, native_lens, sec_paths) = harvest(sections)?;
    // Two-phase classification shared with the single-file detector:
    // def pairs parse as fn headers with a uniform ident delta;
    // non-def pairs are candidate anchored calls.
    let mut old_name: Option<String> = None;
    let mut new_name: Option<String> = None;
    let mut terms: Vec<Option<char>> = Vec::with_capacity(pairs.len());
    let mut is_def: Vec<bool> = Vec::with_capacity(pairs.len());
    for pair in &pairs {
        if let Some((o, n, term)) = super::trait_methods::trait_rename_pair(&pair.minus, &pair.plus)
        {
            if old_name.as_ref().is_some_and(|c| *c != o) {
                used_rules.truncate(checkpoint);
                return None;
            }
            if new_name.as_ref().is_some_and(|c| *c != n) {
                used_rules.truncate(checkpoint);
                return None;
            }
            old_name = Some(o);
            new_name = Some(n);
            terms.push(Some(term));
            is_def.push(true);
        } else {
            terms.push(None);
            is_def.push(false);
        }
    }
    let (old_name, new_name) = (old_name?, new_name?);
    if old_name == new_name {
        return None;
    }
    // Non-def pairs are gated per hunk below (claimable calls join,
    // the rest stay ordinary leftovers); no whole-patch veto here.
    let all_indented = pairs.iter().all(|p| p.minus.starts_with([' ', '\t']));
    let plus_indented = pairs.iter().all(|p| p.plus.starts_with([' ', '\t']));
    if !all_indented || !plus_indented {
        return None;
    }
    // Side classification: every def site in a trait block or a
    // (non-inherent) impl block; inherent defs veto (different items).
    let mut trait_sites: HashMap<String, Vec<usize>> = HashMap::new();
    let mut impl_sites: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, pair) in pairs.iter().enumerate() {
        if !is_def[i] {
            continue;
        }
        let pre_text = pre.get(&pair.path)?;
        if super::trait_methods::in_trait_block(pre_text, pair.site) {
            trait_sites
                .entry(pair.path.clone())
                .or_default()
                .push(pair.site);
        } else if super::trait_methods::in_impl_block(pre_text, pair.site) {
            if super::trait_methods::site_in_inherent_impl(pre_text, pair.site) {
                return None;
            }
            impl_sites
                .entry(pair.path.clone())
                .or_default()
                .push(pair.site);
        } else {
            return None;
        }
    }
    if trait_sites.is_empty() || impl_sites.is_empty() {
        return None;
    }
    // One trait across every def site (WI-22b, last-segment discipline).
    {
        let mut names: HashSet<String> = HashSet::new();
        for (path, sites) in &trait_sites {
            let pre_text = &pre[path];
            for &site in sites {
                names.insert(super::trait_methods::trait_ident_for_site(pre_text, site)?);
            }
        }
        for (path, sites) in &impl_sites {
            let pre_text = &pre[path];
            for &site in sites {
                names.insert(super::trait_methods::impl_trait_ident_for_site(
                    pre_text, site,
                )?);
            }
        }
        if names.len() != 1 {
            return None;
        }
    }
    // Term per site (uniform ident delta already proven; terminator
    // must be uniform per emitted block).
    let mut term_of: HashMap<(String, usize), char> = HashMap::new();
    for (i, pair) in pairs.iter().enumerate() {
        if let Some(t) = terms[i] {
            term_of.insert((pair.path.clone(), pair.site), t);
        }
    }
    // Group per file with the existing block groupers, then merge in
    // document (section, site) order. `trait_files` is kept for the
    // single-hunk emission below (`trait_sites` is moved by grouping).
    let trait_files: HashSet<String> = trait_sites.keys().cloned().collect();
    let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new();
    let mut all_trait: Vec<(String, Vec<usize>)> = trait_sites.into_iter().collect();
    all_trait.sort();
    for (path, mut sites) in all_trait {
        sites.sort_unstable();
        let pre_text = &pre[&path];
        let gs = super::block_headers::group_sites_by_trait_block(Some(pre_text), &sites)?;
        for g in gs {
            groups.push((path.clone(), g.header_line, g.sites));
        }
    }
    let mut all_impl: Vec<(String, Vec<usize>)> = impl_sites.into_iter().collect();
    all_impl.sort();
    for (path, mut sites) in all_impl {
        sites.sort_unstable();
        let pre_text = &pre[&path];
        let gs = super::block_headers::group_sites_by_impl_block(Some(pre_text), &sites)?;
        for g in gs {
            groups.push((path.clone(), g.header_line, g.sites));
        }
    }
    // Document order: trait groups of a file before impl groups of the
    // same file would be ideal; order by (first section index, site).
    let mut sec_order: HashMap<String, usize> = HashMap::new();
    for pair in &pairs {
        sec_order.entry(pair.path.clone()).or_insert(pair.sec);
    }
    groups.sort_by_key(|(p, _, sites)| {
        (
            sec_order.get(p).copied().unwrap_or(usize::MAX),
            sites.first().copied().unwrap_or(usize::MAX),
        )
    });
    // Hunk-level claiming: def hunks hold def pairs (co-residents must
    // be claimable calls); other hunks all-claimable.
    let mut hunk_pairs: Vec<Vec<usize>> = vec![Vec::new(); hunks.len()];
    for (pi, hi) in pair_hunk.iter().enumerate() {
        hunk_pairs[*hi].push(pi);
    }
    let def_pair_set: HashSet<(String, usize)> = {
        let mut s = HashSet::new();
        for (p, _, sites) in &groups {
            for site in sites {
                s.insert((p.clone(), *site));
            }
        }
        s
    };
    let call_gate = |pi: usize| -> bool {
        if is_def[pi] {
            return true;
        }
        let pair = &pairs[pi];
        pair.minus.trim() != pair.plus.trim()
            && contains_ident(&pair.minus, &old_name)
            && contains_ident(&pair.plus, &new_name)
    };
    let is_def_hunk = |pis: &[usize]| -> bool {
        pis.iter().any(|pi| {
            is_def[*pi] && def_pair_set.contains(&(pairs[*pi].path.clone(), pairs[*pi].site))
        })
    };
    for pis in &hunk_pairs {
        if is_def_hunk(pis) {
            for pi in pis {
                if !call_gate(*pi) {
                    return None;
                }
            }
        }
    }
    let mut claimed_hunks: HashSet<usize> = HashSet::new();
    for (hi, pis) in hunk_pairs.iter().enumerate() {
        if pis.is_empty() {
            continue;
        }
        if is_def_hunk(pis) || pis.iter().all(|pi| call_gate(*pi)) {
            claimed_hunks.insert(hi);
        }
    }
    let claimed_pairs: usize = hunk_pairs
        .iter()
        .enumerate()
        .filter(|(hi, _)| claimed_hunks.contains(hi))
        .map(|(_, pis)| pis.len())
        .sum();
    if claimed_pairs < 2 {
        return None;
    }
    if single_section_pure(&hunks, &claimed_hunks, &preserved) {
        return None;
    }
    // Per-block emission (block probe per hunk: twin headers veto):
    // one section per def file, one `@@` hunk per block, shared rule,
    // `# follow=uses` on the document-first hunk only.
    //
    // WI-3 single-hunk shape: the follow anchored on the trait decl
    // claims every trait-proven `impl <Trait> for` definition (same
    // file or not) as `rename` spans, so one trait-block hunk suffices
    // — sibling def hunks would double-claim the lines the follow
    // already rewrites and fail the probe. A failed single-hunk probe
    // declines to ordinary (never a def-only partial, WI-23 posture).
    let rule = unique_rule(used_rules, &format!("rename_{old_name}_to_{new_name}"));
    let (first_path, first_header, first_sites) = groups
        .iter()
        .find(|(p, _, _)| trait_files.contains(p))
        .map(|(p, h, s)| (p.clone(), h.clone(), s.clone()))?;
    let mut term = None;
    for s in &first_sites {
        let t = term_of.get(&(first_path.clone(), *s))?;
        if term.is_some_and(|c| c != *t) {
            used_rules.truncate(checkpoint);
            return None;
        }
        term = Some(*t);
    }
    let minus_fn = format!("V Q fn {old_name} G (...) W {}", term?);
    let plus_fn = format!("V Q fn {new_name} G (...) W {}", term?);
    let first_pre = &pre[&first_path];
    if !super::block_headers::probe_block_hunk(
        &first_path,
        first_pre,
        &first_header,
        &minus_fn,
        &plus_fn,
        first_sites.len(),
    ) {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut sec = item_emission_header(&first_path, &rule, Some(first_pre), &first_sites);
    sec.push_str("# follow=uses\n");
    sec.push_str(&super::block_headers::block_hunk_body(
        &first_header,
        &minus_fn,
        &plus_fn,
    ));
    let mut emissions: HashMap<String, String> = HashMap::new();
    let order = [first_path.clone()];
    emissions.insert(first_path.clone(), sec);
    // Claimed pairs for the full-set probe.
    let mut claimed_idx: Vec<usize> = Vec::new();
    for hi in claimed_hunks.iter() {
        claimed_idx.extend(hunk_pairs[*hi].iter().copied());
    }
    claimed_idx.sort_unstable();
    let claimed: Vec<Pair> = claimed_idx
        .iter()
        .map(|pi| Pair {
            sec: pairs[*pi].sec,
            path: pairs[*pi].path.clone(),
            minus: pairs[*pi].minus.clone(),
            plus: pairs[*pi].plus.clone(),
            site: pairs[*pi].site,
        })
        .collect();
    let joined: String = order.iter().map(|p| emissions[p].as_str()).collect();
    if !probe_reproduces(pre, &joined, &claimed) {
        used_rules.truncate(checkpoint);
        return None;
    }
    if !byte_win(&joined, &hunks, &claimed_hunks, &native_lens, &preserved) {
        used_rules.truncate(checkpoint);
        return None;
    }
    let def_path = first_path.clone();
    let out = rebuild(
        sections,
        &hunks,
        &claimed_hunks,
        &preserved,
        &sec_paths,
        &emissions,
    );
    Some((out, def_path, rule))
}

/// Whole-patch cross-file inherent-method intent: method def pairs
/// uniform in one ident delta, every def in an inherent `impl` block of
/// one self type (last-segment discipline) across files, plus anchored
/// call lines. One shared-rule intent. Returns the rebuilt section list
/// plus the definition path and rule.
/// Single-hunk shape only (mirrors the trait-method WI-3 shape): the
/// document-first impl-block hunk carries `# follow=uses`; the engine
/// inherent follow already claims every same-self-type `impl S` def +
/// anchored calls workspace-wide (WI-6/WI-10), so sibling def hunks
/// would double-claim and fail the probe. A failed single-hunk probe
/// declines to ordinary, never a def-only partial.
pub(crate) fn try_inherent_method_intent_sweep(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    let (hunks, pairs, pair_hunk, preserved, native_lens, sec_paths) = harvest(sections)?;
    let mut old_name: Option<String> = None;
    let mut new_name: Option<String> = None;
    let mut terms: Vec<Option<char>> = Vec::with_capacity(pairs.len());
    let mut is_def: Vec<bool> = Vec::with_capacity(pairs.len());
    for pair in &pairs {
        if let Some((o, n, term)) = super::trait_methods::trait_rename_pair(&pair.minus, &pair.plus)
        {
            if old_name.as_ref().is_some_and(|c| *c != o) {
                used_rules.truncate(checkpoint);
                return None;
            }
            if new_name.as_ref().is_some_and(|c| *c != n) {
                used_rules.truncate(checkpoint);
                return None;
            }
            old_name = Some(o);
            new_name = Some(n);
            terms.push(Some(term));
            is_def.push(true);
        } else {
            terms.push(None);
            is_def.push(false);
        }
    }
    let (old_name, new_name) = (old_name?, new_name?);
    if old_name == new_name {
        return None;
    }
    // Inherent definitions are bodies (`{`); a `;` pair is a trait
    // decl (the trait sweep's domain, not this one). Single-line `}`
    // defs arrive normalized to `{` by `trait_rename_pair`.
    for t in terms.iter().flatten() {
        if *t != '{' {
            return None;
        }
    }
    let all_indented = pairs.iter().all(|p| p.minus.starts_with([' ', '\t']));
    let plus_indented = pairs.iter().all(|p| p.plus.starts_with([' ', '\t']));
    if !all_indented || !plus_indented {
        return None;
    }
    // Domain: every def site in an inherent `impl` block of one
    // self type. Trait blocks and `impl <T> for _` blocks veto
    // (different items, D9 — the trait sweep owns those).
    let mut domain: Option<String> = None;
    let mut def_sites: Vec<(String, usize)> = Vec::new();
    for (i, pair) in pairs.iter().enumerate() {
        if !is_def[i] {
            continue;
        }
        let pre_text = pre.get(&pair.path)?;
        if super::trait_methods::in_trait_block(pre_text, pair.site)
            || !super::trait_methods::site_in_inherent_impl(pre_text, pair.site)
        {
            return None;
        }
        let self_ty = super::trait_methods::inherent_self_ident_for_site(pre_text, pair.site)?;
        if domain.as_ref().is_some_and(|d| *d != self_ty) {
            return None;
        }
        domain = Some(self_ty);
        def_sites.push((pair.path.clone(), pair.site));
    }
    if def_sites.is_empty() {
        return None;
    }
    let _domain = domain?;
    // Hunk-level claiming mirrors the trait sweep: def hunks hold
    // def pairs (co-residents must be claimable calls); other hunks
    // all-claimable. Unclaimable lines stay ordinary leftovers.
    let mut hunk_pairs: Vec<Vec<usize>> = vec![Vec::new(); hunks.len()];
    for (pi, hi) in pair_hunk.iter().enumerate() {
        hunk_pairs[*hi].push(pi);
    }
    let def_pair_set: HashSet<(String, usize)> = def_sites.iter().cloned().collect();
    let call_gate = |pi: usize| -> bool {
        if is_def[pi] {
            return true;
        }
        let pair = &pairs[pi];
        pair.minus.trim() != pair.plus.trim()
            && contains_ident(&pair.minus, &old_name)
            && contains_ident(&pair.plus, &new_name)
    };
    let is_def_hunk = |pis: &[usize]| -> bool {
        pis.iter().any(|pi| {
            is_def[*pi] && def_pair_set.contains(&(pairs[*pi].path.clone(), pairs[*pi].site))
        })
    };
    for pis in &hunk_pairs {
        if is_def_hunk(pis) {
            for pi in pis {
                if !call_gate(*pi) {
                    return None;
                }
            }
        }
    }
    let mut claimed_hunks: HashSet<usize> = HashSet::new();
    for (hi, pis) in hunk_pairs.iter().enumerate() {
        if pis.is_empty() {
            continue;
        }
        if is_def_hunk(pis) || pis.iter().all(|pi| call_gate(*pi)) {
            claimed_hunks.insert(hi);
        }
    }
    // Def-only classes stay with the per-section slot sweep; the
    // follow needs at least one call to ride (same rule as the
    // single-file inherent detector).
    let mut claimed_def = false;
    let mut claimed_call = false;
    for hi in claimed_hunks.iter() {
        for pi in &hunk_pairs[*hi] {
            if is_def[*pi] {
                claimed_def = true;
            } else {
                claimed_call = true;
            }
        }
    }
    if !claimed_def || !claimed_call {
        return None;
    }
    if single_section_pure(&hunks, &claimed_hunks, &preserved) {
        return None;
    }
    // Group def sites by impl block across files, document order.
    let mut per_file: HashMap<String, Vec<usize>> = HashMap::new();
    for (path, site) in &def_sites {
        per_file.entry(path.clone()).or_default().push(*site);
    }
    let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new();
    let mut files: Vec<String> = per_file.keys().cloned().collect();
    files.sort();
    for path in files {
        let mut sites = per_file.remove(&path).unwrap_or_default();
        sites.sort_unstable();
        let pre_text = &pre[&path];
        let gs = super::block_headers::group_sites_by_impl_block(Some(pre_text), &sites)?;
        for g in gs {
            groups.push((path.clone(), g.header_line, g.sites));
        }
    }
    let mut sec_order: HashMap<String, usize> = HashMap::new();
    for pair in &pairs {
        sec_order.entry(pair.path.clone()).or_insert(pair.sec);
    }
    groups.sort_by_key(|(p, _, sites)| {
        (
            sec_order.get(p).copied().unwrap_or(usize::MAX),
            sites.first().copied().unwrap_or(usize::MAX),
        )
    });
    let rule = unique_rule(used_rules, &format!("rename_{old_name}_to_{new_name}"));
    let (first_path, first_header, first_sites) = groups.first().cloned()?;
    let minus_fn = format!("V Q fn {old_name} G (...) W {{");
    let plus_fn = format!("V Q fn {new_name} G (...) W {{");
    let first_pre = &pre[&first_path];
    if !super::block_headers::probe_block_hunk(
        &first_path,
        first_pre,
        &first_header,
        &minus_fn,
        &plus_fn,
        first_sites.len(),
    ) {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut sec = item_emission_header(&first_path, &rule, Some(first_pre), &first_sites);
    sec.push_str("# follow=uses\n");
    sec.push_str(&super::block_headers::block_hunk_body(
        &first_header,
        &minus_fn,
        &plus_fn,
    ));
    let mut claimed_idx: Vec<usize> = Vec::new();
    for hi in claimed_hunks.iter() {
        claimed_idx.extend(hunk_pairs[*hi].iter().copied());
    }
    claimed_idx.sort_unstable();
    let claimed: Vec<Pair> = claimed_idx
        .iter()
        .map(|pi| Pair {
            sec: pairs[*pi].sec,
            path: pairs[*pi].path.clone(),
            minus: pairs[*pi].minus.clone(),
            plus: pairs[*pi].plus.clone(),
            site: pairs[*pi].site,
        })
        .collect();
    for pair in &claimed {
        if !pre.contains_key(&pair.path) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    let joined = sec.clone();
    if !probe_reproduces(pre, &joined, &claimed) {
        used_rules.truncate(checkpoint);
        return None;
    }
    if !byte_win(&joined, &hunks, &claimed_hunks, &native_lens, &preserved) {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut emissions: HashMap<String, String> = HashMap::new();
    emissions.insert(first_path.clone(), sec);
    let def_path = first_path.clone();
    let out = rebuild(
        sections,
        &hunks,
        &claimed_hunks,
        &preserved,
        &sec_paths,
        &emissions,
    );
    Some((out, def_path, rule))
}

/// WI-12 entry for `mod.rs`: assoc, trait-method, then inherent-method
/// cross-file sweeps with provenance commits. Keeps the `mod.rs` wire
/// delta to one call.
pub(crate) fn try_cross_intents(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
    prov: &mut Vec<super::prov::HunkProvenance>,
    compressed_any: &mut bool,
) -> Option<Vec<String>> {
    let mut current: Vec<String> = sections.to_vec();
    let mut fired = false;
    let before_assoc = used_rules.len();
    if let Some((rewritten, def_path, _)) = try_assoc_intent_sweep(&current, pre, used_rules) {
        current = super::prov::commit_pre_pass(
            prov,
            &current,
            rewritten,
            &def_path,
            "assoc_rename",
            before_assoc,
            used_rules.as_slice(),
            pre,
            compressed_any,
        );
        fired = true;
    }
    let before_tm = used_rules.len();
    if let Some((rewritten, def_path, _)) = try_trait_method_intent_sweep(&current, pre, used_rules)
    {
        current = super::prov::commit_pre_pass(
            prov,
            &current,
            rewritten,
            &def_path,
            "trait_method_rename",
            before_tm,
            used_rules.as_slice(),
            pre,
            compressed_any,
        );
        fired = true;
    }
    let before_inh = used_rules.len();
    if let Some((rewritten, def_path, _)) =
        try_inherent_method_intent_sweep(&current, pre, used_rules)
    {
        current = super::prov::commit_pre_pass(
            prov,
            &current,
            rewritten,
            &def_path,
            "inherent_method_rename",
            before_inh,
            used_rules.as_slice(),
            pre,
            compressed_any,
        );
        fired = true;
    }
    fired.then_some(current)
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use super::*;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
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

    const ASSOC_A: &str = "trait Tr {\n    type Item;\n}\n";
    const ASSOC_B: &str = "struct S;\nimpl Tr for S {\n    type Item = u8;\n}\nfn f() -> S::Item {\n    S::Item::default()\n}\n";

    fn assoc_sections() -> Vec<String> {
        vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1 @@\n-    type Item;\n+    type Output;\n",
            ),
            section(
                "b.rs",
                concat!(
                    "@@ -3,1 +3,1 @@\n",
                    "-    type Item = u8;\n",
                    "+    type Output = u8;\n",
                    "@@ -5,1 +5,1 @@\n",
                    "-fn f() -> S::Item {\n",
                    "+fn f() -> S::Output {\n",
                    "@@ -6,1 +6,1 @@\n",
                    "-    S::Item::default()\n",
                    "+    S::Output::default()\n",
                ),
            ),
        ]
    }

    #[test]
    fn assoc_cross_file_fires_one_intent() {
        let sections = assoc_sections();
        let pre = pre_map(&[("a.rs", ASSOC_A), ("b.rs", ASSOC_B)]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_assoc_intent_sweep(&sections, &pre, &mut used).expect("assoc must fire");
        assert_eq!(path, "a.rs");
        // Two def-file sections (one per def file), one shared rule.
        assert_eq!(out.len(), 2, "got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert_eq!(
            out.iter().filter(|s| s.contains("# follow=uses")).count(),
            1,
            "follow exactly once, got: {out:?}"
        );
        let joint = out.join("");
        assert_eq!(joint.matches("rule=").count(), 2, "got:\n{joint}");
        validated_live(&joint, &pre);
        let parsed = rsmpl_patch::parse_patch(&joint).unwrap();
        let mut files = pre.clone();
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            report.rules.iter().all(|r| r.orphans.is_empty()),
            "must verify, got: {:?}",
            report.rules.iter().map(|r| &r.orphans).collect::<Vec<_>>()
        );
        assert!(!files["a.rs"].contains("Item"), "got: {}", files["a.rs"]);
        assert!(!files["b.rs"].contains("Item"), "got: {}", files["b.rs"]);
        assert!(
            files["b.rs"].contains("S::Output::default()"),
            "got: {}",
            files["b.rs"]
        );
    }

    #[test]
    fn assoc_cross_file_pipeline_compresses() {
        let sections = assoc_sections();
        let pre = pre_map(&[("a.rs", ASSOC_A), ("b.rs", ASSOC_B)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- type Item;"), "got:\n{out}");
        validated_live(&out, &pre);
    }

    #[test]
    fn assoc_single_section_stays_with_per_section_detector() {
        // One def + uses in a single section: the whole-section
        // detector owns it; the cross-file pre-pass declines.
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    type Item;\n",
                "+    type Output;\n",
                "@@ -3,1 +3,1 @@\n",
                "-    type Item = u8;\n",
                "+    type Output = u8;\n",
            ),
        );
        let pre = pre_map(&[(
            "f.rs",
            "trait Tr {\n    type Item;\n}\nstruct S;\nimpl Tr for S {\n    type Item = u8;\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_assoc_intent_sweep(std::slice::from_ref(&raw), &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn assoc_bare_use_vetoes() {
        // A bare `Item` use is opaque to the follow: the probe fails,
        // the whole intent stays ordinary.
        let sections = vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1 @@\n-    type Item;\n+    type Output;\n",
            ),
            section(
                "b.rs",
                concat!(
                    "@@ -3,1 +3,1 @@\n",
                    "-    type Item = u8;\n",
                    "+    type Output = u8;\n",
                    "@@ -5,1 +5,1 @@\n",
                    "-    Item\n",
                    "+    Output\n",
                ),
            ),
        ];
        let pre = pre_map(&[
            ("a.rs", ASSOC_A),
            (
                "b.rs",
                "struct S;\nimpl Tr for S {\n    type Item = u8;\n}\nfn f() -> S::Item {\n    Item\n}\n",
            ),
        ]);
        let mut used = Vec::new();
        assert!(try_assoc_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn assoc_two_trait_vetoes() {
        let sections = vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1 @@\n-    type Item;\n+    type Output;\n",
            ),
            section(
                "b.rs",
                "@@ -2,1 +2,1 @@\n-    type Item;\n+    type Output;\n",
            ),
        ];
        let pre = pre_map(&[
            ("a.rs", "trait A {\n    type Item;\n}\n"),
            ("b.rs", "trait B {\n    type Item;\n}\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_assoc_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn assoc_third_file_off_diff_use_vetoes() {
        // `c.rs` names `S::Item` but the diff does not touch it: the
        // unchanged-leg fails and the intent stays ordinary.
        let mut sections = assoc_sections();
        sections.push(section("c.rs", "@@ -1,1 +1,1 @@\n-// touch\n+// touch2\n"));
        let pre = pre_map(&[
            ("a.rs", ASSOC_A),
            ("b.rs", ASSOC_B),
            (
                "c.rs",
                "// touch\nfn g() -> S::Item {\n    S::Item::default()\n}\n",
            ),
        ]);
        let mut used = Vec::new();
        assert!(try_assoc_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    const TM_A: &str = "trait Tr {\n    fn run(&self);\n}\n";
    const TM_B: &str = "struct S;\nimpl Tr for S {\n    fn run(&self) {\n        let _ = self;\n    }\n}\nfn f(s: S) {\n    s.run();\n}\n";

    fn tm_sections() -> Vec<String> {
        vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1 @@\n-    fn run(&self);\n+    fn walk(&self);\n",
            ),
            section(
                "b.rs",
                concat!(
                    "@@ -3,1 +3,1 @@\n",
                    "-    fn run(&self) {\n",
                    "+    fn walk(&self) {\n",
                    "@@ -8,1 +8,1 @@\n",
                    "-    s.run();\n",
                    "+    s.walk();\n",
                ),
            ),
        ]
    }

    #[test]
    fn trait_method_cross_file_fires_one_intent() {
        let sections = tm_sections();
        let pre = pre_map(&[("a.rs", TM_A), ("b.rs", TM_B)]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_trait_method_intent_sweep(&sections, &pre, &mut used).expect("tm must fire");
        assert_eq!(path, "a.rs");
        // Single-hunk shape: the trait hunk + follow owns every remote
        // def and call, so the collapsed use section vanishes.
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert_eq!(
            out.iter().filter(|s| s.contains("# follow=uses")).count(),
            1,
            "follow exactly once, got: {out:?}"
        );
        let joint = out.join("");
        validated_live(&joint, &pre);
        let parsed = rsmpl_patch::parse_patch(&joint).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(files["a.rs"].contains("fn walk"), "got: {}", files["a.rs"]);
        assert!(files["b.rs"].contains("fn walk"), "got: {}", files["b.rs"]);
        assert!(files["b.rs"].contains("s.walk()"), "got: {}", files["b.rs"]);
        assert!(!files["a.rs"].contains("run"), "got: {}", files["a.rs"]);
        assert!(!files["b.rs"].contains("run"), "got: {}", files["b.rs"]);
    }

    #[test]
    fn trait_method_cross_file_pipeline_compresses() {
        let sections = tm_sections();
        let pre = pre_map(&[("a.rs", TM_A), ("b.rs", TM_B)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("fn walk"), "got:\n{out}");
        validated_live(&out, &pre);
        // No ordinary def hunks survive alongside the intent.
        assert!(!out.contains("-    fn run(&self);"), "got:\n{out}");
    }

    #[test]
    fn trait_method_inherent_def_vetoes() {
        // An inherent `impl S` def is a different item (WI-22): the
        // whole intent stays ordinary.
        let sections = vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1 @@\n-    fn run(&self);\n+    fn walk(&self);\n",
            ),
            section(
                "b.rs",
                "@@ -3,1 +3,1 @@\n-    fn run(&self) {\n+    fn walk(&self) {\n",
            ),
        ];
        let pre = pre_map(&[
            ("a.rs", TM_A),
            (
                "b.rs",
                "struct S;\nimpl S {\n    fn run(&self) {\n        let _ = self;\n    }\n}\n",
            ),
        ]);
        let mut used = Vec::new();
        assert!(try_trait_method_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn trait_method_single_section_declines() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    fn run(&self);\n",
                "+    fn walk(&self);\n",
                "@@ -6,1 +6,1 @@\n",
                "-    fn run(&self) {\n",
                "+    fn walk(&self) {\n",
                "@@ -10,1 +10,1 @@\n",
                "-    s.run();\n",
                "+    s.walk();\n",
            ),
        );
        let pre = pre_map(&[(
            "f.rs",
            "trait Tr {\n    fn run(&self);\n}\nstruct S;\nimpl Tr for S {\n    fn run(&self) {\n        let _ = self;\n    }\n}\nfn f(s: S) {\n    s.run();\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(
            try_trait_method_intent_sweep(std::slice::from_ref(&raw), &pre, &mut used).is_none()
        );
        assert!(used.is_empty());
    }

    #[test]
    fn trait_method_third_file_off_diff_call_vetoes() {
        let mut sections = tm_sections();
        sections.push(section("c.rs", "@@ -1,1 +1,1@@\n-// touch\n+// touch2\n"));
        let pre = pre_map(&[
            ("a.rs", TM_A),
            ("b.rs", TM_B),
            (
                "c.rs",
                "// touch\nstruct T;\nimpl Tr for T {\n    fn run(&self) {\n        let _ = self;\n    }\n}\nfn g(t: T) {\n    t.run();\n}\n",
            ),
        ]);
        // The follow would rewrite `c.rs`'s `fn run` def + `t.run()`
        // call, but the native diff leaves them: the unchanged-leg
        // fails and the whole intent stays ordinary.
        let mut used = Vec::new();
        assert!(try_trait_method_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    const INH_A: &str =
        "struct S;\nimpl S {\n    fn old(&self) {\n        let _ = self;\n    }\n}\n";
    const INH_B: &str = "fn f(s: S) {\n    s.old();\n}\n";

    fn inh_sections() -> Vec<String> {
        vec![
            section(
                "a.rs",
                "@@ -3,1 +3,1@@\n-    fn old(&self) {\n+    fn new(&self) {\n",
            ),
            section("b.rs", "@@ -2,1 +2,1@@\n-    s.old();\n+    s.new();\n"),
        ]
    }

    #[test]
    fn inherent_cross_file_fires_one_intent() {
        let sections = inh_sections();
        let pre = pre_map(&[("a.rs", INH_A), ("b.rs", INH_B)]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_inherent_method_intent_sweep(&sections, &pre, &mut used).expect("must fire");
        assert_eq!(path, "a.rs");
        // Single-hunk shape: the impl hunk + follow owns the remote
        // call, so the collapsed use section vanishes.
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert_eq!(
            out.iter().filter(|s| s.contains("# follow=uses")).count(),
            1,
            "follow exactly once, got: {out:?}"
        );
        let joint = out.join("");
        validated_live(&joint, &pre);
        let parsed = rsmpl_patch::parse_patch(&joint).unwrap();
        let mut files = pre.clone();
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            report.rules.iter().all(|r| r.orphans.is_empty()),
            "must verify, got: {:?}",
            report.rules.iter().map(|r| &r.orphans).collect::<Vec<_>>()
        );
        assert!(files["a.rs"].contains("fn new"), "got: {}", files["a.rs"]);
        assert!(files["b.rs"].contains("s.new()"), "got: {}", files["b.rs"]);
        assert!(!files["a.rs"].contains("old"), "got: {}", files["a.rs"]);
        assert!(!files["b.rs"].contains("s.old()"), "got: {}", files["b.rs"]);
    }

    #[test]
    fn inherent_cross_file_pipeline_compresses() {
        let sections = inh_sections();
        let pre = pre_map(&[("a.rs", INH_A), ("b.rs", INH_B)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("fn new"), "got:\n{out}");
        validated_live(&out, &pre);
        assert!(!out.contains("-    fn old(&self) {"), "got:\n{out}");
    }

    #[test]
    fn inherent_two_self_types_veto() {
        let sections = vec![
            section(
                "a.rs",
                "@@ -3,1 +3,1@@\n-    fn old(&self) {\n+    fn new(&self) {\n",
            ),
            section(
                "c.rs",
                "@@ -3,1 +3,1@@\n-    fn old(&self) {\n+    fn new(&self) {\n",
            ),
            section("b.rs", "@@ -2,1 +2,1@@\n-    s.old();\n+    s.new();\n"),
        ];
        let pre = pre_map(&[
            ("a.rs", INH_A),
            (
                "c.rs",
                "struct T;\nimpl T {\n    fn old(&self) {\n        let _ = self;\n    }\n}\n",
            ),
            ("b.rs", INH_B),
        ]);
        let mut used = Vec::new();
        assert!(try_inherent_method_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn inherent_trait_def_vetoes() {
        // A `trait`-block decl is the trait sweep's domain, never this one.
        let sections = vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1@@\n-    fn old(&self);\n+    fn new(&self);\n",
            ),
            section("b.rs", "@@ -2,1 +2,1@@\n-    s.old();\n+    s.new();\n"),
        ];
        let pre = pre_map(&[
            ("a.rs", "trait Tr {\n    fn old(&self);\n}\n"),
            ("b.rs", INH_B),
        ]);
        let mut used = Vec::new();
        assert!(try_inherent_method_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn inherent_single_section_declines() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -3,1 +3,1@@\n",
                "-    fn old(&self) {\n",
                "+    fn new(&self) {\n",
                "@@ -8,1 +8,1@@\n",
                "-    s.old();\n",
                "+    s.new();\n",
            ),
        );
        let pre = pre_map(&[(
            "f.rs",
            "struct S;\nimpl S {\n    fn old(&self) {\n        let _ = self;\n    }\n}\nfn f(s: S) {\n    s.old();\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(
            try_inherent_method_intent_sweep(std::slice::from_ref(&raw), &pre, &mut used).is_none()
        );
        assert!(used.is_empty());
    }

    #[test]
    fn inherent_third_file_off_diff_call_vetoes() {
        let mut sections = inh_sections();
        sections.push(section("c.rs", "@@ -1,1 +1,1@@\n-// touch\n+// touch2\n"));
        let pre = pre_map(&[
            ("a.rs", INH_A),
            ("b.rs", INH_B),
            (
                "c.rs",
                "// touch\nstruct S2;\nimpl S2 {\n    fn old(&self) {\n        let _ = self;\n    }\n}\nfn g(s: S) {\n    s.old();\n}\n",
            ),
        ]);
        // `c.rs` holds an unclaimed same-name call the follow would
        // rewrite: the unchanged-leg fails and the intent stays ordinary.
        // (Note: `S2::old` is a different item and correctly ignored;
        // the `s.old()` call on `S` is the blocker.)
        let mut used = Vec::new();
        assert!(try_inherent_method_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }
}
