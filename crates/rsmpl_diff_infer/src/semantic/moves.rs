//! Whole-item 1-to-N move/copy inference (T430; 1-to-N fan-out T432).
//!
//! `diffrs` emitted 873 ordinary lines for 1b8993a (four byte-identical
//! items moved `matcher_fn.rs` -> new `matcher_impl.rs`): no move
//! inference existed. This whole-patch pre-pass pairs byte-identical named
//! items deleted in one file and added in another (or the same file,
//! or a new file) and emits `# item I` + `# bind` cut hunks spliced at
//! the consumed minus hunk plus `# use` paste hunks spliced after the
//! rewritten plus hunk. A hand-authored probe of the 1b8993a shape
//! round-trips byte-identical (`--check` green, exact spans), so the
//! shape is engine-legal — `diffrs` just never emitted it.
//!
//! v1 fires only on the probe-proven shape; everything else stays
//! ordinary by design: named items, pure-minus / pure-plus `-U0`
//! hunks only (mixed-hunk surgery unreachable - the source always
//! runs `git diff --unified=0`), global N-minus/M-plus pairing (one
//! cut per surviving minus item sharing its bind across its pastes;
//! duplicate plus-run claims dedup to the first pair in document
//! order, the rest ride ordinary),
//! cut-file pre-image required with flat-ws uniqueness (twin veto; legacy
//! no-pre path skips), minus paired runs with exactly one blank between
//! runs (a cut absorbs exactly one following blank; multi-cut hunks (distinct
//! items; fan-out pastes of one item need no cap) whose last run has no
//! following blank cap to N-1 cuts, the last run staying
//! an ordinary trailing leftover; a single item with a blank-ending prefix
//! and no following blank trims the absorbed blank from the prefix leftover
//! hunk (prefix-trim, f09edcd);
//! uncovered prefix/trailing minus lines stay ordinary around the cuts
//! (T432c; one post-run blank is absorbed by the cut, never re-emitted),
//! interior blanks ride with their item (the cut removes the whole span,
//! so covered lines need no gap accounting),
//! plus paired runs with exactly one blank between runs (the same-anchor
//! paste join restores exactly one); uncovered prefix/trailing plus lines
//! stay ordinary around pastes (T432b), deleted-file cuts allowed (T432d;
//! the rebuilt prelude keeps the deletion), rename sections skipped,
//! and a global T375 byte-win guard.

use std::collections::{HashMap, HashSet};
use syn::spanned::Spanned;

