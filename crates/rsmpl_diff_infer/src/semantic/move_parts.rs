//! Move section anatomy + gate predicates (WI-8b-pre split).
//!
//! Pure-move extraction from moves.rs (D5 gate relief: moves.rs sat at
//! 1986/2000). Owns the collapsed-section anatomy (SectionParts,
//! HunkParts, Side, line_text, range parsing, classify) plus the
//! hunk-level gate predicates (minus_hunk_holds, minus_last_cut_sound,
//! plus_runs_hold). Pairing-owned types (MoveItem, MovePair) live
//! here too so the gates stay signature-stable; the pairing, rebuild, and
//! item-span helpers stay in moves.rs. Zero behavior change: byte-moved
//! code only, visibility widened to pub(crate) for the single importer.

#[derive(Clone)]
pub(crate) struct MoveItem {
    pub(crate) ident: String,
    pub(crate) text: String,
    pub(crate) lines: Vec<String>,
    pub(crate) first: usize,
    pub(crate) last: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvolvedKind {
    Fn,
    Struct,
    Enum,
    Union,
    Impl,
    Type,
    Const,
    Static,
    Mod,
    Trait,
}

#[derive(Clone)]
pub(crate) struct EvolvedPair {
    pub(crate) kind: EvolvedKind,
    pub(crate) prefix: String,
    pub(crate) name: String,
    /// WI-8 slice 6: the plus-side `mut` for `Static` pairs (the
    /// template carries it verbatim); always `false` for the other eight
    /// kinds.
    pub(crate) static_mut: bool,
    /// WI-8 slice 9: a verbatim concrete paste line that overrides the
    /// kind-built template (signature-evolving `fn` moves paste the
    /// plus-side header concretely); `None` for the eight abstract
    /// shapes, whose paste is built from `kind` + `prefix` + `name`.
    pub(crate) template: Option<String>,
}

#[derive(Clone)]
pub(crate) struct MovePair {
    pub(crate) minus_hunk_key: (usize, usize),
    pub(crate) plus_hunk_key: (usize, usize),
    pub(crate) item: MoveItem,
    pub(crate) plus_first: usize,
    pub(crate) plus_last: usize,
    /// WI-8a + WI-8 slices 1-8: plus-side header evolution (kind +
    /// prefix + new name); `None` for byte-identical pairs
    /// (bare-`bind` paste).
    pub(crate) evolved: Option<EvolvedPair>,
}

pub(crate) struct HunkParts {
    pub(crate) header: String,
    pub(crate) old_start: u32,
    pub(crate) old_len: u32,
    pub(crate) new_start: u32,
    pub(crate) new_len: u32,
    pub(crate) body: Vec<String>,
    /// WI-6 slice 3a: native-hunk group id for runs of an ordered
    /// run-split mixed native hunk (`None` for native pure hunks).
    pub(crate) group: Option<usize>,
    /// WI-6 slice 3a: the original header+body of a run-split mixed
    /// native hunk, re-emitted verbatim when no run of the group is
    /// touched by the move (`None` for native pure hunks).
    pub(crate) unsplit: Option<(String, Vec<String>)>,
}

pub(crate) struct SectionParts {
    pub(crate) orig_idx: usize,
    pub(crate) path: String,
    pub(crate) is_deleted: bool,
    pub(crate) prelude: String,
    pub(crate) hunks: Vec<HunkParts>,
}

#[derive(PartialEq)]
pub(crate) enum Side {
    Minus,
    Plus,
}

pub(crate) fn line_text(line: &str) -> &str {
    line.get(1..).unwrap_or("")
}

/// E1 slice 3: single-line removed-attr leftover predicate for the
/// hunk-interior gate below: exactly one `#[...]` (single line, one
/// occurrence) or one outer-doc `///` line. Multi-line attrs, `//!`,
/// and code all decline (stay ordinary via the gate veto).
fn is_single_attr_doc(t: &str) -> bool {
    let s = t.trim();
    if s.starts_with("///") {
        return true;
    }
    s.starts_with("#[") && s.ends_with(']') && s.matches("#[").count() == 1
}

/// Minus-hunk gate over a narrowed pair set: paired runs sit anywhere
/// with exactly one blank between consecutive runs (a cut absorbs exactly
/// one following blank); uncovered prefix lines stay ordinary before the
/// cuts, uncovered trailing lines after (T432c; the single blank right
/// after the last paired run is absorbed by the cut, never re-emitted).
/// Interior blanks ride with their item (the cut removes the whole span,
/// so covered lines need no gap accounting); uncovered middle lines keep
/// the strict model — non-blanks veto (an unpaired item would be dropped)
/// and blank runs veto (the paste join restores exactly one).
/// `minus_items_in` already validated the whole body as item spans, so
/// leftovers are whole items (or blanks) by construction.
pub(crate) fn minus_hunk_holds(
    body: &[String],
    pairs: &[MovePair],
    msi: usize,
    mhi: usize,
) -> bool {
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
    let mut gap_attrs: u32 = 0;
    for (i, line) in body.iter().enumerate().take(hi + 1).skip(lo) {
        if covered[i] {
            prev_blank = false;
            gap_attrs = 0;
            continue;
        }
        let blank = line_text(line).trim().is_empty();
        if blank {
            if prev_blank {
                return false;
            }
            prev_blank = true;
            continue;
        }
        prev_blank = false;
        if gap_attrs >= 1 {
            return false;
        }
        if !is_single_attr_doc(line_text(line)) {
            return false;
        }
        gap_attrs += 1;
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
pub(crate) fn minus_last_cut_sound(
    body: &[String],
    pairs: &[MovePair],
    msi: usize,
    mhi: usize,
    old_start: u32,
    file_old: &str,
) -> bool {
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for p in pairs.iter().filter(|p| p.minus_hunk_key == (msi, mhi)) {
        lo = lo.min(p.item.first);
        hi = hi.max(p.item.last);
        let span = (p.item.first, p.item.last);
        if !spans.contains(&span) {
            spans.push(span);
        }
    }
    // Fan-out shares one cut across N pastes: soundness counts distinct
    // minus items (T438), not pairs.
    let n = spans.len();
    if n == 0 {
        return false;
    }
    // The file line right after the last paired run: in-hunk first
    // (covers trailing leftovers), else the pre-image's next line (a cut
    // absorbs beyond hunk bounds; EOF has no following line).
    // WI-6: a following blank OUTSIDE the hunk is a kept line the last
    // cut would silently eat (git alignment keeps it; the rebuild has
    // no leftover accounting for it) — veto so capping degrades to the
    // sound prefix instead of diverging.
    let following_in_hunk = hi + 1 < body.len();
    let follows_blank = if following_in_hunk {
        line_text(&body[hi + 1]).trim().is_empty()
    } else {
        file_old
            .lines()
            .nth(old_start as usize + hi)
            .is_some_and(|l| l.trim().is_empty())
    };
    if follows_blank {
        return following_in_hunk;
    }
    // WI-18: a source file emptied to zero bytes is a first-class move
    // source. The minus hunk covers the whole file (BOF..EOF, cut range
    // from the first body line), so no outside preceding/following line
    // exists to double-claim: BOF absorbs nothing, and the EOF
    // preceding-blank absorption lands inside the hunk where
    // `coalesce_deletes` union-merges the overlapping pure deletes.
    // Adjacent items absorb nothing at all. Without this gate the
    // capping loop degrades whole-file collections to a single item
    // that then loses T375 (fixture 44).
    if old_start == 1 && lo == 0 {
        let nlines = file_old.lines().count();
        let last_file_line = old_start as usize + hi;
        if last_file_line >= nlines {
            return true;
        }
    }
    // E1 slice 3: multi-cut hunks whose last cut is preceded by a
    // non-blank hunk line absorb nothing (following is non-blank or
    // EOF per the checks above; preceding is the interior leftover
    // the rebuild emits ordinary) — sound, so the interior-removed
    // companion sets
    // keep their full pair set instead of capping to a prefix subset.
    let last_first = pairs
        .iter()
        .filter(|p| p.minus_hunk_key == (msi, mhi))
        .map(|p| p.item.first)
        .max();
    if let Some(lf) = last_first {
        // Narrow: only an UNCOVERED single-attr predecessor unlocks.
        // A covered predecessor (adjacent multi-cuts) keeps the
        // existing capping behavior pinned by
        // `partial_adjacent_multi_cut_stays_unsound`.
        if lf > 0 && lf <= hi {
            let prev_covered = pairs
                .iter()
                .filter(|p| p.minus_hunk_key == (msi, mhi))
                .any(|p| (p.item.first..=p.item.last).contains(&(lf - 1)));
            if !prev_covered && is_single_attr_doc(line_text(&body[lf - 1])) {
                return true;
            }
        }
    }
    if n != 1 {
        return false;
    }
    if lo == 0 {
        // No prefix in-hunk. Away from EOF the cut absorbs nothing, so
        // the outside preceding line is untouched. At EOF the cut
        // absorbs a preceding blank — sound only when that file line
        // is absent (BOF) or non-blank; a kept blank outside the hunk
        // would be silently eaten (same WI-6 root cause as above).
        let last_file_line = old_start as usize + hi;
        let at_eof = last_file_line >= file_old.lines().count();
        if !at_eof {
            return true;
        }
        if old_start as usize + lo < 2 {
            return true;
        }
        return !file_old
            .lines()
            .nth(old_start as usize + lo - 2)
            .is_some_and(|l| l.trim().is_empty());
    }
    if !line_text(&body[lo - 1]).trim().is_empty() {
        return true;
    }
    // Blank-ending prefix with no following blank (prefix-trim): the cut
    // absorbs the preceding blank, and the rebuild trims that one line
    // from the prefix leftover hunk — nothing double-claims it.
    true
}

/// Paired runs in a plus-hunk body, in any position./// Paired runs sit anywhere in the plus hunk with exactly one blank
/// line between consecutive runs (the same-anchor paste join restores
/// exactly one blank between pastes, so only single-blank gaps
/// round-trip; adjacent or multi-blank gaps veto). Uncovered prefix
/// lines stay ordinary before the pastes, trailing lines after.
pub(crate) fn plus_runs_hold(body: &[String], runs: &[(usize, usize)]) -> bool {
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

/// WI-6 slice 3a: same-group pairing veto — never pair two runs of one
/// run-split native hunk. The split separates deletion runs from
/// insertion runs for cross-hunk pairing; pairing same-group runs
/// relabels a same-spot edit (e.g. a same-file rename) as a move and
/// starves the intent detectors that own it (assoc/member/item
/// follow). A genuine adjacent same-file swap stays ordinary
/// (conservative, sound).
pub(crate) fn is_same_group_pair(parsed: &[SectionParts], p: &MovePair) -> bool {
    let (msi, mhi) = p.minus_hunk_key;
    let (psi, phi) = p.plus_hunk_key;
    msi == psi
        && parsed[msi].hunks[mhi].group.is_some()
        && parsed[msi].hunks[mhi].group == parsed[psi].hunks[phi].group
}

/// WI-6 slice 3a: anchor-in-cut veto — true when an untouched plus run
/// anchors inside a same-section cut's claimed range (closed: the
/// engine overlaps an insertion at the cut's first line, while an
/// anchor past the last line is the proven same-file order). Run-split
/// anchors are exact by construction (ordered walk), so only genuinely
/// interior insertions veto — a trailing-edit run anchors past the cut
/// and fires. Untouched minus runs need no check: leftover deletions
/// are disjoint from cuts by construction, and paste/leftover
/// coexistence is probe-proven.
pub(crate) fn anchor_in_cut_veto(parsed: &[SectionParts], pairs: &[MovePair]) -> bool {
    let mut cut_ranges: Vec<(usize, u32, u32)> = Vec::new();
    for q in pairs {
        let (msi, mhi) = q.minus_hunk_key;
        let h = &parsed[msi].hunks[mhi];
        cut_ranges.push((
            msi,
            h.old_start + q.item.first as u32,
            h.old_start + q.item.last as u32,
        ));
    }
    parsed.iter().enumerate().any(|(si, sec)| {
        sec.hunks.iter().enumerate().any(|(hi, h)| {
            h.unsplit.is_some()
                && h.old_len == 0
                && !pairs.iter().any(|p| p.plus_hunk_key == (si, hi))
                && cut_ranges
                    .iter()
                    .any(|(csi, cs, ce)| *csi == si && *cs <= h.old_start && h.old_start <= *ce)
        })
    })
}

/// WI-6 slice 3a: push one native hunk, splitting mixed (`-`+`+`)
/// `-U0` hunks into ordered pure runs for pairing. A moved item
/// deleted adjacent to an edited line merges into one mixed hunk (no
/// unchanged line separates them); without the split the minority side
/// is invisible to pairing and the move vetoes. Runs keep file order
/// with exact anchors (a plus run cites the insertion point after the
/// last consumed old line, `@@ -0,0 @@` for a leading run), so the
/// anchor-in-cut veto only fires on genuinely interior insertions.
/// Every run carries the group id plus the original header+body so the
/// rebuild re-emits untouched groups verbatim. Anything beyond `-`/`+`
/// lines (context, empty, `\` markers) declines the split — the whole
/// hunk stays single and invisible, as before.
fn push_hunk(
    hunks: &mut Vec<HunkParts>,
    header: String,
    old_start: u32,
    old_len: u32,
    new_start: u32,
    new_len: u32,
    body: Vec<String>,
) {
    let mut minus = 0;
    let mut plus = 0;
    let mut other = false;
    for line in &body {
        if line.starts_with('-') && !line.starts_with("---") {
            minus += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            plus += 1;
        } else {
            other = true;
            break;
        }
    }
    if other || minus == 0 || plus == 0 {
        hunks.push(HunkParts {
            header,
            old_start,
            old_len,
            new_start,
            new_len,
            body,
            group: None,
            unsplit: None,
        });
        return;
    }
    // Ordered maximal pure runs with walked anchors: a minus run of k
    // cites the next k old lines; a plus run of k cites the insertion
    // point after the last consumed old line (`old_start - 1`, possibly
    // `0`, when the run leads the hunk). Virtual ranges cite actual
    // line counts (a `-U0` header agrees by construction).
    let gid = hunks.len();
    let mut minus_done: u32 = 0;
    let mut plus_done: u32 = 0;
    let mut idx = 0;
    while idx < body.len() {
        let is_minus = body[idx].starts_with('-');
        let mut j = idx;
        while j < body.len() && body[j].starts_with('-') == is_minus {
            j += 1;
        }
        let run: Vec<String> = body[idx..j].to_vec();
        let k = run.len() as u32;
        if is_minus {
            hunks.push(HunkParts {
                header: format!(
                    "@@ -{},{} +{},0 @@",
                    old_start + minus_done,
                    k,
                    new_start + plus_done
                ),
                old_start: old_start + minus_done,
                old_len: k,
                new_start: new_start + plus_done,
                new_len: 0,
                body: run,
                group: Some(gid),
                unsplit: Some((header.clone(), body.clone())),
            });
            minus_done += k;
        } else {
            let anchor = old_start.saturating_add(minus_done).saturating_sub(1);
            hunks.push(HunkParts {
                header: format!("@@ -{anchor},0 +{},{} @@", new_start + plus_done, k),
                old_start: anchor,
                old_len: 0,
                new_start: new_start + plus_done,
                new_len: k,
                body: run,
                group: Some(gid),
                unsplit: Some((header.clone(), body.clone())),
            });
            plus_done += k;
        }
        idx = j;
    }
}

/// Split a collapsed section into prelude + hunks; `None` when the
/// section can never hold a move (rename, non-`.rs`, CRLF, bad range).
pub(crate) fn split_section(section: &str, orig_idx: usize) -> Option<SectionParts> {
    let header = section.lines().next()?;
    if !header.starts_with("diff --git ") || section.contains('\r') {
        return None;
    }
    if is_rename_section(header) {
        return None;
    }
    // WI-4d: pre-pass emissions never re-enter later pre-passes.
    if super::is_semantic_section(section) {
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
                push_hunk(&mut hunks, h, a, b, c, d, std::mem::take(&mut cur_body));
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
        push_hunk(&mut hunks, h, a, b, c, d, cur_body);
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
pub(crate) fn parse_ranges(header: &str) -> Option<(u32, u32, u32, u32)> {
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

pub(crate) fn parse_range(s: &str) -> Option<(u32, u32)> {
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

pub(crate) fn is_rename_section(header: &str) -> bool {
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
pub(crate) fn classify(hunk: &HunkParts) -> Option<Side> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pair(first: usize, last: usize) -> MovePair {
        MovePair {
            minus_hunk_key: (0, 0),
            plus_hunk_key: (1, 0),
            item: MoveItem {
                ident: format!("i{first}"),
                text: String::new(),
                lines: Vec::new(),
                first,
                last,
            },
            plus_first: 0,
            plus_last: 0,
            evolved: None,
        }
    }

    #[test]
    fn whole_file_adjacent_multi_cut_is_sound() {
        // WI-18 (fixture 44): whole-file adjacent items with no blanks
        // absorb nothing — the multi-cut is sound despite no following
        // blank.
        let body: Vec<String> = (0..9).map(|i| format!("-line{i}")).collect();
        let pairs = vec![test_pair(0, 2), test_pair(3, 5), test_pair(6, 8)];
        let old = "l0\nl1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
        assert!(minus_last_cut_sound(&body, &pairs, 0, 0, 1, old));
    }

    #[test]
    fn partial_adjacent_multi_cut_stays_unsound() {
        // Same three adjacent items mid-file with kept code after the
        // range: without a following blank the last cut's absorption is
        // unproven — the veto stands.
        let body: Vec<String> = (0..9).map(|i| format!("-line{i}")).collect();
        let pairs = vec![test_pair(0, 2), test_pair(3, 5), test_pair(6, 8)];
        let old = "head\nl0\nl1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\ntail\n";
        // Minus hunk starts at file line 2, covers to line 10, tail kept.
        assert!(!minus_last_cut_sound(&body, &pairs, 0, 0, 2, old));
    }

    #[test]
    fn ordered_runs_split_with_precise_anchors() {
        // WI-6 slice 3a: a mixed `-U0` hunk (insertion adjacent to an
        // edit) parses as ordered pure runs with walked anchors,
        // sharing one group id and keeping the original for verbatim
        // re-emission when untouched.
        let section = "diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,2 @@\n-fn old() {}\n+fn bar() {}\n+fn new() {}\n";
        let parts = split_section(section, 0).expect("must parse");
        assert_eq!(parts.hunks.len(), 2);
        let (m, p) = (&parts.hunks[0], &parts.hunks[1]);
        assert_eq!(m.group, Some(0));
        assert_eq!(p.group, Some(0));
        assert_eq!(m.header, "@@ -1,1 +1,0 @@");
        assert_eq!(p.header, "@@ -1,0 +1,2 @@");
        assert_eq!(
            (m.old_start, m.old_len, m.new_start, m.new_len),
            (1, 1, 1, 0)
        );
        assert_eq!(
            (p.old_start, p.old_len, p.new_start, p.new_len),
            (1, 0, 1, 2)
        );
        assert_eq!(m.body, vec!["-fn old() {}".to_string()]);
        assert_eq!(
            p.body,
            vec!["+fn bar() {}".to_string(), "+fn new() {}".to_string()]
        );
        assert!(m.unsplit.is_some() && p.unsplit.is_some());
        let (h, b) = m.unsplit.clone().unwrap();
        assert_eq!(h, "@@ -1,1 +1,2 @@");
        assert_eq!(b.len(), 3);
    }

    #[test]
    fn pure_hunks_keep_no_group() {
        // WI-6 slice 3a: pure hunks parse exactly as before — no group
        // id, no unsplit original.
        let section = "diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,1 +2,0 @@\n-fn old() {}\n@@ -5,0 +5,1 @@\n+fn new() {}\n";
        let parts = split_section(section, 0).expect("must parse");
        assert_eq!(parts.hunks.len(), 2);
        for h in &parts.hunks {
            assert_eq!(h.group, None);
            assert!(h.unsplit.is_none());
        }
        assert_eq!(parts.hunks[0].body, vec!["-fn old() {}".to_string()]);
        assert_eq!(parts.hunks[1].body, vec!["+fn new() {}".to_string()]);
    }

    #[test]
    fn interleaved_body_splits_into_ordered_runs() {
        // WI-6 slice 3a: interleaved `-`/`+` lines split into one run
        // per maximal pure block with walked anchors — a later plus
        // run anchors past the consumed old lines, not at the native
        // hunk start.
        let section =
            "diff --git c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -5,2 +7,2 @@\n-l5\n+m7\n-l6\n+m8\n";
        let parts = split_section(section, 0).expect("must parse");
        assert_eq!(parts.hunks.len(), 4);
        for h in &parts.hunks {
            assert_eq!(h.group, Some(0));
            assert!(h.unsplit.is_some());
        }
        assert_eq!(parts.hunks[0].header, "@@ -5,1 +7,0 @@");
        assert_eq!(parts.hunks[1].header, "@@ -5,0 +7,1 @@");
        assert_eq!(parts.hunks[2].header, "@@ -6,1 +8,0 @@");
        assert_eq!(parts.hunks[3].header, "@@ -6,0 +8,1 @@");
        assert_eq!(
            (
                parts.hunks[1].old_start,
                parts.hunks[1].old_len,
                parts.hunks[1].new_start,
                parts.hunks[1].new_len
            ),
            (5, 0, 7, 1)
        );
        assert_eq!(
            (
                parts.hunks[3].old_start,
                parts.hunks[3].old_len,
                parts.hunks[3].new_start,
                parts.hunks[3].new_len
            ),
            (6, 0, 8, 1)
        );
        let (h, b) = parts.hunks[0].unsplit.clone().unwrap();
        assert_eq!(h, "@@ -5,2 +7,2 @@");
        assert_eq!(b.len(), 4);
    }

    #[test]
    fn interior_single_attr_gap_holds() {
        // E1 slice 3: one uncovered single-attr line between two covered
        // runs holds — the rebuild emits it as an interior ordinary hunk
        // (disjoint old-range deletion), so nothing is dropped.
        let body: Vec<String> = vec![
            "-fn a() {}".to_string(),
            "-".to_string(),
            "-#[old]".to_string(),
            "-fn b() {}".to_string(),
        ];
        let pairs = vec![test_pair(0, 0), test_pair(3, 3)];
        assert!(minus_hunk_holds(&body, &pairs, 0, 0));
    }

    #[test]
    fn interior_two_attr_gap_vetoes() {
        // Two removed attrs in one gap stay ordinary (no pair may claim
        // a two-line interior; the whole sweep degrades instead).
        let body: Vec<String> = vec![
            "-fn a() {}".to_string(),
            "-#[a]".to_string(),
            "-#[b]".to_string(),
            "-fn b() {}".to_string(),
        ];
        let pairs = vec![test_pair(0, 0), test_pair(3, 3)];
        assert!(!minus_hunk_holds(&body, &pairs, 0, 0));
    }

    #[test]
    fn interior_code_gap_vetoes() {
        // Unpaired code between paired runs is never silently droppable.
        let body: Vec<String> = vec![
            "-fn a() {}".to_string(),
            "-let x = 1;".to_string(),
            "-fn b() {}".to_string(),
        ];
        let pairs = vec![test_pair(0, 0), test_pair(2, 2)];
        assert!(!minus_hunk_holds(&body, &pairs, 0, 0));
    }

    #[test]
    fn multi_cut_nonblank_preceding_last_is_sound() {
        // E1 slice 3: `[aaa, blank, #[old], bbb]` at EOF — the last cut
        // (bbb) is preceded by the non-blank removed attr, so it absorbs
        // nothing and the two-cut set stays sound (no capping).
        let body: Vec<String> = vec![
            "-fn a() {}".to_string(),
            "-".to_string(),
            "-#[old]".to_string(),
            "-fn b() {}".to_string(),
        ];
        let pairs = vec![test_pair(0, 0), test_pair(3, 3)];
        let old = "fn keep() {}\nfn a() {}\n\n#[old]\nfn b() {}\n";
        assert!(minus_last_cut_sound(&body, &pairs, 0, 0, 2, old));
    }

    #[test]
    fn multi_cut_blank_preceding_last_at_eof_stays_unsound() {
        // Counterpart: two bare companions `[aaa, blank, bbb]` at EOF —
        // the last cut's preceding blank is already claimed by the
        // first cut's following-blank absorption, so without a following
        // blank the set still caps to one (existing behavior kept).
        let body: Vec<String> = vec![
            "-fn a() {}".to_string(),
            "-".to_string(),
            "-fn b() {}".to_string(),
        ];
        let pairs = vec![test_pair(0, 0), test_pair(2, 2)];
        let old = "fn keep() {}\nfn a() {}\n\nfn b() {}\n";
        assert!(!minus_last_cut_sound(&body, &pairs, 0, 0, 2, old));
    }
}
