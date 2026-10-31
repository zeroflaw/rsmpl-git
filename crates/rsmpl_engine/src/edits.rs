//! Byte-range text edits over original coordinates.

use rsmpl_patch::{ErrorCode, StructuredError};

use crate::lines::LineIndex;

/// A replacement of `original[start..end]` with `replacement`.
///
/// WI-2 `label` names the slot for multi-edit rules (`generics`/`params`/
/// `return`/`rename`/`follow`); `None` everywhere else. Labels never
/// affect application or overlap checks — evidence only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
    pub label: Option<&'static str>,
}

impl TextEdit {
    pub fn new(start: usize, end: usize, replacement: String) -> Self {
        debug_assert!(start <= end);
        Self {
            start,
            end,
            replacement,
            label: None,
        }
    }

    pub fn with_label(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn is_insertion(&self) -> bool {
        self.start == self.end
    }

    pub fn overlaps(&self, other: &TextEdit) -> bool {
        // Insertions overlap only if strictly inside another edit's range.
        if self.is_insertion() && other.is_insertion() {
            return false; // same-point insertions are tie-broken by order
        }
        if self.is_insertion() {
            return other.start < self.start && self.start < other.end;
        }
        if other.is_insertion() {
            return self.start < other.start && other.start < self.end;
        }
        self.start < other.end && other.start < self.end
    }

    pub fn contains(&self, other: &TextEdit) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// A whole-range removal (T464): non-empty range, empty replacement.
    /// Overlapping pure deletions union-merge — the union deletes
    /// everything both asked to delete, with no content conflict possible.
    pub fn is_pure_delete(&self) -> bool {
        self.start < self.end && self.replacement.is_empty()
    }
}

/// Apply edits (original coordinates) to `text`. Edits are sorted descending
/// so earlier offsets stay valid. Overlapping non-nested edits are rejected.
/// (Plan-time `coalesce_deletes` union-merges overlapping pure deletions
/// first, so by the time the planner calls this no delete+delete pair remains.)
///
/// Same-point insertions materialize in document order: the first-pushed
/// edit appears first (topmost) in the output (SPEC 20.4, 23.5). The input
/// `Vec` order is document order, so ties break on reverse sequence — the
/// later-doc edit applies first, the earlier-doc edit applies last and ends
/// up first in the file.
pub fn apply_edits(text: &str, edits: Vec<TextEdit>) -> Result<String, StructuredError> {
    let mut indexed: Vec<(usize, TextEdit)> = edits.into_iter().enumerate().collect();
    indexed.sort_by(|(ai, a), (bi, b)| {
        b.start
            .cmp(&a.start)
            .then(b.end.cmp(&a.end))
            .then(bi.cmp(ai))
    });
    let mut out = text.to_string();
    let mut applied: Vec<TextEdit> = Vec::new();
    for (_, e) in indexed {
        if e.end > out.len() || e.start > e.end {
            return Err(StructuredError::new(
                ErrorCode::RangeConflict,
                format!("edit range {}..{} outside file", e.start, e.end),
            ));
        }
        for prev in &applied {
            // `applied` holds edits at higher offsets already applied; `e`
            // is at a lower offset and cannot overlap them unless ranges
            // cross. A crossing check on original coordinates:
            if e.start < prev.start && prev.start < e.end {
                return Err(StructuredError::new(
                    ErrorCode::GitHunkConflict,
                    "overlapping edits cannot be merged",
                ));
            }
        }
        out.replace_range(e.start..e.end, &e.replacement);
        applied.push(e);
    }
    Ok(out)
}

/// Union-merge overlapping pure-deletion edits (T464).
///
/// `delete_lines` absorbs one adjacent blank per item, so adjacent item
/// deletions double-claim their shared separator blank; the overlap gates
/// would reject the pair even though the merged outcome (delete the union)
/// is unambiguous — both edits remove text and insert nothing, so no
/// content conflict is possible. Any overlap involving a replacement or an
/// insertion keeps the existing rejection. Output preserves input
/// document order.
///
/// WI-9f seam rule: touching deletes (`[a,b)` + `[b,c)`) merge only when no
/// insertion anchors at the shared boundary point `b`. A seam-anchored
/// insertion composes at the boundary (`overlaps()` uses strict inequality),
/// but merging first would swallow the seam and turn it into a
/// strictly-inside overlap. Strict overlaps always merge (genuine
/// double-claim).
pub fn coalesce_deletes(edits: Vec<TextEdit>) -> Vec<TextEdit> {
    let seam_points: Vec<usize> = edits
        .iter()
        .filter(|e| e.is_insertion())
        .map(|e| e.start)
        .collect();
    let mut out: Vec<TextEdit> = Vec::new();
    for e in edits {
        if !e.is_pure_delete() {
            out.push(e);
            continue;
        }
        let mut cur = e;
        let mut i = 0;
        while i < out.len() {
            // Touching counts: whole-line deletes meet at line starts
            // (`[a,b)` + `[b,c)`), and the union is still just a deletion —
            // unless an insertion anchors at the seam (WI-9f, above).
            let touching =
                out[i].is_pure_delete() && out[i].start <= cur.end && cur.start <= out[i].end;
            let strict =
                out[i].is_pure_delete() && out[i].start < cur.end && cur.start < out[i].end;
            let seam = out[i].start.max(cur.start);
            let seam_blocked = touching && !strict && seam_points.contains(&seam);
            if touching && !seam_blocked {
                let merged = TextEdit::new(
                    out[i].start.min(cur.start),
                    out[i].end.max(cur.end),
                    String::new(),
                );
                out.remove(i);
                cur = merged;
            } else {
                i += 1;
            }
        }
        out.push(cur);
    }
    out
}

/// Token-normalized equality for code fragments: parse both as expressions
/// (fallback: types, then whitespace-squashed strings) and compare the
/// `quote` rendering. Implements repeated-metavar structural equality
/// (SPEC 15.7) and trailing-comma insensitivity (SPEC 17.7).
pub fn fragments_equal(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if let (Ok(ea), Ok(eb)) = (
        rsmpl_pattern::try_parse_expr(a),
        rsmpl_pattern::try_parse_expr(b),
    ) {
        return norm(&quote::quote!(#ea).to_string()) == norm(&quote::quote!(#eb).to_string());
    }
    if let (Ok(ta), Ok(tb)) = (
        syn::parse_str::<syn::Type>(a),
        syn::parse_str::<syn::Type>(b),
    ) {
        return norm(&quote::quote!(#ta).to_string()) == norm(&quote::quote!(#tb).to_string());
    }
    norm(a) == norm(b)
}

/// Normalize rendered code: drop whitespace and insignificant trailing commas
/// (`(a, b,)` == `(a, b)`, SPEC 17.7).
fn norm(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    s.replace(",)", ")").replace(",]", "]").replace(",}", "}")
}

/// Per-match original-source line spans for one hunk's new edits (T27).
///
/// Each `TextEdit` covers `original[start..end]`; the span is the 1-based
/// inclusive `[line_of(start), line_of(end)]` pair. Two normalizations keep
/// spans tight: insertions (`start == end`) report `[anchor, anchor]`, and an
/// `end` landing exactly on a line start (whole-line deletes) is attributed
/// to the previous line so deleting line 5 reports `[5, 5]`, not `[5, 6]`.
pub(crate) fn edit_spans(index: &LineIndex, new_edits: &[TextEdit]) -> Vec<(u32, u32)> {
    new_edits
        .iter()
        .map(|e| {
            let s = index.line_of(e.start);
            let en = if e.end > e.start && index.line_start(index.line_of(e.end)) == Some(e.end) {
                index.line_of(e.end).saturating_sub(1).max(s)
            } else {
                index.line_of(e.end)
            };
            (s, en)
        })
        .collect()
}

/// WI-2 span edit labels for one hunk's new edits, parallel to
/// `edit_spans` (position `i` labels the edit `edit_spans` spans).
pub(crate) fn edit_span_labels(new_edits: &[TextEdit]) -> Vec<Option<String>> {
    new_edits
        .iter()
        .map(|e| e.label.map(str::to_string))
        .collect()
}

/// Translate original-source line spans to post-image coordinates.
///
/// Rewrites that change line counts (a 6-line `if` collapsing to one
/// line, a call wrapping to four) shift every later site in the file,
/// so original spans miss their output in the post-image. The shift for
/// a span starting at original line `s` is the net newline delta of all
/// edits ending at or before line `s`'s start (boundary insertions at the
/// line start materialize above the line, so they count). Span lengths
/// are preserved; callers extend windows for template growth. Spans past
/// EOF map with the full-file shift (slicing clamps handle the rest).
pub(crate) fn translate_spans_to_post(
    original: &str,
    index: &LineIndex,
    edits: &[TextEdit],
    spans: &[(u32, u32)],
) -> Vec<(u32, u32)> {
    // Newline deltas sorted by original start; disjoint-or-same-point by
    // construction (overlap gates), so a prefix sum over end positions
    // is order-independent.
    let mut deltas: Vec<(usize, i64)> = edits
        .iter()
        .map(|e| {
            let orig_nl = original
                .get(e.start..e.end)
                .map(|s| s.matches('\n').count())
                .unwrap_or(0) as i64;
            let repl_nl = e.replacement.matches('\n').count() as i64;
            (e.end, repl_nl - orig_nl)
        })
        .collect();
    deltas.sort_unstable();
    spans
        .iter()
        .map(|(s, e)| {
            let boundary = index.line_start(*s).unwrap_or(original.len());
            let mut shift: i64 = 0;
            for (end, d) in &deltas {
                if *end <= boundary {
                    shift += *d;
                } else {
                    break;
                }
            }
            let ps = (*s as i64 + shift).max(1) as u32;
            let len = e.saturating_sub(*s) + 1;
            (ps, ps + len - 1)
        })
        .collect()
}
/// Reject incompatible overlapping edits (SPEC 10.6, 23.5).
///
/// Same-point insertions are allowed (deterministic stable order, document
/// order). An insertion strictly inside a deleted/replaced range is rejected,
/// as are partially overlapping replacements.
pub(crate) fn check_overlaps(edits: &[TextEdit], path: &str) -> Result<(), StructuredError> {
    for i in 0..edits.len() {
        for j in (i + 1)..edits.len() {
            let (a, b) = (&edits[i], &edits[j]);
            if a.is_insertion() && b.is_insertion() {
                continue; // same-point insertions: deterministic order
            }
            if a.overlaps(b) {
                return Err(StructuredError::new(
                    ErrorCode::GitHunkConflict,
                    "overlapping edits cannot be merged",
                )
                .with_file(path)
                .with_suggestion(
                    "apply the overlapping rewrites in separate runs, or make the ranges disjoint",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_descending() {
        let out = apply_edits(
            "aaa bbb",
            vec![
                TextEdit::new(0, 3, "x".into()),
                TextEdit::new(4, 7, "y".into()),
            ],
        )
        .unwrap();
        assert_eq!(out, "x y");
    }

    #[test]
    fn trailing_comma_insensitive() {
        assert!(fragments_equal("foo(1, 2)", "foo(1, 2,)"));
        assert!(!fragments_equal("foo()", "bar()"));
    }

    #[test]
    fn same_point_insertions_keep_document_order() {
        // T249: first-pushed insertion appears first in the output.
        let out = apply_edits(
            "AB",
            vec![
                TextEdit::new(1, 1, "X\n".into()),
                TextEdit::new(1, 1, "Y\n".into()),
                TextEdit::new(1, 1, "Z\n".into()),
            ],
        )
        .unwrap();
        assert_eq!(out, "AX\nY\nZ\nB");
    }

    #[test]
    fn coalesce_merges_shared_separator_blank() {
        // Adjacent whole-line deletes double-claiming one blank line
        // merge to the union instead of conflicting (T464).
        let out = coalesce_deletes(vec![
            TextEdit::new(0, 4, String::new()),
            TextEdit::new(4, 8, String::new()),
        ]);
        assert_eq!(out, vec![TextEdit::new(0, 8, String::new())]);
    }

    #[test]
    fn coalesce_merges_nested_deletes() {
        let out = coalesce_deletes(vec![
            TextEdit::new(0, 10, String::new()),
            TextEdit::new(2, 5, String::new()),
        ]);
        assert_eq!(out, vec![TextEdit::new(0, 10, String::new())]);
    }

    #[test]
    fn coalesce_leaves_disjoint_deletes_alone() {
        let out = coalesce_deletes(vec![
            TextEdit::new(0, 4, String::new()),
            TextEdit::new(8, 12, String::new()),
        ]);
        assert_eq!(
            out,
            vec![
                TextEdit::new(0, 4, String::new()),
                TextEdit::new(8, 12, String::new()),
            ]
        );
    }

    #[test]
    fn coalesce_keeps_delete_plus_replace_conflict() {
        // A replacement overlapping a deletion is not a pure pair, so
        // both survive for the overlap gates to reject (T464).
        let out = coalesce_deletes(vec![
            TextEdit::new(0, 8, String::new()),
            TextEdit::new(4, 12, "x".into()),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn coalesce_seam_insertion_blocks_touching_merge() {
        // WI-9f: touching deletes with an insertion anchored at the seam
        // stay separate — merging would swallow the seam and turn the
        // composable boundary insert into a strictly-inside overlap.
        let out = coalesce_deletes(vec![
            TextEdit::new(0, 4, String::new()),
            TextEdit::new(4, 4, "x\n".into()),
            TextEdit::new(4, 8, String::new()),
        ]);
        assert_eq!(
            out,
            vec![
                TextEdit::new(0, 4, String::new()),
                TextEdit::new(4, 4, "x\n".into()),
                TextEdit::new(4, 8, String::new()),
            ]
        );
    }

    #[test]
    fn coalesce_strict_overlap_merges_despite_seam_insertion() {
        // A strictly-overlapping pair still merges even when an insertion
        // sits inside (the insert stays vetoed downstream — merging never
        // hides a genuine conflict, it only avoids false ones).
        let out = coalesce_deletes(vec![
            TextEdit::new(0, 6, String::new()),
            TextEdit::new(4, 4, "x\n".into()),
            TextEdit::new(4, 8, String::new()),
        ]);
        assert_eq!(
            out,
            vec![
                TextEdit::new(4, 4, "x\n".into()),
                TextEdit::new(0, 8, String::new()),
            ]
        );
    }
}
