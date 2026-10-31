//! Anchor fallback for stale ranged hunks (WI-10b, SPEC 10.8).
//!
//! When a ranged semantic hunk finds zero matches in its cited range, the
//! planner retries once with file scope, anchored by the hunk's body
//! context (` `) lines: leading context (before the first `-`/`+` line)
//! must occur in order in the lines just before the single file-wide
//! candidate, trailing context (after the last `-`/`+` line) in order in
//! the lines just after it. Exactly one anchored candidate applies with
//! `drift` evidence; zero or more than one keeps the original conflict.
//!
//! Anchors are trimmed-literal (never fuzzy); patterns still match
//! semantically. `# scope=file` / sweep hunks never drift (no locality
//! claim), nor do insertions (anchor-positioned already), `# optional`
//! hunks (zero is success), or ranged `# allow=many` (the exactly-one
//! gate contradicts fan-out — use `# scope=file` for mobile sweeps).

use rsmpl_patch::{Allow, RsmplHunk, Scope};
use rsmpl_pattern::{CompiledPattern, PatternKind};
use syn::spanned::Spanned;

/// Lines of file context searched on each side of a drift candidate.
const ANCHOR_WINDOW: usize = 10;

/// A successful drift retry: scratch edits to splice into the plan, the
/// per-site bindings for the normal evidence path, and the bounding-box
/// `actual` span for the drift report.
pub(crate) struct DriftOutcome {
    pub(crate) edits: Vec<crate::edits::TextEdit>,
    pub(crate) bindings: Vec<std::collections::HashMap<String, String>>,
    pub(crate) actual: (u32, u32),
}

/// Split body context lines into (leading, trailing) anchors.
///
/// Leading = ` ` lines before the first `-`/`+` line; trailing = ` ` lines
/// after the last one. Whitespace-only and `...`-only lines carry no
/// disambiguation and are dropped; the rest compare trimmed-literal.
pub(crate) fn drift_anchors(rh: &RsmplHunk) -> (Vec<String>, Vec<String>) {
    let mut first = None;
    let mut last = None;
    for (i, l) in rh.lines.iter().enumerate() {
        if l.is_removal() || l.is_addition() {
            if first.is_none() {
                first = Some(i);
            }
            last = Some(i);
        }
    }
    let (Some(f), Some(e)) = (first, last) else {
        return (Vec::new(), Vec::new());
    };
    let keep = |t: &str| {
        let s = t.trim();
        !s.is_empty() && s != "..."
    };
    let leading = rh.lines[..f]
        .iter()
        .filter(|l| l.is_context() && keep(&l.text))
        .map(|l| l.text.trim().to_string())
        .collect();
    let trailing = rh.lines[e + 1..]
        .iter()
        .filter(|l| l.is_context() && keep(&l.text))
        .map(|l| l.text.trim().to_string())
        .collect();
    (leading, trailing)
}

/// Whether this hunk may drift-retry on a zero match (SPEC 10.8).
///
/// Header-context shapes (`impl`/`trait` contexts, member renames) are
/// excluded: their header is already a precise file-wide selector, so a
/// range/header disagreement is an authoring error the WI-4 pins keep
/// loud (the T62 `# scope=file` hint offers the exact retry). Drift
/// serves shapes where the cited range is the only locality claim.
pub(crate) fn is_drift_eligible(rh: &RsmplHunk, compiled: &CompiledPattern) -> bool {
    if rh.scope() == Scope::File {
        return false;
    }
    if rh.allow() != Allow::One || rh.is_optional() {
        return false;
    }
    !matches!(
        &compiled.kind,
        PatternKind::Insertion { .. }
            | PatternKind::ItemCapture { .. }
            | PatternKind::Raw { .. }
            | PatternKind::ImplContext { .. }
            | PatternKind::FieldRename { .. }
            | PatternKind::VariantRename { .. }
    )
}

/// True when every leading anchor occurs trimmed-equal in order within
/// `ANCHOR_WINDOW` lines before `span`, and every trailing anchor within
/// `ANCHOR_WINDOW` lines after it. An empty side is vacuous, but the
/// caller guarantees at least one side is non-empty.
pub(crate) fn anchors_satisfied(
    file_lines: &[&str],
    span: (u32, u32),
    leading: &[String],
    trailing: &[String],
) -> bool {
    let start = span.0 as usize;
    let end = span.1 as usize;
    // Lines before the span (1-based `start` → 0-based window end).
    let before_lo = start.saturating_sub(1 + ANCHOR_WINDOW);
    let before_hi = start.saturating_sub(1).min(file_lines.len());
    if !in_order(&file_lines[before_lo..before_hi], leading) {
        return false;
    }
    // Lines after the span (1-based inclusive `end` → 0-based start).
    let after_lo = end.min(file_lines.len());
    let after_hi = (end + ANCHOR_WINDOW).min(file_lines.len());
    in_order(&file_lines[after_lo..after_hi], trailing)
}

