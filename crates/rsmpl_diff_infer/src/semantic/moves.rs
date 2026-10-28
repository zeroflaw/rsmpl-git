//! Whole-item 1-to-1 move inference (T430).
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
//! ordinary by design (T432 remainder): named items, pure-minus /
//! pure-plus `-U0` hunks only (never mixed-hunk surgery), global 1-minus/1-plus
//! pairing (N-to-1 dedup like 7945da1 and 1-to-2 fan-out skip),
//! cut-file pre-image required with flat-ws uniqueness (twin veto; legacy
//! no-pre path skips), minus paired runs with exactly one blank between
//! following blank; the last run needs a following blank in the file —
//! runs (a cut absorbs exactly one following blank; the last run needs a
//! following blank in the file — without one a multi-pair last cut or a
//! blank-ending prefix double-claims the preceding blank, f09edcd);
//! uncovered prefix/trailing minus lines stay ordinary around the cuts
//! (T432c; one post-run blank is absorbed by the cut, never re-emitted),
//! plus paired
//! runs with exactly one blank between runs (the same-anchor paste join
//! restores exactly one); uncovered prefix/trailing plus lines stay
//! ordinary around pastes (T432b), deleted-file cuts allowed (T432d;
//! the rebuilt prelude keeps the deletion), rename sections skipped,
//! and a global T375 byte-win guard.