/// Try the whole-patch move pre-pass: returns the rebuilt section list
/// (same length and order, touched sections rewritten with move hunks
/// spliced in) when at least one pair fires and clears the token-win
/// guard, else `None` (caller keeps the ordinary flow; reserved rule
/// names released).
pub(crate) fn try_move_sweep(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<Vec<String>> {
    let checkpoint = used_rules.len();
    let parsed: Vec<SectionParts> = sections
        .iter()
        .enumerate()
        .filter_map(|(i, s)| split_section(s, i))
        .collect();
    if parsed.is_empty() {
        return None;
    }
    // Candidate named items per pure hunk, keyed by (parsed section idx,
    // hunk idx). Deleted-file sections seed cuts like any other (T432d):
    // the rebuilt section keeps the `deleted file mode` prelude, so the
    // file still goes away once cuts + leftover hunks empty it.
    let mut minus_items: Vec<(usize, usize, Vec<MoveItem>)> = Vec::new();
    let mut plus_hunks: Vec<(usize, usize)> = Vec::new();
    for (si, sec) in parsed.iter().enumerate() {
        for (hi, hunk) in sec.hunks.iter().enumerate() {
            match classify(hunk) {
                Some(Side::Minus) => {
                    if let Some(items) = minus_items_in(&hunk.body) {
                        if !items.is_empty() {
                            minus_items.push((si, hi, items));
                        }
                    }
                }
                Some(Side::Plus) => {
                    // Plus hunks must also parse as whole items (else a
                    // run could align with a sub-range of a bigger item
                    // and splice in the wrong order).
                    if !sec.is_deleted && plus_items_in(&hunk.body).is_some() {
                        plus_hunks.push((si, hi));
                    }
                }
                None => {}
            }
        }
    }
    if minus_items.is_empty() || plus_hunks.is_empty() {
        return None;
    }
    // Global N-minus/M-plus pairing by exact item text (T432 fan-out,
    // T432 dedup): one minus item may match N plus runs; each pair
    // shares the minus cut bind downstream (one cut, N pastes). N
    // distinct minus items may also claim one plus run (byte-identical
    // twins across files - same-file twins never reach here, the
    // uniqueness gate vetoes count != 1); each run needs exactly one
    // paste, so only the first claim survives below.
    let mut pairs: Vec<MovePair> = Vec::new();
    for (msi, mhi, items) in &minus_items {
        for item in items {
            let mut hits: Vec<(usize, usize, usize, usize)> = Vec::new();
            for (psi, phi) in &plus_hunks {
                let pbody = &parsed[*psi].hunks[*phi].body;
                for (first, last) in find_runs(pbody, &item.lines) {
                    hits.push((*psi, *phi, first, last));
                }
            }
            if hits.is_empty() {
                // WI-8a + WI-8 slices 1-9: no byte-identical run — the
                // evolved-header cascade lives in `move_pair_evolve`
                // (D5 split; push order preserved for run-dedup).
                pairs.extend(super::move_pair_evolve::evolved_pairs_for_item(
                    item,
                    &parsed[..],
                    &plus_hunks[..],
                    (*msi, *mhi),
                ));
                continue;
            }
            for (psi, phi, first, last) in hits {
                // The plus run must coincide with a parsed fn-item span
                // (never a sub-range of a larger item).
                if !plus_run_is_item(&parsed[psi].hunks[phi].body, first, last) {
                    continue;
                }
                pairs.push(MovePair {
                    minus_hunk_key: (*msi, *mhi),
                    plus_hunk_key: (psi, phi),
                    item: item.clone(),
                    plus_first: first,
                    plus_last: last,
                    evolved: None,
                });
            }
        }
    }
    if pairs.is_empty() {
        return None;
    }
    // Plus-run claim dedup (2-to-1): keep the first pair per plus
    // run in document order (pairs build in index order, so this is
    // deterministic); dropped claims re-enter the gates below with
    // no pair, leaving their minus hunks ordinary. Same span
    // implies same text - runs are exact text matches - so the run
    // key alone groups identical claims.
    let mut seen_runs: HashSet<(usize, usize, usize, usize)> = HashSet::new();
    let mut deduped: Vec<MovePair> = Vec::new();
    for p in pairs {
        if seen_runs.insert((
            p.plus_hunk_key.0,
            p.plus_hunk_key.1,
            p.plus_first,
            p.plus_last,
        )) {
            deduped.push(p);
        }
    }
    let pairs = deduped;
    // Minus-hunk gate: whole-hunk drop only — every non-blank line must
    // belong to a paired item, no leading blank, no blank runs (a cut
    // absorbs exactly one following blank).
    let mut gated: Vec<MovePair> = Vec::new();
    for (msi, mhi, _) in &minus_items {
        if !minus_hunk_holds(&parsed[*msi].hunks[*mhi].body, &pairs, *msi, *mhi) {
            continue;
        }
        gated.extend(
            pairs
                .iter()
                .filter(|p| p.minus_hunk_key == (*msi, *mhi))
                .cloned(),
        );
    }
    if gated.is_empty() {
        return None;
    }
    // Plus gap gate per hunk: paired runs tile a suffix with exactly one
    // blank between consecutive runs (leftovers form a prefix, so
    // splice-at-position preserves order; other gaps would corrupt order).
    let mut suffixed: Vec<MovePair> = Vec::new();
    for (psi, phi) in &plus_hunks {
        let body = &parsed[*psi].hunks[*phi].body;
        let runs: Vec<(usize, usize)> = gated
            .iter()
            .filter(|p| p.plus_hunk_key == (*psi, *phi))
            .map(|p| (p.plus_first, p.plus_last))
            .collect();
        if !plus_runs_hold(body, &runs) {
            continue;
        }
        suffixed.extend(
            gated
                .iter()
                .filter(|p| p.plus_hunk_key == (*psi, *phi))
                .cloned(),
        );
    }
    if suffixed.is_empty() {
        return None;
    }
    // Pre-image uniqueness gate on the cut side (twin veto; legacy
    // no-pre path skips — same residual as the other detectors).
    let mut sound: Vec<MovePair> = Vec::new();
    for p in &suffixed {
        let path = &parsed[p.minus_hunk_key.0].path;
        let Some(old) = pre.get(path) else { continue };
        if syn::parse_str::<syn::File>(old).is_err() {
            continue;
        }
        let needle = super::flat_ws(&p.item.text);
        if super::count_occurrences(&super::flat_ws(old), &needle) != 1 {
            continue;
        }
        sound.push(p.clone());
    }
    if sound.is_empty() {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Re-check hunk-level gates on the narrowed set (a hunk whose only
    // pairs were vetoed above must not drop; a plus hunk whose runs
    // shrank must still tile a suffix).
    let mut emit: Vec<MovePair> = Vec::new();
    for (msi, mhi, _) in &minus_items {
        let hunk = &parsed[*msi].hunks[*mhi];
        if !minus_hunk_holds(&hunk.body, &sound, *msi, *mhi) {
            continue;
        }
        // Last-cut absorption soundness (f09edcd): the last cut absorbs
        // its following blank, else its preceding blank — which is
        // already claimed (middle-gap blank by the previous cut,
        // blank-ending prefix by the leftover hunk). Needs the pre-image
        // file text; every sound pair verified it, so it exists here.
        let Some(file_old) = pre.get(&parsed[*msi].path) else {
            continue;
        };
        // Multi-pair-at-EOF capping (T432 slice 6): a last run with no
        // following blank vetoed the whole hunk; instead drop the last
        // paired minus item (all its fan-out hits) until the survivors
        // are sound. The dropped run stays an ordinary trailing leftover
        // (the surviving cut absorbs the gap blank). Single-item unsound
        // hunks stay vetoed (prefix-trim deferred).
        let mut cand: Vec<MovePair> = sound
            .iter()
            .filter(|p| p.minus_hunk_key == (*msi, *mhi))
            .cloned()
            .collect();
        while !minus_last_cut_sound(&hunk.body, &cand, *msi, *mhi, hunk.old_start, file_old) {
            let mut spans: Vec<(usize, usize)> =
                cand.iter().map(|p| (p.item.first, p.item.last)).collect();
            spans.sort();
            spans.dedup();
            if spans.len() < 2 {
                break;
            }
            let last = cand.iter().map(|p| p.item.last).max().unwrap();
            cand.retain(|p| p.item.last != last);
            if !minus_hunk_holds(&hunk.body, &cand, *msi, *mhi) {
                cand.clear();
                break;
            }
        }
        if cand.is_empty()
            || !minus_last_cut_sound(&hunk.body, &cand, *msi, *mhi, hunk.old_start, file_old)
        {
            continue;
        }
        emit.extend(cand);
    }
    let mut final_pairs: Vec<MovePair> = Vec::new();
    for (psi, phi) in &plus_hunks {
        let body = &parsed[*psi].hunks[*phi].body;
        let runs: Vec<(usize, usize)> = emit
            .iter()
            .filter(|p| p.plus_hunk_key == (*psi, *phi))
            .map(|p| (p.plus_first, p.plus_last))
            .collect();
        if !plus_runs_hold(body, &runs) {
            continue;
        }
        final_pairs.extend(
            emit.iter()
                .filter(|p| p.plus_hunk_key == (*psi, *phi))
                .cloned(),
        );
    }
    if final_pairs.is_empty() {
        used_rules.truncate(checkpoint);
        return None;
    }
    final_pairs.sort_by_key(|p| (p.minus_hunk_key.0, p.minus_hunk_key.1, p.item.first));
    // Name every pair, then rebuild each touched section once. Fan-out
    // shares one cut+bind per distinct minus item (one cut, N pastes);
    // each paste keeps its own rule name.
    let mut named: Vec<(MovePair, String, String)> = Vec::new();
    let mut bind_seq: usize = 0;
    let mut bind_of: HashMap<(usize, usize, usize, usize), (String, String)> = HashMap::new();
    for p in &final_pairs {
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
        let paste_rule = super::unique_rule(used_rules, &format!("paste_{}", p.item.ident));
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
                        super::move_parts::EvolvedKind::Trait => {
                            super::move_evolve::trait_evolution_template(&ev.prefix, &ev.name)
                        }
                    },
                };
                format!("@@ -{anchor},0 @@ rule={paste_rule}\n# use={bind}\n+ {tmpl}\n")
            }
            None => format!("@@ -{anchor},0 @@ rule={paste_rule}\n# use={bind}\n+ {bind}\n"),
        };
        named.push((p.clone(), cut, paste));
    }
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
    // touched sections, in document order
    let mut touched_secs: Vec<usize> = Vec::new();
    for (si, _) in touched_minus.iter().chain(touched_plus.iter()) {
        if !touched_secs.contains(si) {
            touched_secs.push(*si);
        }
    }
    for si in &touched_secs {
        let sec = &parsed[*si];
        let mut rebuilt = sec.prelude.clone();
        for (hi, hunk) in sec.hunks.iter().enumerate() {
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
            for q in pastes {
                emitted += q.len();
                rebuilt.push_str(q);
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

use super::move_parts::{
    classify, line_text, minus_hunk_holds, minus_last_cut_sound, plus_runs_hold, split_section,
    MoveItem, MovePair, SectionParts, Side,
};

/// Parse a pure-hunk body as a `syn::File`; `None` when any non-blank
/// line is not whole-item content. Returns the named items with
/// body-line spans (fn + struct/enum/trait/type/mod/union/const/static).
fn minus_items_in(body: &[String]) -> Option<Vec<MoveItem>> {
    file_items_in(body, false)
}

/// Lenient plus-side parse (T430): `//`-comment lines (`//`, `///`,
/// `//!` — all trivia outside item spans, e.g. file-header docs above
/// the moved items) are coverable leftovers, never vetoes. Block
/// `/* */` interiors still veto (rare; ordinary fallback).
fn plus_items_in(body: &[String]) -> Option<Vec<MoveItem>> {
    file_items_in(body, true)
}

fn file_items_in(body: &[String], comments_ok: bool) -> Option<Vec<MoveItem>> {
    let texts: Vec<&str> = body.iter().map(|l| line_text(l)).collect();
    let fragment = texts.join("\n");
    let file: syn::File = syn::parse_str(&fragment).ok()?;
    let mut covered = vec![false; body.len()];
    for it in &file.items {
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > body.len() {
            return None;
        }
        covered[(s - 1)..e].fill(true);
    }
    for (i, t) in texts.iter().enumerate() {
        let comment = comments_ok && t.trim_start().starts_with("//");
        if !covered[i] && !t.trim().is_empty() && !comment {
            return None;
        }
    }
    let mut items = Vec::new();
    for it in &file.items {
        // WI-mod-content-cut: the scalar `- I` cut cannot isolate an item
        // with nested items (it would match them too — `expected one match,
        // found N`). Content-bearing inline `mod` (≥1 nested item) and
        // method-bearing `trait` (any trait item) stay ordinary; empty
        // `mod {}` / `mod;` / empty trait still move. Mirrors the
        // empty-only evolved `mod`/`trait` narrowing (WI-8 slices 7-8).
        if !move_cut_eligible(it) {
            continue;
        }
        let Some(ident) = item_ident(it) else {
            continue;
        };
        let s = it.span().start().line;
        let e = it.span().end().line;
        let lines: Vec<String> = texts[(s - 1)..e].iter().map(|s| s.to_string()).collect();
        items.push(MoveItem {
            ident,
            text: lines.join("\n"),
            lines,
            first: s - 1,
            last: e - 1,
        });
    }
    Some(items)
}

/// WI-mod-content-cut: scalar-cut eligibility for byte-identical moves.
///
/// The engine's `- I` cut matches nested items too (`collect_fns` recurses
/// into inline `mod` content; `collect_impl_method_spans` collects trait
/// methods), so cutting a content-bearing `mod` or a method-bearing
/// `trait` always fails with `expected one match, found N`. Such items
/// stay ordinary. Empty inline `mod {}` (`Some` empty), outline
/// `mod;` (`None`), and empty traits stay eligible.
fn move_cut_eligible(it: &syn::Item) -> bool {
    match it {
        syn::Item::Mod(m) => match &m.content {
            None => true,
            Some((_, inner)) => inner.is_empty(),
        },
        syn::Item::Trait(t) => t.items.is_empty(),
        _ => true,
    }
}

/// Single plain ident for named items; `None` for ident-less items
/// (impl blocks, use decls, macros) which stay ordinary.
fn item_ident(it: &syn::Item) -> Option<String> {
    match it {
        syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
        syn::Item::Struct(s) => Some(s.ident.to_string()),
        syn::Item::Enum(e) => Some(e.ident.to_string()),
        syn::Item::Trait(t) => Some(t.ident.to_string()),
        syn::Item::Type(t) => Some(t.ident.to_string()),
        syn::Item::Const(c) => Some(c.ident.to_string()),
        syn::Item::Static(s) => Some(s.ident.to_string()),
        syn::Item::Mod(m) => Some(m.ident.to_string()),
        syn::Item::Union(u) => Some(u.ident.to_string()),
        _ => None,
    }
}

/// All occurrences of the contiguous `needle` line run in `body`
/// (0-based spans); callers require exactly one.
fn find_runs(body: &[String], needle: &[String]) -> Vec<(usize, usize)> {
    if needle.is_empty() || needle.len() > body.len() {
        return Vec::new();
    }
    let texts: Vec<&str> = body.iter().map(|l| line_text(l)).collect();
    let needles: Vec<&str> = needle.iter().map(|s| s.as_str()).collect();
    let mut out = Vec::new();
    for i in 0..=body.len() - needle.len() {
        if texts[i..i + needle.len()] == needles {
            out.push((i, i + needle.len() - 1));
        }
    }
    out
}

/// The plus run must coincide with a parsed fn-item span (never a
/// sub-range of a larger item).
fn plus_run_is_item(body: &[String], first: usize, last: usize) -> bool {
    let Some(items) = plus_items_in(body) else {
        return false;
    };
    items.iter().any(|it| it.first == first && it.last == last)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A long-enough fn body to clear the T375 byte-win guard (a
    /// 3-line item costs more as bind/use than as ordinary lines).
    fn big_fn(name: &str) -> String {
        let mut s = format!("fn {name}() {{\n");
        for i in 0..40 {
            s.push_str(&format!("    let v{i} = {i};\n"));
        }
        s.push_str("}\n");
        s
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

    #[test]
    fn move_fire_cross_file_existing_target() {
        // `big` moves a.rs -> existing b.rs (insert after line 2).
        let big = big_fn("big");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[("a.rs", &a_old), ("b.rs", "use x;\nuse y;\n")]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("must fire");
        let text = out.join("");
        assert!(
            text.contains("@@ -2,42 @@ rule=r1\n# bind=M1\n- I\n"),
            "got:\n{text}"
        );
        assert!(
            text.contains("@@ -2,0 @@ rule=r2\n# use=M1\n+ M1\n"),
            "got:\n{text}"
        );
        assert!(
            !text.contains("-fn big()"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn big()"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_fire_new_file_target_with_leftover_prefix() {
        // `big` moves a.rs -> new b.rs; imports stay ordinary, pastes
        // follow the rewritten leftover hunk (suffix gate).
        let big = big_fn("big");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\nnew file mode 100644\n--- /dev/null\n+++ b/b.rs\n@@ -0,0 +1,45 @@\n+use x;\n+\n+// lead\n{b_plus}");
        let pre = pre_map(&[("a.rs", &a_old)]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("must fire");
        let text = out.join("");
        assert!(
            text.contains("@@ -0,0 @@ rule=r2\n# use=M1\n+ M1\n"),
            "got:\n{text}"
        );
        assert!(
            text.contains("@@ -0,0 +1,3 @@\n+use x;\n+\n+// lead\n"),
            "got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_fire_same_file_reorder() {
        // Same-section delete + insert is a reorder (anchor math is
        // identical; the engine plans against original coordinates).
        let big = big_fn("big");
        let a_old = format!("{big}fn keep() {{}}\n");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,42 +1,0 @@\n{a_minus}@@ -43,0 +2,42 @@\n+fn keep2() {{}}\n{a_plus}");
        let pre = pre_map(&[("a.rs", &a_old)]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a], &pre, &mut used).expect("must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        validated_live(&text, &pre);
    }

    #[test]
    fn move_tiny_item_fires_as_diet() {
        // WI-1e diet (no `# item I`, `# sweep`-short cut): the 3-line
        // move now wins T375 (byte-guard flip pinned).
        let a = "diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,3 +2,0 @@\n-fn m() {\n-    1\n-}\n";
        let b = "diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 +2,3 @@\n+fn m() {\n+    1\n+}\n";
        let pre = pre_map(&[
            ("a.rs", "fn keep() {}\nfn m() {\n    1\n}\n"),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a.to_string(), b.to_string()], &pre, &mut used)
            .expect("diet wins T375");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("# bind="), "got:\n{text}");
    }

    #[test]
    fn move_without_pre_image_stays_ordinary() {
        // Legacy no-pre path cannot verify uniqueness — skip.
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &HashMap::new(), &mut used).is_none());
    }

    #[test]
    fn move_twin_in_pre_image_stays_ordinary() {
        // The cut range would still select exactly one site, but a twin
        // means any span-mapping slip silently cuts the wrong twin —
        // veto (same conservatism as the other detectors).
        let big = big_fn("big");
        let a_old = format!("{big}fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[("a.rs", &a_old), ("b.rs", "use x;\n")]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_non_fn_struct_fires() {
        // T432a: named non-fn items (struct/enum/...) move like fns.
        let body: String = (0..40).map(|i| format!("    f{i}: u32,\n")).collect();
        let strukt = format!("struct Big {{\n{body}}}\n");
        let a_minus: String = strukt.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = strukt.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{strukt}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("struct move must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        validated_live(&text, &pre);
    }

    #[test]
    fn move_mixed_hunk_stays_ordinary() {
        // v1 never does surgical line removal: a hunk mixing the moved
        // item with an edited line stays ordinary (T432).
        let big = big_fn("big");
        let mut a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        a_minus.push_str("-fn edited() {}\n+fn edited2() {}\n");
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,43 +2,1 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{big}fn edited() {{}}\n")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_leading_blank_prefix_trimmed_at_eof_fires() {
        // T432 slice 7 (prefix-trim): a single item with a leading blank
        // and no following blank (EOF) — the cut absorbs the preceding
        // blank, so the prefix leftover hunk trims away entirely.
        let big = big_fn("big");
        let mut a_minus = String::from("-\n");
        a_minus.push_str(&big.lines().map(|l| format!("-{l}\n")).collect::<String>());
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,43 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n\n{big}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used)
            .expect("blank-prefix at EOF must fire via prefix-trim");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            !text.contains("-fn big()"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn big()"),
            "plus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("@@ -2,1"),
            "trimmed prefix hunk must be gone, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_blank_ending_prefix_trimmed_at_eof_fires() {
        // T432 slice 7 (prefix-trim): a longer blank-ending prefix at EOF
        // keeps its non-blank lines ordinary while the cut absorbs the
        // last prefix blank.
        let big = big_fn("big");
        let mut a_minus = String::from("-fn gone() {}\n-\n");
        a_minus.push_str(&big.lines().map(|l| format!("-{l}\n")).collect::<String>());
        let n = big.lines().count() + 2;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\nfn gone() {{}}\n\n{big}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used)
            .expect("blank-ending prefix at EOF must fire via prefix-trim");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -2,1 +2,0 @@\n-fn gone() {}\n"),
            "got:\n{text}"
        );
        assert!(
            !text.contains("-fn big()"),
            "minus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_leading_blank_with_following_blank_fires() {
        // T432c: prefix blank + following blank — the cut absorbs the
        // following one, the prefix hunk deletes the preceding one.
        let big = big_fn("big");
        let mut a_minus = String::from("-\n");
        a_minus.push_str(&big.lines().map(|l| format!("-{l}\n")).collect::<String>());
        a_minus.push_str("-\n");
        let n = big.lines().count() + 2;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n\n{big}\nfn keep2() {{}}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used)
            .expect("leading blank with following blank must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(text.contains("@@ -2,1 +2,0 @@\n-\n"), "got:\n{text}");
        validated_live(&text, &pre);
    }

    #[test]
    fn move_blank_run_minus_stays_ordinary() {
        // Two consecutive blanks: one cut absorbs one following blank —
        // the second would leak — veto.
        let one = big_fn("one");
        let two = big_fn("two");
        let mut a_minus = String::new();
        for l in one.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        a_minus.push_str("-\n-\n");
        for l in two.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = one.lines().count() + two.lines().count() + 2;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let mut b_plus = String::new();
        for l in one.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        b_plus.push_str("+\n");
        for l in two.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        let m = one.lines().count() + two.lines().count() + 1;
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{m} @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{one}\n\n{two}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_plus_double_blank_gap_stays_ordinary() {
        // Same-anchor pastes join with exactly one blank: a double-blank
        // gap in the real file cannot round-trip — veto.
        let one = big_fn("one");
        let two = big_fn("two");
        let mut a_minus = String::new();
        for l in one.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        a_minus.push_str("-\n");
        for l in two.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = one.lines().count() + two.lines().count() + 1;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let mut b_plus = String::new();
        for l in one.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        b_plus.push_str("+\n+\n");
        for l in two.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        let m = one.lines().count() + two.lines().count() + 2;
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{m} @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{one}\n{two}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_plus_adjacent_items_stay_ordinary() {
        // Adjacent moved items (no blank between): pastes would force a
        // join blank the real file lacks — veto.
        let one = big_fn("one");
        let two = big_fn("two");
        let mut a_minus = String::new();
        for l in one.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        for l in two.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = one.lines().count() + two.lines().count();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let mut b_plus = String::new();
        for l in one.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        for l in two.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{n} @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{one}{two}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_plus_trailing_leftovers_fire() {
        // T432b: trailing plus leftovers stay as a rewritten ordinary
        // hunk after the pastes (same-point insertions keep order).
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!(
            "diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,43 @@\n{b_plus}+// trailing\n"
        );
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{big}")),
            ("b.rs", "use x;\nuse y;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("trailing leftovers must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -2,0 +45,1 @@\n+// trailing\n"),
            "got:\n{text}"
        );
        assert!(
            !text.contains("+fn big()"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_plus_prefix_and_trailing_fire() {
        // T432b: prefix stays ordinary before the pastes, trailing
        // after — both preserved verbatim around the moved item.
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!(
            "diff --git b.rs\nnew file mode 100644\n--- /dev/null\n+++ b/b.rs\n@@ -0,0 +1,46 @@\n+use x;\n+\n+// lead\n{b_plus}+// trailing\n"
        );
        let pre = pre_map(&[("a.rs", &format!("fn keep() {{}}\n{big}"))]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("prefix+trailing must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -0,0 +1,3 @@\n+use x;\n+\n+// lead\n"),
            "got:\n{text}"
        );
        assert!(
            text.contains("@@ -0,0 +46,1 @@\n+// trailing\n"),
            "got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_one_to_two_fanout_fires() {
        // T432 fan-out: one minus item, two plus copies — one cut,
        // two pastes sharing the bind (a copy, round-trips exactly).
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let mk_plus = || big.lines().map(|l| format!("+{l}\n")).collect::<String>();
        let b = format!(
            "diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{}",
            mk_plus()
        );
        let c = format!(
            "diff --git c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -2,0 +3,42 @@\n{}",
            mk_plus()
        );
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{big}\nfn keep2() {{}}")),
            ("b.rs", "use x;\n"),
            ("c.rs", "use y;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b, c], &pre, &mut used).expect("fan-out must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert_eq!(text.matches("+ M1\n").count(), 2, "got:\n{text}");
        assert!(
            !text.contains("-fn big()"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn big()"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_single_item_fanout_at_eof_fires() {
        // T438: one minus item fanning out to two pastes at EOF. Only one
        // cut exists, so the no-following-blank path counts distinct items
        // (1, with lo == 0) and stays sound; the old pair count (2) vetoed.
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let mk_plus = || big.lines().map(|l| format!("+{l}\n")).collect::<String>();
        let b = format!(
            "diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{}",
            mk_plus()
        );
        let c = format!(
            "diff --git c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -2,0 +3,42 @@\n{}",
            mk_plus()
        );
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{big}")),
            ("b.rs", "use x;\n"),
            ("c.rs", "use y;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b, c], &pre, &mut used).expect("EOF fan-out must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert_eq!(text.matches("+ M1\n").count(), 2, "got:\n{text}");
        assert!(
            !text.contains("-fn big()"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn big()"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_two_to_one_dedup_fires() {
        // T432 dedup: byte-identical twins in a.rs and b.rs consolidate
        // into one c.rs copy - first claim wins the cut+paste, the
        // other minus hunk stays ordinary (still deleted, byte-exact).
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let c_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let c = format!("diff --git c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -2,0 +3,42 @@\n{c_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep_a() {{}}\n{big}")),
            ("b.rs", &format!("fn keep_b() {{}}\n{big}")),
            ("c.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b, c], &pre, &mut used).expect("dedup must fire");
        let text = out.join("");
        assert_eq!(text.matches("rule=r1\n").count(), 1, "got:\n{text}");
        assert_eq!(text.matches("rule=r2\n").count(), 1, "got:\n{text}");
        assert_eq!(text.matches("+ M1\n").count(), 1, "got:\n{text}");
        assert!(
            text.contains("@@ -2,42 +2,0 @@\n-fn big()"),
            "losing twin stays ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_two_to_two_cross_dedup_fires() {
        // T432 dedup, general rule: two twins consolidate into two
        // copies - one cut (first claim wins both runs), two pastes,
        // the losing minus hunk stays ordinary.
        let big = big_fn("big");
        let mk_minus = || big.lines().map(|l| format!("-{l}\n")).collect::<String>();
        let mk_plus = || big.lines().map(|l| format!("+{l}\n")).collect::<String>();
        let a = format!(
            "diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{}",
            mk_minus()
        );
        let b = format!(
            "diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,42 +2,0 @@\n{}",
            mk_minus()
        );
        let c = format!(
            "diff --git c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -2,0 +3,42 @@\n{}",
            mk_plus()
        );
        let d = format!(
            "diff --git d.rs\n--- a/d.rs\n+++ b/d.rs\n@@ -2,0 +3,42 @@\n{}",
            mk_plus()
        );
        let pre = pre_map(&[
            (
                "a.rs",
                &format!("fn keep_a() {{}}\n{big}\nfn keep2_a() {{}}"),
            ),
            (
                "b.rs",
                &format!("fn keep_b() {{}}\n{big}\nfn keep2_b() {{}}"),
            ),
            ("c.rs", "use x;\n"),
            ("d.rs", "use y;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b, c, d], &pre, &mut used).expect("cross dedup must fire");
        let text = out.join("");
        assert_eq!(text.matches("rule=r1\n").count(), 1, "got:\n{text}");
        assert_eq!(text.matches("+ M1\n").count(), 2, "got:\n{text}");
        assert!(
            text.contains("@@ -2,42 +2,0 @@\n-fn big()"),
            "losing twin stays ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_rename_section_stays_ordinary() {
        // Rename sections never compress (T377): b-side seeding would
        // target the wrong file.
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a/old.rs b/new.rs\nsimilarity index 89%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("new.rs", &format!("fn keep() {{}}\n{big}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_non_rs_section_stays_ordinary() {
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -2,42 +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[("a.txt", "keep\n")]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_backslash_marker_hunk_stays_ordinary() {
        // `\ No newline` markers decline the hunk (counters unmapped).
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{a_minus}\\ No newline at end of file\n");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", format!("fn keep() {{}}\n{big}").trim_end()),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_mixed_fn_and_struct_fire_together() {
        // T432a/c: one minus hunk holding fn + struct pairs both, plus a
        // trailing deleted item (the struct's following blank keeps the
        // last cut sound at non-EOF).
        let big = big_fn("big");
        let zed = "fn zed() {\n    2\n}\n";
        let mut strukt = String::from("struct Big2 {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    g{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let mut minus_body = String::new();
        for l in big.lines() {
            minus_body.push_str(&format!("-{l}\n"));
        }
        minus_body.push_str("-\n");
        for l in strukt.lines() {
            minus_body.push_str(&format!("-{l}\n"));
        }
        minus_body.push_str("-\n");
        for l in zed.lines() {
            minus_body.push_str(&format!("-{l}\n"));
        }
        let n = big.lines().count() + strukt.lines().count() + zed.lines().count() + 2;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{minus_body}");
        let mut plus_body = String::new();
        for l in big.lines() {
            plus_body.push_str(&format!("+{l}\n"));
        }
        plus_body.push_str("+\n");
        for l in strukt.lines() {
            plus_body.push_str(&format!("+{l}\n"));
        }
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{n} @@\n{plus_body}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{big}\n{strukt}\n{zed}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("mixed move must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -88,3 +2,0 @@\n-fn zed() {\n-    2\n-}\n"),
            "got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_multi_pair_at_eof_caps_to_first_fires() {
        // T432 slice 6: the last of several cuts at file EOF would absorb
        // its preceding (middle-gap) blank — already claimed by the
        // previous cut's following-blank absorption — so the last paired
        // minus item drops and the survivors fire; the dropped run stays
        // an ordinary trailing leftover on both sides.
        let one = big_fn("one");
        let two = big_fn("two");
        let mut a_minus = String::new();
        for l in one.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        a_minus.push_str("-\n");
        for l in two.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = one.lines().count() + two.lines().count() + 1;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let mut b_plus = String::new();
        for l in one.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        b_plus.push_str("+\n");
        for l in two.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        let m = one.lines().count() + two.lines().count() + 1;
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{m} @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{one}\n{two}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used)
            .expect("multi-pair at EOF must cap to the first pair");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(!text.contains("rule=r3\n"), "got:\n{text}");
        assert!(!text.contains("rule=r4\n"), "got:\n{text}");
        assert!(
            !text.contains("-fn one()"),
            "paired lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn one()"),
            "paired lines must be consumed, got:\n{text}"
        );
        assert!(text.contains("-fn two() {\n"), "got:\n{text}");
        assert!(text.contains("+fn two() {\n"), "got:\n{text}");
        validated_live(&text, &pre);
    }

    #[test]
    fn move_minus_trailing_leftovers_fire() {
        // T432c: a trailing deleted whole item stays as a rewritten
        // ordinary hunk after the cuts (adjacent: no absorbed blank).
        let big = big_fn("big");
        let tail = "fn tail() {\n    1\n}\n";
        let mut a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        for l in tail.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = big.lines().count() + tail.lines().count();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{big}{tail}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out =
            try_move_sweep(&[a, b], &pre, &mut used).expect("trailing minus leftovers must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -44,3 +2,0 @@\n-fn tail() {\n-    1\n-}\n"),
            "got:\n{text}"
        );
        assert!(
            !text.contains("-fn big()"),
            "paired lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_minus_trailing_blank_sep_fire() {
        // T432c: the single blank between the last paired run and a
        // trailing leftover is absorbed by the cut — the trailing hunk
        // holds the leftover item only (no double-delete).
        let big = big_fn("big");
        let tail = "fn tail() {\n    1\n}\n";
        let mut a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        a_minus.push_str("-\n");
        for l in tail.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = big.lines().count() + tail.lines().count() + 1;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{big}\n{tail}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out =
            try_move_sweep(&[a, b], &pre, &mut used).expect("blank-separated trailing must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -45,3 +2,0 @@\n-fn tail() {\n-    1\n-}\n"),
            "got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_minus_prefix_leftovers_fire() {
        // T432c: a leading deleted item stays ordinary before the cuts.
        let big = big_fn("big");
        let mut a_minus = String::from("-use x;\n");
        a_minus.push_str(&big.lines().map(|l| format!("-{l}\n")).collect::<String>());
        let n = big.lines().count() + 1;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\nuse x;\n{big}")),
            ("b.rs", "use y;\n"),
        ]);
        let mut used = Vec::new();
        let out =
            try_move_sweep(&[a, b], &pre, &mut used).expect("prefix minus leftovers must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("@@ -2,1 +2,0 @@\n-use x;\n"), "got:\n{text}");
        validated_live(&text, &pre);
    }

    #[test]
    fn move_impl_block_stays_ordinary() {
        // T432a: ident-less items (impl blocks) never seed moves.
        let imp = "impl Foo {\n    fn m(&self) {}\n}\n";
        let a_minus: String = imp.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,3 +2,0 @@\n{a_minus}");
        let b_plus: String = imp.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,3 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", "fn keep() {}\nimpl Foo {\n    fn m(&self) {}\n}\n"),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
    }

    #[test]
    fn move_deleted_file_cut_fires() {
        // T432d: the cut side may be a `deleted file mode` section —
        // the rebuilt prelude keeps the deletion, so cuts + leftovers
        // still empty the file. Whole file = one moved item here.
        let big = big_fn("big");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let n = big.lines().count();
        let a = format!("diff --git a.rs\ndeleted file mode 100644\n--- a/a.rs\n+++ /dev/null\n@@ -1,{n} +0,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[("a.rs", &big), ("b.rs", "use x;\n")]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("deleted-file cut must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            !text.contains("-fn big()"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn big()"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_deleted_file_with_leftover_fires() {
        // T432d with a trailing deleted item: it stays as a rewritten
        // ordinary hunk after the cuts (single pair, no prefix, so the
        // last-cut gate passes: nothing preceding to double-claim).
        let big = big_fn("big");
        let tail = "fn tail() {\n    1\n}\n";
        let mut a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        for l in tail.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = big.lines().count() + tail.lines().count();
        let a = format!("diff --git a.rs\ndeleted file mode 100644\n--- a/a.rs\n+++ /dev/null\n@@ -1,{n} +0,0 @@\n{a_minus}");
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,42 @@\n{b_plus}");
        let pre = pre_map(&[("a.rs", &format!("{big}{tail}")), ("b.rs", "use x;\n")]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used)
            .expect("deleted-file cut with leftover must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -43,3 +0,0 @@\n-fn tail() {\n-    1\n-}\n"),
            "got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_multi_paste_with_prefix_to_new_file_fires() {
        // T436 first candidate: two items move into a new file that also
        // carries an ordinary prefix hunk at the same anchor — the
        // multi-paste + leftover shape was parse-only-tested before the
        // live-check rule (validated_live now applies it via the engine).
        let one = big_fn("one");
        let two = big_fn("two");
        let mut a_minus = String::new();
        for l in one.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        a_minus.push_str("-\n");
        for l in two.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = one.lines().count() + two.lines().count() + 1;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let mut b_plus = String::from("+use x;\n+\n+// lead\n");
        for l in one.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        b_plus.push_str("+\n");
        for l in two.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        let m = one.lines().count() + two.lines().count() + 4;
        let b = format!("diff --git b.rs\nnew file mode 100644\n--- /dev/null\n+++ b/b.rs\n@@ -0,0 +1,{m} @@\n{b_plus}");
        let pre = pre_map(&[(
            "a.rs",
            &format!("fn keep() {{}}\n{one}\n{two}\nfn keep2() {{}}"),
        )]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used)
            .expect("multi-paste with prefix to new file must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -0,0 +1,3 @@\n+use x;\n+\n+// lead\n"),
            "got:\n{text}"
        );
        assert!(
            !text.contains("-fn one()"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn one()"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_item_with_interior_blanks_fires() {
        // Interior blanks ride with their item: the cut removes the whole
        // span, so covered lines need no gap accounting (675431e: eight
        // test fns move file-to-file, one carrying interior blanks, which
        // used to veto the whole minus hunk via `blank && covered`).
        let big = big_fn("big");
        let mut spaced = String::from("fn spaced() {\n");
        for i in 0..10 {
            spaced.push_str(&format!("    let v{i} = {i};\n"));
            if i % 3 == 2 {
                spaced.push('\n');
            }
        }
        spaced.push_str("}\n");
        let mut a_minus = String::new();
        for l in big.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        a_minus.push_str("-\n");
        for l in spaced.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        let n = big.lines().count() + spaced.lines().count() + 1;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let mut b_plus = String::new();
        for l in big.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        b_plus.push_str("+\n");
        for l in spaced.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        let m = big.lines().count() + spaced.lines().count() + 1;
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{m} @@\n{b_plus}");
        let pre = pre_map(&[
            (
                "a.rs",
                &format!("fn keep() {{}}\n{big}\n{spaced}\n\nfn keep2() {{}}"),
            ),
            ("b.rs", "use x;\nuse y;\n"),
        ]);
        let mut used = Vec::new();
        let out =
            try_move_sweep(&[a, b], &pre, &mut used).expect("move with interior blanks must fire");
        let text = out.join("");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r1\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(text.contains("rule=r2\n"), "got:\n{text}");
        assert!(
            !text.contains("-fn big()"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+let v0"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_content_bearing_mod_stays_ordinary() {
        // WI-mod-content-cut: the scalar `- I` cut cannot isolate a mod
        // with nested items (it would match them too — `expected one match,
        // found N`). Byte-identical content-bearing mods stay ordinary.
        let mut inner = String::from("    fn inner() {\n");
        for i in 0..40 {
            inner.push_str(&format!("        let v{i} = {i};\n"));
        }
        inner.push_str("    }\n");
        let modu = format!("mod bigmod {{\n{inner}}}\n");
        let a_minus: String = modu.lines().map(|l| format!("-{l}\n")).collect();
        let n = modu.lines().count();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = modu.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{n} @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{modu}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(
            try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "content-bearing mod must stay ordinary"
        );
    }

    #[test]
    fn move_method_bearing_trait_stays_ordinary() {
        // WI-mod-content-cut: same shape for traits — a trait with methods
        // matches 1 trait + N methods under `- I`, so it stays ordinary.
        // (Evolved-trait pairing already vetoes this; WI-8 slice 8.)
        let mut methods = String::new();
        for m in 0..2 {
            methods.push_str(&format!("    fn m{m}(&self) {{\n"));
            for i in 0..20 {
                methods.push_str(&format!("        let v{i} = {i};\n"));
            }
            methods.push_str("    }\n");
        }
        let tra = format!("trait Big {{\n{methods}}}\n");
        let a_minus: String = tra.lines().map(|l| format!("-{l}\n")).collect();
        let n = tra.lines().count();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = tra.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{n} @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{tra}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(
            try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "method-bearing trait must stay ordinary"
        );
    }

    #[test]
    fn move_empty_mod_still_fires() {
        // WI-mod-content-cut: empty inline `mod {}` stays eligible. Doc
        // padding clears the T375 byte-win guard (docs ride with the item).
        let mut docs = String::new();
        for i in 0..40 {
            docs.push_str(&format!("/// doc line {i}\n"));
        }
        let modu = format!("{docs}mod emptymod {{}}\n");
        let a_minus: String = modu.lines().map(|l| format!("-{l}\n")).collect();
        let n = modu.lines().count();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = modu.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{n} @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{modu}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("empty mod must fire");
        validated_live(&out.join(""), &pre);
    }

    #[test]
    fn move_empty_trait_still_fires() {
        // WI-mod-content-cut: empty trait stays eligible (same doc padding).
        let mut docs = String::new();
        for i in 0..40 {
            docs.push_str(&format!("/// doc line {i}\n"));
        }
        let tra = format!("{docs}trait Empty {{}}\n");
        let a_minus: String = tra.lines().map(|l| format!("-{l}\n")).collect();
        let n = tra.lines().count();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let b_plus: String = tra.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{n} @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{tra}")),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = try_move_sweep(&[a, b], &pre, &mut used).expect("empty trait must fire");
        validated_live(&out.join(""), &pre);
    }
}