/// Ordered-subsequence match of trimmed anchors against trimmed lines.
fn in_order(window: &[&str], anchors: &[String]) -> bool {
    let mut need = anchors.iter();
    let mut cur = need.next();
    for line in window {
        if let Some(want) = cur {
            if line.trim() == want {
                cur = need.next();
            }
        } else {
            break;
        }
    }
    cur.is_none()
}

/// Bounding box over scratch spans: the drifted `actual` range.
pub(crate) fn bounding_box(spans: &[(u32, u32)]) -> Option<(u32, u32)> {
    let mut it = spans.iter();
    let first = *it.next()?;
    let (mut lo, mut hi) = first;
    for &(s, e) in it {
        lo = lo.min(s);
        hi = hi.max(e);
    }
    Some((lo, hi))
}

/// Smallest top-level item span (1-based inclusive lines) containing
/// `actual`, or `None` when the edit sits outside every item (then the
/// caller falls back to the edit bbox itself).
pub(crate) fn enclosing_top_item_span(
    parsed: &syn::File,
    actual: (u32, u32),
) -> Option<(u32, u32)> {
    let mut best: Option<(u32, u32)> = None;
    for item in &parsed.items {
        let s = item.span().start().line as u32;
        let e = item.span().end().line as u32;
        if s <= actual.0 && e >= actual.1 {
            let len = e.saturating_sub(s);
            let better = best.is_none_or(|(bs, be)| len < be.saturating_sub(bs));
            if better {
                best = Some((s, e));
            }
        }
    }
    best
}

/// True when the stale cited range shares no line with `region` (the
/// drifted item). An overlapping range is a scope/filter statement
/// about that item (SPEC 17.3 pins), not line drift — only full
/// disjointness retries.
pub(crate) fn cited_is_disjoint(cited: (u32, u32), region: (u32, u32)) -> bool {
    cited.1 < region.0 || cited.0 > region.1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(text: &str) -> RsmplHunk {
        let patch = rsmpl_patch::parse_patch(text).unwrap();
        match &patch.files[0].hunks[0] {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh.clone(),
            _ => panic!("want rsmpl hunk"),
        }
    }

    #[test]
    fn splits_leading_and_trailing_and_skips_ellipsis() {
        let rh = hunk(
            "diff --git src/lib.rs\n@@ -30,5 @@ rule=r\n  fn before() {}\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n  fn after() {}\n",
        );
        let (lead, trail) = drift_anchors(&rh);
        assert_eq!(lead, vec!["fn before() {}"]);
        // `...` drops; `}` and the trailing item stay.
        assert_eq!(trail, vec!["}", "fn after() {}"]);
    }

    #[test]
    fn no_minus_plus_means_no_anchors() {
        let rh = hunk("diff --git src/lib.rs\n@@ -1,0 @@ rule=p\n# use=H\n+ H\n");
        assert_eq!(drift_anchors(&rh), (Vec::new(), Vec::new()));
    }

    #[test]
    fn satisfaction_needs_order_inside_window() {
        let file = [
            "fn a() {}",
            "fn before() {}",
            "fn foo() {}",
            "fn after() {}",
        ];
        let f: Vec<&str> = file.to_vec();
        assert!(anchors_satisfied(
            &f,
            (3, 3),
            &["fn before() {}".into()],
            &["fn after() {}".into()]
        ));
        // Wrong order fails.
        assert!(!anchors_satisfied(
            &f,
            (3, 3),
            &["fn after() {}".into()],
            &[]
        ));
        // Anchor only at line 1 with the span at line 13: outside the
        // 10-line window, so no drift.
        let mut far = vec!["fn far() {}"];
        far.extend(std::iter::repeat_n("fn pad() {}", 11));
        far.push("fn foo() {}");
        assert!(!anchors_satisfied(
            &far,
            (13, 13),
            &["fn far() {}".into()],
            &[]
        ));
        // Same file, span at line 2: the line-1 anchor is adjacent.
        assert!(anchors_satisfied(
            &far,
            (2, 2),
            &["fn far() {}".into()],
            &[]
        ));
    }

    #[test]
    fn disjointness_gates_scope_probes() {
        // `fn foo` 1-4: a mid-body cited range 2-3 overlaps the item,
        // so it is a scope statement (SPEC 17.3), never drift.
        let parsed: syn::File = syn::parse_str("fn foo() {\n    1\n}\n").unwrap();
        let region = enclosing_top_item_span(&parsed, (1, 1)).unwrap();
        assert_eq!(region, (1, 3));
        assert!(!cited_is_disjoint((2, 3), region));
        // A fully shifted-away range is disjoint: drift-eligible.
        assert!(cited_is_disjoint((10, 12), region));
        // Touching at one line still overlaps.
        assert!(!cited_is_disjoint((3, 5), region));
    }
}
