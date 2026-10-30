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
}

#[derive(Clone)]
pub(crate) struct EvolvedPair {
    pub(crate) kind: EvolvedKind,
    pub(crate) prefix: String,
    pub(crate) name: String,
}

#[derive(Clone)]
pub(crate) struct MovePair {
    pub(crate) minus_hunk_key: (usize, usize),
    pub(crate) plus_hunk_key: (usize, usize),
    pub(crate) item: MoveItem,
    pub(crate) plus_first: usize,
    pub(crate) plus_last: usize,
    /// WI-8a + WI-8 slices 1-3: plus-side header evolution (kind +
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
    for (i, line) in body.iter().enumerate().take(hi + 1).skip(lo) {
        if covered[i] {
            prev_blank = false;
            continue;
        }
        let blank = line_text(line).trim().is_empty();
        if !blank || prev_blank {
            return false;
        }
        prev_blank = true;
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
    if !line_text(&body[lo - 1]).trim().is_empty() {
        return true;
    }
    // Blank-ending prefix with no following blank (prefix-trim): the cut
    // absorbs the preceding blank, and the rebuild trims that one line
    // from the prefix leftover hunk — nothing double-claims it.
    true
}

/// Paired runs in a plus-hunk body, in any position.
/// Paired runs sit anywhere in the plus hunk with exactly one blank
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