use std::collections::HashMap;
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
    // Global 1-to-1 pairing by exact item text.
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
            if hits.len() != 1 {
                continue;
            }
            let (psi, phi, first, last) = hits[0];
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
            });
        }
    }
    if pairs.is_empty() {
        return None;
    }
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
        if !minus_last_cut_sound(&hunk.body, &sound, *msi, *mhi, hunk.old_start, file_old) {
            continue;
        }
        emit.extend(
            sound
                .iter()
                .filter(|p| p.minus_hunk_key == (*msi, *mhi))
                .cloned(),
        );
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
    // Name every pair, then rebuild each touched section once.
    let mut named: Vec<(MovePair, String, String)> = Vec::new();
    let mut bind_seq: usize = 0;
    for p in &final_pairs {
        let cut_rule = super::unique_rule(used_rules, &format!("move_{}", p.item.ident));
        let paste_rule = super::unique_rule(used_rules, &format!("paste_{}", p.item.ident));
        bind_seq += 1;
        let bind = format!("M{bind_seq}");
        let hunk = &parsed[p.minus_hunk_key.0].hunks[p.minus_hunk_key.1];
        let start = hunk.old_start + p.item.first as u32;
        let len = (p.item.last - p.item.first + 1) as u32;
        let cut = format!("@@ -{start},{len} @@ rule={cut_rule}\n# item I\n# bind={bind}\n- I\n");
        let anchor = parsed[p.plus_hunk_key.0].hunks[p.plus_hunk_key.1].old_start;
        let paste = format!("@@ -{anchor},0 @@ rule={paste_rule}\n# use={bind}\n+ {bind}\n");
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
            let cuts: Vec<&String> = named
                .iter()
                .filter(|(p, _, _)| p.minus_hunk_key == (*si, hi))
                .map(|(_, c, _)| c)
                .collect();
            if !cuts.is_empty() {
                // Prefix leftovers stay ordinary before the cuts (T432c;
                // disjoint old-range deletions, SPEC 10.6 option-1).
                // Cuts never absorb preceding blanks, so no skip here.
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
                if mlo > 0 {
                    let header = format!(
                        "@@ -{},{} +{},{} @@",
                        hunk.old_start, mlo, hunk.new_start, 0
                    );
                    emitted += header.len() + 1;
                    rebuilt.push_str(&header);
                    rebuilt.push('\n');
                    for line in hunk.body.iter().take(mlo) {
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

#[derive(Clone)]
struct MoveItem {
    ident: String,
    text: String,
    lines: Vec<String>,
    first: usize,
    last: usize,
}

#[derive(Clone)]
struct MovePair {
    minus_hunk_key: (usize, usize),
    plus_hunk_key: (usize, usize),
    item: MoveItem,
    plus_first: usize,
    plus_last: usize,
}

struct HunkParts {
    header: String,
    old_start: u32,
    old_len: u32,
    new_start: u32,
    new_len: u32,
    body: Vec<String>,
}

struct SectionParts {
    orig_idx: usize,
    path: String,
    is_deleted: bool,
    prelude: String,
    hunks: Vec<HunkParts>,
}

#[derive(PartialEq)]
enum Side {
    Minus,
    Plus,
}

fn line_text(line: &str) -> &str {
    line.get(1..).unwrap_or("")
}

/// Minus-hunk gate over a narrowed pair set: paired runs sit anywhere
/// with exactly one blank between consecutive runs (a cut absorbs exactly
/// one following blank); uncovered prefix lines stay ordinary before the
/// cuts, uncovered trailing lines after (T432c; the single blank right
/// after the last paired run is absorbed by the cut, never re-emitted).
/// `minus_items_in` already validated the whole body as item spans, so
/// leftovers are whole items (or blanks) by construction.
fn minus_hunk_holds(body: &[String], pairs: &[MovePair], msi: usize, mhi: usize) -> bool {
    if body.is_empty() {
        return false;
    }
    let mut covered = vec![false; body.len()];
    for p in pairs.iter().filter(|p| p.minus_hunk_key == (msi, mhi)) {
        covered[p.item.first..=p.item.last].fill(true);
    }
    let Some(lo) = covered.iter().position(|&c| c) else {
        return false;
    };
    let Some(hi) = covered.iter().rposition(|&c| c) else {
        return false;
    };
    let mut prev_blank = false;
    for (i, line) in body.iter().enumerate().take(hi + 1).skip(lo) {
        let blank = line_text(line).trim().is_empty();
        if blank && covered[i] {
            return false;
        }
        if !covered[i] && (!blank || prev_blank) {
            return false;
        }
        prev_blank = !covered[i];
    }
    true
}

/// Last-cut absorption soundness (f09edcd): a cut absorbs its following
/// blank line, otherwise (non-blank or EOF) its preceding blank line.
/// The preceding blank of the last paired run is always claimed already
/// — the middle-gap blank by the previous cut (multi-pair), or the last
/// line of a blank-ending prefix leftover hunk — so without a following
/// blank in the file the last cut double-claims it (plan-stage overlap).
/// Single pairs with a non-blank preceding line (no prefix, or a
/// non-blank-ending prefix) absorb nothing and stay sound.
fn minus_last_cut_sound(
    body: &[String],
    pairs: &[MovePair],
    msi: usize,
    mhi: usize,
    old_start: u32,
    file_old: &str,
) -> bool {
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    let mut n = 0usize;
    for p in pairs.iter().filter(|p| p.minus_hunk_key == (msi, mhi)) {
        lo = lo.min(p.item.first);
        hi = hi.max(p.item.last);
        n += 1;
    }
    if n == 0 {
        return false;
    }
    // The file line right after the last paired run: in-hunk first
    // (covers trailing leftovers), else the pre-image's next line (a cut
    // absorbs beyond hunk bounds; EOF has no following line).
    let follows_blank = if hi + 1 < body.len() {
        line_text(&body[hi + 1]).trim().is_empty()
    } else {
        file_old
            .lines()
            .nth(old_start as usize + hi)
            .is_some_and(|l| l.trim().is_empty())
    };
    if follows_blank {
        return true;
    }
    if n != 1 {
        return false;
    }
    if lo == 0 {
        // No prefix: the preceding line lives outside the hunk,
        // unclaimed by any leftover hunk — absorbing it is harmless.
        return true;
    }
    !line_text(&body[lo - 1]).trim().is_empty()
}

/// Paired runs in a plus-hunk body, in any position.
/// Paired runs sit anywhere in the plus hunk with exactly one blank
/// line between consecutive runs (the same-anchor paste join restores
/// exactly one blank between pastes, so only single-blank gaps
/// round-trip; adjacent or multi-blank gaps veto). Uncovered prefix
/// lines stay ordinary before the pastes, trailing lines after.
fn plus_runs_hold(body: &[String], runs: &[(usize, usize)]) -> bool {
    if runs.is_empty() || body.is_empty() {
        return false;
    }
    let mut sorted = runs.to_vec();
    sorted.sort();
    for w in sorted.windows(2) {
        let (_, l1) = w[0];
        let (f2, _) = w[1];
        if f2 != l1 + 2 || !line_text(&body[l1 + 1]).trim().is_empty() {
            return false;
        }
    }
    true
}

/// Split a collapsed section into prelude + hunks; `None` when the
/// section can never hold a move (rename, non-`.rs`, CRLF, bad range).
fn split_section(section: &str, orig_idx: usize) -> Option<SectionParts> {
    let header = section.lines().next()?;
    if !header.starts_with("diff --git ") || section.contains('\r') {
        return None;
    }
    if is_rename_section(header) {
        return None;
    }
    let path = super::super::b_side_path(header.strip_prefix("diff --git ")?)?;
    if !path.ends_with(".rs") {
        return None;
    }
    let is_deleted = section.lines().any(|l| {
        l.starts_with("deleted file mode") || (l.starts_with("+++") && l.contains("/dev/null"))
    });
    let mut prelude = String::new();
    let mut hunks: Vec<HunkParts> = Vec::new();
    let mut cur_header: Option<String> = None;
    let mut cur_body: Vec<String> = Vec::new();
    for line in section.lines() {
        if line.starts_with("@@") {
            if let Some(h) = cur_header.take() {
                let (a, b, c, d) = parse_ranges(&h)?;
                hunks.push(HunkParts {
                    header: h,
                    old_start: a,
                    old_len: b,
                    new_start: c,
                    new_len: d,
                    body: std::mem::take(&mut cur_body),
                });
            }
            cur_header = Some(line.to_string());
            continue;
        }
        match cur_header {
            Some(_) => cur_body.push(line.to_string()),
            None => {
                prelude.push_str(line);
                prelude.push('\n');
            }
        }
    }
    if let Some(h) = cur_header {
        let (a, b, c, d) = parse_ranges(&h)?;
        hunks.push(HunkParts {
            header: h,
            old_start: a,
            old_len: b,
            new_start: c,
            new_len: d,
            body: cur_body,
        });
    }
    if hunks.is_empty() {
        return None;
    }
    Some(SectionParts {
        orig_idx,
        path,
        is_deleted,
        prelude,
        hunks,
    })
}

/// `@@ -a[,b] +c[,d] @@` (length defaults to 1); `None` on anything
/// else (semantic headers never appear in native input, but decline
/// defensively instead of mis-citing).
fn parse_ranges(header: &str) -> Option<(u32, u32, u32, u32)> {
    let inner = header.strip_prefix("@@")?.split("@@").next()?.trim();
    let mut parts = inner.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    if parts.next().is_some() {
        return None;
    }
    let (a, b) = parse_range(old)?;
    let (c, d) = parse_range(new)?;
    Some((a, b, c, d))
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    let mut it = s.split(',');
    let start: u32 = it.next()?.parse().ok()?;
    let len: u32 = match it.next() {
        Some(l) => l.parse().ok()?,
        None => 1,
    };
    if it.next().is_some() {
        return None;
    }
    Some((start, len))
}

fn is_rename_section(header: &str) -> bool {
    // Same test as the per-section path (T377): detectors key
    // emissions to the b-side path, which would seed from the wrong
    // file when sides differ.
    let Some(rest) = header.strip_prefix("diff --git ") else {
        return false;
    };
    let Some(parts) = rsmpl_patch::split_git_paths(rest) else {
        return false;
    };
    if parts.len() != 2 {
        return false;
    }
    let old = parts[0].strip_prefix("a/").unwrap_or(&parts[0]);
    let new = parts[1].strip_prefix("b/").unwrap_or(&parts[1]);
    if old == "/dev/null" || old == "dev/null" || new == "/dev/null" || new == "dev/null" {
        return false;
    }
    old != new
}

/// Pure-minus / pure-plus classification (`-U0` input carries no
/// context lines; anything else — context, empty, `\` markers —
/// declines the hunk).
fn classify(hunk: &HunkParts) -> Option<Side> {
    if hunk.body.is_empty() {
        return None;
    }
    let mut minus = 0;
    let mut plus = 0;
    for line in &hunk.body {
        if line.starts_with('-') && !line.starts_with("---") {
            minus += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            plus += 1;
        } else {
            return None;
        }
    }
    if minus > 0 && plus == 0 && hunk.new_len == 0 {
        Some(Side::Minus)
    } else if plus > 0 && minus == 0 && hunk.old_len == 0 {
        Some(Side::Plus)
    } else {
        None
    }
}

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

    fn validated(patch: &str) {
        rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
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
            text.contains("@@ -2,42 @@ rule=move_big\n# item I\n# bind=M1\n- I\n"),
            "got:\n{text}"
        );
        assert!(
            text.contains("@@ -2,0 @@ rule=paste_big\n# use=M1\n+ M1\n"),
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
        validated(&text);
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
            text.contains("@@ -0,0 @@ rule=paste_big\n# use=M1\n+ M1\n"),
            "got:\n{text}"
        );
        assert!(
            text.contains("@@ -0,0 +1,3 @@\n+use x;\n+\n+// lead\n"),
            "got:\n{text}"
        );
        validated(&text);
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("rule=paste_big\n"), "got:\n{text}");
        validated(&text);
    }

    #[test]
    fn move_tiny_item_stays_ordinary() {
        // T375: a 3-line item costs more as bind/use than as two
        // ordinary hunks — the byte-win guard vetoes.
        let a = "diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,3 +2,0 @@\n-fn m() {\n-    1\n-}\n";
        let b = "diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 +2,3 @@\n+fn m() {\n+    1\n+}\n";
        let pre = pre_map(&[
            ("a.rs", "fn keep() {}\nfn m() {\n    1\n}\n"),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_move_sweep(&[a.to_string(), b.to_string()], &pre, &mut used).is_none());
        assert!(used.is_empty(), "vetoed sweep must release rule names");
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
        assert!(text.contains("rule=move_Big\n"), "got:\n{text}");
        assert!(text.contains("rule=paste_Big\n"), "got:\n{text}");
        validated(&text);
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
    fn move_leading_blank_minus_stays_ordinary() {
        // A leading blank with no following blank (EOF): the single cut
        // would absorb the preceding prefix blank the leftover hunk also
        // deletes — veto (T432c last-cut gate, f09edcd).
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
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("rule=paste_big\n"), "got:\n{text}");
        assert!(text.contains("@@ -2,1 +2,0 @@\n-\n"), "got:\n{text}");
        validated(&text);
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("rule=paste_big\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -2,0 +45,1 @@\n+// trailing\n"),
            "got:\n{text}"
        );
        assert!(
            !text.contains("+fn big()"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated(&text);
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("rule=paste_big\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -0,0 +1,3 @@\n+use x;\n+\n+// lead\n"),
            "got:\n{text}"
        );
        assert!(
            text.contains("@@ -0,0 +46,1 @@\n+// trailing\n"),
            "got:\n{text}"
        );
        validated(&text);
    }

    #[test]
    fn move_one_to_two_fanout_stays_ordinary() {
        // Same text added in two files: no unique target — veto.
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
        assert!(try_move_sweep(&[a, b, c], &pre, &mut used).is_none());
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("rule=move_Big2\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -88,3 +2,0 @@\n-fn zed() {\n-    2\n-}\n"),
            "got:\n{text}"
        );
        validated(&text);
    }

    #[test]
    fn move_multi_pair_at_eof_stays_ordinary() {
        // f09edcd: the last of several cuts at file EOF would absorb its
        // preceding (middle-gap) blank — already claimed by the previous
        // cut's following-blank absorption — so multi-pair hunks whose
        // last run has no following blank stay ordinary.
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
        assert!(try_move_sweep(&[a, b], &pre, &mut used).is_none());
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("rule=paste_big\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -44,3 +2,0 @@\n-fn tail() {\n-    1\n-}\n"),
            "got:\n{text}"
        );
        assert!(
            !text.contains("-fn big()"),
            "paired lines must be consumed, got:\n{text}"
        );
        validated(&text);
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -45,3 +2,0 @@\n-fn tail() {\n-    1\n-}\n"),
            "got:\n{text}"
        );
        validated(&text);
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("@@ -2,1 +2,0 @@\n-use x;\n"), "got:\n{text}");
        validated(&text);
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("rule=paste_big\n"), "got:\n{text}");
        assert!(
            !text.contains("-fn big()"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn big()"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated(&text);
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
        assert!(text.contains("rule=move_big\n"), "got:\n{text}");
        assert!(text.contains("rule=paste_big\n"), "got:\n{text}");
        assert!(
            text.contains("@@ -43,3 +0,0 @@\n-fn tail() {\n-    1\n-}\n"),
            "got:\n{text}"
        );
        validated(&text);
    }
}
