//! Move emission rebuild (D5 split): naming + section rebuild.
//!
//! Byte-identical extraction of the emission tail out of `moves.rs`
//! (D5: that file sat at 2013/2000). When the pair gates settle
//! `final_pairs`, this names every pair (one cut+bind per distinct
//! minus item, one paste per pair, collections planned first), rewrites
//! each touched section once, and applies the global T375 byte-win
//! guard. Zero behavior change.

use std::collections::{HashMap, HashSet};

use super::move_parts::{line_text, MovePair, SectionParts};

/// The `rule=<name>` token of an emitted cut hunk (`None` when the
/// header shape is unexpected — the join then stays split).
fn rule_of(cut: &str) -> Option<String> {
    let (_, rest) = cut.split_once("rule=")?;
    Some(rest.lines().next()?.split_whitespace().next()?.to_string())
}

/// Name every pair, then rebuild each touched section once. Returns the
/// rebuilt section list when the emission beats the replaced bytes
/// (T375), else `None` (caller releases the reserved rule names).
/// WI-15 residue: `mod_join` splices a `mod` pair under the first cut's
/// rule (collection-free moves sharing one file pair only); `None` keeps
/// today's split emission.
pub(crate) fn rebuild_sections(
    mut final_pairs: Vec<MovePair>,
    parsed: &[SectionParts],
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
    checkpoint: usize,
    mod_join: Option<&super::move_mod_join::ModJoin>,
) -> Option<Vec<String>> {
    final_pairs.sort_by_key(|p| (p.minus_hunk_key.0, p.minus_hunk_key.1, p.item.first));
    // Name every pair, then rebuild each touched section once. Fan-out
    // shares one cut+bind per distinct minus item (one cut, N pastes);
    // each paste keeps its own rule name.
    let mut named: Vec<(MovePair, String, String)> = Vec::new();
    let mut bind_seq: usize = 0;
    // WI-6: collection cells plan first (sibling module keeps moves.rs
    // net-flat); planned pairs share one cut, paste on the primary only.
    let collections =
        super::move_collection::plan_collections(&final_pairs, parsed, used_rules, &mut bind_seq);
    let mut bind_of: HashMap<(usize, usize, usize, usize), (String, String)> = HashMap::new();
    // WI-15 residue: the join rides the first cut's rule (one file
    // pair, collection-free moves only — wider shapes keep today's
    // names; `find_mod_join` already proved the file pair).
    let join_pending = mod_join.is_some() && !final_pairs.is_empty() && collections.is_empty();
    let mut join_cut_rule: Option<String> = None;
    for (pair_idx, p) in final_pairs.iter().enumerate() {
        if let Some((cell, is_primary)) = super::move_collection::cell_for(&collections, pair_idx) {
            let paste = if is_primary {
                cell.paste.clone()
            } else {
                String::new()
            };
            named.push((p.clone(), cell.cut.clone(), paste));
            continue;
        }
        let key = (
            p.minus_hunk_key.0,
            p.minus_hunk_key.1,
            p.item.first,
            p.item.last,
        );
        let (bind, cut) = if let Some((b, c)) = bind_of.get(&key) {
            (b.clone(), c.clone())
        } else {
            let cut_rule = super::unique_rule(used_rules, &format!("move_{}", p.item.ident));
            bind_seq += 1;
            let bind = format!("M{bind_seq}");
            let hunk = &parsed[p.minus_hunk_key.0].hunks[p.minus_hunk_key.1];
            let start = hunk.old_start + p.item.first as u32;
            let len = (p.item.last - p.item.first + 1) as u32;
            let cut = format!("@@ -{start},{len} @@ rule={cut_rule}\n# bind={bind}\n- I\n");
            bind_of.insert(key, (bind.clone(), cut.clone()));
            (bind, cut)
        };
        // WI-15 residue: capture the first cut's rule for the first
        // paste and the `mod` pair to share (later pastes keep their
        // own rule names).
        let is_first_cut = join_cut_rule.is_none();
        if join_pending && is_first_cut {
            join_cut_rule = rule_of(&cut);
        }
        let paste_rule = if join_pending && is_first_cut {
            // The first paste shares the first cut's rule (named just
            // above; the sorted loop always names the first cut first).
            join_cut_rule.clone().unwrap_or_else(|| {
                super::unique_rule(used_rules, &format!("paste_{}", p.item.ident))
            })
        } else {
            super::unique_rule(used_rules, &format!("paste_{}", p.item.ident))
        };
        let anchor = parsed[p.plus_hunk_key.0].hunks[p.plus_hunk_key.1].old_start;
        // WI-8a + WI-8 slices 1-9: evolved pairs paste the plus-side
        // header template (the engine splices the captured body under
        // it); identical pairs keep the bare-`bind` verbatim paste.
        let paste = match &p.evolved {
            Some(ev) => {
                // WI-8 slice 9: a concrete template overrides the
                // kind-built abstract shape (signature-evolving `fn`
                // moves carry the plus-side header verbatim).
                let tmpl = match &ev.template {
                    Some(t) => t.clone(),
                    None => match ev.kind {
                        super::move_parts::EvolvedKind::Fn => {
                            super::move_evolve::evolution_template(&ev.prefix, &ev.name)
                        }
                        super::move_parts::EvolvedKind::Struct => {
                            super::move_evolve::struct_evolution_template(&ev.prefix, &ev.name)
                        }
                        super::move_parts::EvolvedKind::Enum => {
                            super::move_evolve::enum_evolution_template(&ev.prefix, &ev.name)
                        }
                        super::move_parts::EvolvedKind::Union => {
                            super::move_evolve::union_evolution_template(&ev.prefix, &ev.name)
                        }
                        super::move_parts::EvolvedKind::Type => {
                            super::move_evolve::type_evolution_template(&ev.prefix, &ev.name)
                        }
                        super::move_parts::EvolvedKind::Const => {
                            super::move_evolve::const_evolution_template(&ev.prefix, &ev.name)
                        }
                        super::move_parts::EvolvedKind::Static => {
                            super::move_evolve::static_evolution_template(
                                &ev.prefix,
                                &ev.name,
                                ev.static_mut,
                            )
                        }
                        super::move_parts::EvolvedKind::Mod => {
                            super::move_evolve::mod_evolution_template(&ev.prefix, &ev.name)
                        }
                        super::move_parts::EvolvedKind::Impl => {
                            debug_assert!(ev.template.is_some());
                            // Template always set above; fallback keeps
                            // the build total (unreachable in practice).
                            ev.template.clone().unwrap_or_else(|| {
                                rsmpl_pattern::impl_body_evolution_template(
                                    &ev.prefix,
                                    &p.item.ident,
                                    &ev.name,
                                )
                            })
                        }
                        super::move_parts::EvolvedKind::Trait => {
                            super::move_evolve::trait_evolution_template(&ev.prefix, &ev.name)
                        }
                    },
                };
                // WI-19 slice 2: a concrete template may span lines
                // (multi-line plus header verbatim); each template line
                // becomes its own `+ ` line. Single-line output is
                // byte-identical to before. Continuation lines keep
                // Git verbatim form (`+` + content, no separator
                // space): the patch parser strips exactly one marker
                // char, so a separator would indent every continuation
                // line by one.
                let plus_lines = tmpl
                    .lines()
                    .enumerate()
                    .map(|(i, l)| {
                        if i == 0 {
                            format!("+ {l}\n")
                        } else {
                            format!("+{l}\n")
                        }
                    })
                    .collect::<String>();
                format!("@@ -{anchor},0 @@ rule={paste_rule}\n# use={bind}\n{plus_lines}")
            }
            None => format!("@@ -{anchor},0 @@ rule={paste_rule}\n# use={bind}\n+ {bind}\n"),
        };
        named.push((p.clone(), cut, paste));
    }
    // WI-15 residue: settle the join — the `mod` pair rides the first
    // cut's rule. Anything off (collection cell, unparsed cut header)
    // keeps today's split emission.
    let join: Option<(&super::move_mod_join::ModJoin, String)> = match (mod_join, &join_cut_rule) {
        (Some(j), Some(rule)) => Some((j, rule.clone())),
        _ => None,
    };
    let mut out_sections = sections.to_vec();
    // Replaced bytes (dropped minus hunks + original plus hunks) vs
    // emitted bytes (cuts + pastes + rewritten leftover hunks).
    let mut replaced: usize = 0;
    let mut emitted: usize = 0;
    let mut touched_minus: Vec<(usize, usize)> = Vec::new();
    let mut touched_plus: Vec<(usize, usize)> = Vec::new();
    for (p, _, _) in &named {
        if !touched_minus.contains(&p.minus_hunk_key) {
            touched_minus.push(p.minus_hunk_key);
        }
        if !touched_plus.contains(&p.plus_hunk_key) {
            touched_plus.push(p.plus_hunk_key);
        }
    }
    for (si, hi) in &touched_minus {
        let h = &parsed[*si].hunks[*hi];
        replaced += h.header.len() + 1 + h.body.iter().map(|l| l.len() + 1).sum::<usize>();
    }
    for (si, hi) in &touched_plus {
        let h = &parsed[*si].hunks[*hi];
        replaced += h.header.len() + 1 + h.body.iter().map(|l| l.len() + 1).sum::<usize>();
    }
    // WI-15 residue: the joined `mod` group counts as replaced bytes
    // (its native lines leave the section like any claimed hunk). No
    // original carried (never for run-split groups) declines the join.
    if let Some((j, _)) = &join {
        replaced += super::move_mod_join::replaced_bytes(parsed, j)?;
    }
    // touched sections, in document order
    let mut touched_secs: Vec<usize> = Vec::new();
    for (si, _) in touched_minus.iter().chain(touched_plus.iter()) {
        if !touched_secs.contains(si) {
            touched_secs.push(*si);
        }
    }
    if let Some((j, _)) = &join {
        if !touched_secs.contains(&j.sec_idx) {
            touched_secs.push(j.sec_idx);
        }
    }
    for si in &touched_secs {
        let sec = &parsed[*si];
        // WI-15 residue: the joined `mod` section re-emits with the
        // pair replaced by the shared-rule semantic hunk; every other
        // hunk rides verbatim.
        if let Some((j, rule)) = &join {
            if *si == j.sec_idx {
                // WI-15 residue: the two joined `mod` runs are replaced
                // by the shared-rule semantic hunk (emitted once at the
                // first run); other runs of a mixed group re-emit as
                // ordinary run hunks (exact run anchors), every other
                // run-group re-emits its untouched original once and
                // every pure hunk rides verbatim.
                let mut rebuilt = sec.prelude.clone();
                let mut mod_done = false;
                let mut group_done: HashSet<usize> = HashSet::new();
                for (hi, hunk) in sec.hunks.iter().enumerate() {
                    if hunk.group == Some(j.group) {
                        if hi == j.mod_minus_hi || hi == j.mod_plus_hi {
                            // `mod`-carrying run: leftover spans around
                            // the `mod` line stay ordinary (exact ranges,
                            // run anchors reused per the leftover
                            // convention below); the semantic hunk fires
                            // once at the first run.
                            let is_minus = hi == j.mod_minus_hi;
                            let midx = if is_minus {
                                j.mod_minus_idx
                            } else {
                                j.mod_plus_idx
                            };
                            let span = |lo: usize, len: usize| {
                                let header = if is_minus {
                                    format!(
                                        "@@ -{},{} +{},0 @@",
                                        hunk.old_start + lo as u32,
                                        len,
                                        hunk.new_start
                                    )
                                } else {
                                    format!(
                                        "@@ -{},0 +{},{} @@",
                                        hunk.old_start,
                                        hunk.new_start + lo as u32,
                                        len
                                    )
                                };
                                (header, &hunk.body[lo..lo + len])
                            };
                            if midx > 0 {
                                let (header, lines) = span(0, midx);
                                emitted += header.len()
                                    + 1
                                    + lines.iter().map(|l| l.len() + 1).sum::<usize>();
                                rebuilt.push_str(&header);
                                rebuilt.push('\n');
                                for line in lines {
                                    rebuilt.push_str(line);
                                    rebuilt.push('\n');
                                }
                            }
                            if !mod_done {
                                let mh = super::move_mod_join::mod_hunk_text(j, rule);
                                emitted += mh.len();
                                rebuilt.push_str(&mh);
                                mod_done = true;
                            }
                            if midx + 1 < hunk.body.len() {
                                let len = hunk.body.len() - midx - 1;
                                let (header, lines) = span(midx + 1, len);
                                emitted += header.len()
                                    + 1
                                    + lines.iter().map(|l| l.len() + 1).sum::<usize>();
                                rebuilt.push_str(&header);
                                rebuilt.push('\n');
                                for line in lines {
                                    rebuilt.push_str(line);
                                    rebuilt.push('\n');
                                }
                            }
                            continue;
                        }
                        // Mixed-group leftover run: ordinary hunk with
                        // the run's own header + body.
                        emitted += hunk.header.len()
                            + 1
                            + hunk.body.iter().map(|l| l.len() + 1).sum::<usize>();
                        rebuilt.push_str(&hunk.header);
                        rebuilt.push('\n');
                        for line in &hunk.body {
                            rebuilt.push_str(line);
                            rebuilt.push('\n');
                        }
                        continue;
                    }
                    if let Some((orig_header, orig_body)) = &hunk.unsplit {
                        let gid = hunk.group.unwrap_or(hi);
                        if group_done.contains(&gid) {
                            continue;
                        }
                        group_done.insert(gid);
                        rebuilt.push_str(orig_header);
                        rebuilt.push('\n');
                        for line in orig_body {
                            rebuilt.push_str(line);
                            rebuilt.push('\n');
                        }
                        continue;
                    }
                    rebuilt.push_str(&hunk.header);
                    rebuilt.push('\n');
                    for line in &hunk.body {
                        rebuilt.push_str(line);
                        rebuilt.push('\n');
                    }
                }
                out_sections[sec.orig_idx] = rebuilt;
                continue;
            }
        }
        let mut rebuilt = sec.prelude.clone();
        // WI-6 slice 3a: runs of a run-split mixed hunk whose whole
        // group is untouched re-emit the original verbatim once.
        let mut group_done: HashSet<usize> = HashSet::new();
        for (hi, hunk) in sec.hunks.iter().enumerate() {
            if let Some((orig_header, orig_body)) = &hunk.unsplit {
                let gid = hunk.group.unwrap_or(hi);
                if group_done.contains(&gid) {
                    continue;
                }
                let group_touched = named.iter().any(|(p, _, _)| {
                    [p.minus_hunk_key, p.plus_hunk_key]
                        .iter()
                        .any(|(ksi, khi)| {
                            *ksi == *si
                                && sec.hunks[*khi].group == hunk.group
                                && sec.hunks[*khi].unsplit.is_some()
                        })
                });
                if !group_touched {
                    rebuilt.push_str(orig_header);
                    rebuilt.push('\n');
                    for line in orig_body {
                        rebuilt.push_str(line);
                        rebuilt.push('\n');
                    }
                    group_done.insert(gid);
                    continue;
                }
            }
            let mut cuts: Vec<&String> = Vec::new();
            for (p, c, _) in &named {
                if p.minus_hunk_key == (*si, hi) && !cuts.contains(&c) {
                    cuts.push(c);
                }
            }
            if !cuts.is_empty() {
                // Prefix leftovers stay ordinary before the cuts (T432c;
                // disjoint old-range deletions, SPEC 10.6 option-1).
                // Prefix-trim: a single item with a blank-ending prefix
                // and no following blank has its cut absorb the preceding
                // blank, so that one line leaves the leftover hunk.
                let mlo = named
                    .iter()
                    .filter(|(p, _, _)| p.minus_hunk_key == (*si, hi))
                    .map(|(p, _, _)| p.item.first)
                    .min()
                    .unwrap();
                let mhi = named
                    .iter()
                    .filter(|(p, _, _)| p.minus_hunk_key == (*si, hi))
                    .map(|(p, _, _)| p.item.last)
                    .max()
                    .unwrap();
                let mut plo = mlo;
                if plo > 0 && line_text(&hunk.body[plo - 1]).trim().is_empty() {
                    let mut distinct: Vec<(usize, usize)> = named
                        .iter()
                        .filter(|(p, _, _)| p.minus_hunk_key == (*si, hi))
                        .map(|(p, _, _)| (p.item.first, p.item.last))
                        .collect();
                    distinct.sort();
                    distinct.dedup();
                    if distinct.len() == 1 {
                        let follows = if mhi + 1 < hunk.body.len() {
                            line_text(&hunk.body[mhi + 1]).trim().is_empty()
                        } else {
                            pre.get(&sec.path)
                                .and_then(|old| old.lines().nth(hunk.old_start as usize + mhi))
                                .is_some_and(|l| l.trim().is_empty())
                        };
                        if !follows {
                            plo -= 1;
                        }
                    }
                }
                if plo > 0 {
                    let header = format!(
                        "@@ -{},{} +{},{} @@",
                        hunk.old_start, plo, hunk.new_start, 0
                    );
                    emitted += header.len() + 1;
                    rebuilt.push_str(&header);
                    rebuilt.push('\n');
                    for line in hunk.body.iter().take(plo) {
                        emitted += line.len() + 1;
                        rebuilt.push_str(line);
                        rebuilt.push('\n');
                    }
                }
                for c in cuts {
                    emitted += c.len();
                    rebuilt.push_str(c);
                }
                // E1 slice 3: interior gaps between distinct cuts stay
                // ordinary (disjoint old-range deletions, SPEC 10.6
                // option-1). Each non-last cut absorbs its following
                // blank (`delete_lines`), so one leading blank per gap
                // is skipped; the remainder is the removed attr line
                // admitted by `minus_hunk_holds` and re-emits verbatim.
                let mut spans: Vec<(usize, usize)> = named
                    .iter()
                    .filter(|(p, _, _)| p.minus_hunk_key == (*si, hi))
                    .map(|(p, _, _)| (p.item.first, p.item.last))
                    .collect();
                spans.sort();
                spans.dedup();
                for w in spans.windows(2) {
                    let gap_start = w[0].1 + 1;
                    let gap_end = w[1].0;
                    if gap_start >= gap_end || gap_end > hunk.body.len() {
                        continue;
                    }
                    let mut glo = gap_start;
                    if glo < gap_end && line_text(&hunk.body[glo]).trim().is_empty() {
                        glo += 1;
                    }
                    if glo >= gap_end {
                        continue;
                    }
                    let glen = gap_end - glo;
                    let header = format!(
                        "@@ -{},{} +{},{} @@",
                        hunk.old_start + glo as u32,
                        glen,
                        hunk.new_start,
                        0
                    );
                    emitted += header.len() + 1;
                    rebuilt.push_str(&header);
                    rebuilt.push('\n');
                    for line in hunk.body.iter().skip(glo).take(glen) {
                        emitted += line.len() + 1;
                        rebuilt.push_str(line);
                        rebuilt.push('\n');
                    }
                }
                // Trailing leftovers stay ordinary after the cuts; the
                // single blank right after the last paired run is
                // absorbed by the cut, never re-emitted.
                let mut tlo = mhi + 1;
                if tlo < hunk.body.len() && line_text(&hunk.body[tlo]).trim().is_empty() {
                    tlo += 1;
                }
                if tlo < hunk.body.len() {
                    let tlen = hunk.body.len() - tlo;
                    let header = format!(
                        "@@ -{},{} +{},{} @@",
                        hunk.old_start + tlo as u32,
                        tlen,
                        hunk.new_start,
                        0
                    );
                    emitted += header.len() + 1;
                    rebuilt.push_str(&header);
                    rebuilt.push('\n');
                    for line in hunk.body.iter().skip(tlo) {
                        emitted += line.len() + 1;
                        rebuilt.push_str(line);
                        rebuilt.push('\n');
                    }
                }
                continue;
            }
            let pastes: Vec<&String> = named
                .iter()
                .filter(|(p, _, _)| p.plus_hunk_key == (*si, hi))
                .map(|(_, _, q)| q)
                .collect();
            if pastes.is_empty() {
                rebuilt.push_str(&hunk.header);
                rebuilt.push('\n');
                for line in &hunk.body {
                    rebuilt.push_str(line);
                    rebuilt.push('\n');
                }
                continue;
            }
            // Leftover prefix stays ordinary with recomputed counts,
            // pastes follow in document order, then any trailing
            // leftovers stay ordinary (T432b; same-point insertions
            // keep order). A fully consumed hunk drops entirely.
            let lo = named
                .iter()
                .filter(|(p, _, _)| p.plus_hunk_key == (*si, hi))
                .map(|(p, _, _)| p.plus_first)
                .min()
                .unwrap();
            let phi = named
                .iter()
                .filter(|(p, _, _)| p.plus_hunk_key == (*si, hi))
                .map(|(p, _, _)| p.plus_last)
                .max()
                .unwrap();
            if lo > 0 {
                let header = format!(
                    "@@ -{},{} +{},{} @@",
                    hunk.old_start, hunk.old_len, hunk.new_start, lo
                );
                emitted += header.len() + 1;
                rebuilt.push_str(&header);
                rebuilt.push('\n');
                for line in hunk.body.iter().take(lo) {
                    emitted += line.len() + 1;
                    rebuilt.push_str(line);
                    rebuilt.push('\n');
                }
            }
            // E1 slice 4: interior gaps between distinct plus runs stay
            // ordinary (disjoint new-range insertions, SPEC 10.6 option-1).
            // Each same-anchor paste join restores exactly one blank
            // between pastes, so one leading blank per gap is skipped;
            // the remainder is the replaced-axis new attr line admitted by
            // `plus_runs_hold` and re-emits verbatim. Pure single-blank
            // gaps emit nothing (skip consumes them).
            let mut runs: Vec<(usize, usize)> = named
                .iter()
                .filter(|(p, _, _)| p.plus_hunk_key == (*si, hi))
                .map(|(p, _, _)| (p.plus_first, p.plus_last))
                .collect();
            runs.sort();
            runs.dedup();
            for (ri, run) in runs.iter().enumerate() {
                for (_, _, q) in named.iter().filter(|(p, _, _)| {
                    p.plus_hunk_key == (*si, hi) && (p.plus_first, p.plus_last) == *run
                }) {
                    emitted += q.len();
                    rebuilt.push_str(q);
                }
                if ri + 1 >= runs.len() {
                    continue;
                }
                let gap_start = run.1 + 1;
                let gap_end = runs[ri + 1].0;
                if gap_start >= gap_end || gap_end > hunk.body.len() {
                    continue;
                }
                let mut glo = gap_start;
                if glo < gap_end && line_text(&hunk.body[glo]).trim().is_empty() {
                    glo += 1;
                }
                if glo >= gap_end {
                    continue;
                }
                let glen = gap_end - glo;
                let header = format!(
                    "@@ -{},{} +{},{} @@",
                    hunk.old_start,
                    hunk.old_len,
                    hunk.new_start + glo as u32,
                    glen
                );
                emitted += header.len() + 1;
                rebuilt.push_str(&header);
                rebuilt.push('\n');
                for line in hunk.body.iter().skip(glo).take(glen) {
                    emitted += line.len() + 1;
                    rebuilt.push_str(line);
                    rebuilt.push('\n');
                }
            }
            if phi + 1 < hunk.body.len() {
                let tlo = phi + 1;
                let tlen = hunk.body.len() - tlo;
                let header = format!(
                    "@@ -{},{} +{},{} @@",
                    hunk.old_start,
                    hunk.old_len,
                    hunk.new_start + tlo as u32,
                    tlen
                );
                emitted += header.len() + 1;
                rebuilt.push_str(&header);
                rebuilt.push('\n');
                for line in hunk.body.iter().skip(tlo) {
                    emitted += line.len() + 1;
                    rebuilt.push_str(line);
                    rebuilt.push('\n');
                }
            }
        }
        out_sections[sec.orig_idx] = rebuilt;
    }
    // Global T375 byte-win guard over the replaced set.
    if emitted >= replaced {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some(out_sections)
}
