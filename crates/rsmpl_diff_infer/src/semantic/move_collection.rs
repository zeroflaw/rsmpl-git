//! WI-6 collection move emission (D5 sibling of `moves.rs`).
//!
//! A collection cell is one `(minus_hunk, plus_hunk)` cell of `final_pairs`
//! holding >= 3 distinct moved items that covers every pair of its minus
//! hunk and every paired run of its plus hunk (no fan-out interleave, no
//! split anchors, no evolved pairs). The cell emits one ranged
//! `# allow=many` cut (`- I` binds every in-scope item in source order)
//! plus one bare `+ Mk` paste when that spelling beats the scalar fan-in
//! under T375; otherwise the cell keeps scalar emission.
//!
//! The rebuild in `moves.rs` is untouched: each cell contributes one
//! `named` entry per member pair sharing a single cut string, with the
//! paste string carried by the plus-first primary only (empty elsewhere),
//! so per-hunk cut dedup, leftover bounds, and byte accounting all reuse
//! the scalar paths exactly.

use std::collections::{HashMap, HashSet};

use super::move_parts::{MovePair, SectionParts};

/// One planned collection cell: the member pair indices into the
/// `final_pairs` slice handed to [`plan_collections`], plus the concrete
/// emission strings (rule ids and bind already reserved).
pub(crate) struct PlannedCell {
    /// Indices into the `final_pairs` slice (document/minus order).
    pub members: Vec<usize>,
    /// Index of the plus-first member: carries the paste string.
    pub primary: usize,
    pub cut: String,
    pub paste: String,
}

/// Group `final_pairs` into collection cells, reserving rule ids and binds.
///
/// `bind_seq` is the shared `M{n}` counter owned by the caller (the scalar
/// naming loop keeps using it afterwards, so numbering stays dense and
/// deterministic). Only strictly-smaller cells are returned; ties and
/// smaller wins keep scalar emission.
pub(crate) fn plan_collections(
    final_pairs: &[MovePair],
    parsed: &[SectionParts],
    used_rules: &mut Vec<String>,
    bind_seq: &mut usize,
) -> Vec<PlannedCell> {
    let mut by_cell: HashMap<CellKey, Vec<usize>> = HashMap::new();
    for (idx, p) in final_pairs.iter().enumerate() {
        by_cell
            .entry((p.minus_hunk_key, p.plus_hunk_key))
            .or_default()
            .push(idx);
    }
    let mut cells: Vec<(CellKey, Vec<usize>)> = by_cell.into_iter().collect();
    // Deterministic emission order: minus document order.
    cells.sort_by_key(|(k, members)| (*k, members.first().copied().unwrap_or(0)));
    let mut out = Vec::new();
    for (_, members) in cells {
        let Some(cell) = plan_cell(&members, final_pairs, parsed) else {
            continue;
        };
        // Structural byte comparison with fixed-width placeholder ids
        // (`r0`/`M0`): rule-id widths are positional noise beside the
        // hunk-count gap, so placeholders measure the honest delta.
        if !(cell.collection_bytes < cell.scalar_bytes) {
            continue;
        }
        *bind_seq += 1;
        let bind = format!("M{bind_seq}");
        let cut_rule = super::unique_rule(used_rules, "collection");
        let paste_rule = super::unique_rule(used_rules, "collection");
        out.push(cell.emit(&bind, &cut_rule, &paste_rule));
    }
    out
}

/// A (minus hunk, plus hunk) cell key: pair slots into `final_pairs`.
type CellKey = ((usize, usize), (usize, usize));

/// Look up the planned cell (and whether `pair_idx` is its primary) for a
/// `final_pairs` index. `None` keeps scalar emission.
pub(crate) fn cell_for(cells: &[PlannedCell], pair_idx: usize) -> Option<(&PlannedCell, bool)> {
    cells.iter().find_map(|c| {
        if c.members.contains(&pair_idx) {
            Some((c, c.primary == pair_idx))
        } else {
            None
        }
    })
}

struct CellDraft {
    members: Vec<usize>,
    primary: usize,
    scalar_bytes: usize,
    collection_bytes: usize,
    cut_start: u32,
    cut_len: u32,
    anchor: u32,
}

impl CellDraft {
    fn emit(self, bind: &str, cut_rule: &str, paste_rule: &str) -> PlannedCell {
        let cut = format!(
            "@@ -{},{} @@ rule={cut_rule}\n# allow=many\n# bind={bind}\n- I\n",
            self.cut_start, self.cut_len
        );
        let paste = format!(
            "@@ -{},0 @@ rule={paste_rule}\n# use={bind}\n+ {bind}\n",
            self.anchor
        );
        PlannedCell {
            members: self.members,
            primary: self.primary,
            cut,
            paste,
        }
    }
}

/// Validate one cell and draft its byte comparison. `None` keeps scalar.
fn plan_cell(
    members: &[usize],
    final_pairs: &[MovePair],
    parsed: &[SectionParts],
) -> Option<CellDraft> {
    // Distinct item spans; >= 3 items move together (SPEC D.4).
    let mut spans: HashSet<(usize, usize)> = HashSet::new();
    for &i in members {
        let p = &final_pairs[i];
        if p.evolved.is_some() {
            return None;
        }
        spans.insert((p.item.first, p.item.last));
    }
    if spans.len() < 3 || spans.len() != members.len() {
        return None;
    }
    let (minus_key, plus_key) = (
        final_pairs[members[0]].minus_hunk_key,
        final_pairs[members[0]].plus_hunk_key,
    );
    // No fan-out interleave: the cell owns every pair of its minus hunk
    // and every pair of its plus hunk (each member span fires exactly
    // once, so one cut deletes each item exactly once).
    if final_pairs
        .iter()
        .any(|p| (p.minus_hunk_key == minus_key) != (p.plus_hunk_key == plus_key))
    {
        return None;
    }
    let hunk = &parsed[minus_key.0].hunks[minus_key.1];
    let min_first = members
        .iter()
        .map(|&i| final_pairs[i].item.first)
        .min()
        .unwrap_or(0);
    let max_last = members
        .iter()
        .map(|&i| final_pairs[i].item.last)
        .max()
        .unwrap_or(0);
    let cut_start = hunk.old_start + min_first as u32;
    let cut_len = (max_last - min_first + 1) as u32;
    let anchor = parsed[plus_key.0].hunks[plus_key.1].old_start;
    // Scalar spelling (placeholder ids): one tight cut per span (minus
    // order, as the naming loop emits) plus one paste per pair.
    let mut scalar_bytes = 0usize;
    let mut ordered: Vec<usize> = members.to_vec();
    ordered.sort_by_key(|&i| final_pairs[i].item.first);
    for &i in &ordered {
        let p = &final_pairs[i];
        let start = hunk.old_start + p.item.first as u32;
        let len = (p.item.last - p.item.first + 1) as u32;
        scalar_bytes += format!("@@ -{start},{len} @@ rule=r0\n# bind=M0\n- I\n").len();
        scalar_bytes += format!("@@ -{anchor},0 @@ rule=r0\n# use=M0\n+ M0\n").len();
    }
    let collection_bytes =
        format!("@@ -{cut_start},{cut_len} @@ rule=r0\n# allow=many\n# bind=M0\n- I\n").len()
            + format!("@@ -{anchor},0 @@ rule=r0\n# use=M0\n+ M0\n").len();
    // Plus-first primary: the rebuild's leftover bounds read every member;
    // only the primary carries the paste string.
    let primary = *members
        .iter()
        .min_by_key(|&&i| final_pairs[i].plus_first)
        .unwrap_or(&members[0]);
    Some(CellDraft {
        members: members.to_vec(),
        primary,
        scalar_bytes,
        collection_bytes,
        cut_start,
        cut_len,
        anchor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::move_parts::{HunkParts, MoveItem};

    fn item(ident: &str, first: usize, last: usize) -> MoveItem {
        let lines: Vec<String> = (first..=last)
            .map(|l| format!("-line{l} // {ident}"))
            .collect();
        MoveItem {
            ident: ident.to_string(),
            text: lines.join("\n"),
            lines,
            first,
            last,
        }
    }

    fn hunk(old_start: u32, body: &[&str]) -> HunkParts {
        HunkParts {
            header: format!("@@ -{old_start},{} @@", body.len()),
            old_start,
            old_len: body.len() as u32,
            new_start: old_start,
            new_len: 0,
            body: body.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn section(path: &str, hunks: Vec<HunkParts>) -> SectionParts {
        SectionParts {
            orig_idx: 0,
            path: path.to_string(),
            is_deleted: false,
            prelude: format!("diff --git {path}"),
            hunks,
        }
    }

    fn pair(
        minus: (usize, usize),
        plus: (usize, usize),
        ident: &str,
        first: usize,
        last: usize,
        plus_first: usize,
        plus_last: usize,
    ) -> MovePair {
        MovePair {
            minus_hunk_key: minus,
            plus_hunk_key: plus,
            item: item(ident, first, last),
            plus_first,
            plus_last,
            evolved: None,
        }
    }

    fn validated_live(patch: &str, pre: &std::collections::HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    #[test]
    fn three_item_cell_plans_collection() {
        // Three items, one minus hunk, one plus hunk: plans.
        let parsed = vec![
            section(
                "a.rs",
                vec![hunk(
                    2,
                    &["-fn a() {}", "-", "-fn b() {}", "-", "-fn c() {}"],
                )],
            ),
            section(
                "b.rs",
                vec![hunk(
                    2,
                    &["+fn a() {}", "+", "+fn b() {}", "+", "+fn c() {}"],
                )],
            ),
        ];
        let pairs = vec![
            pair((0, 0), (1, 0), "a", 0, 0, 0, 0),
            pair((0, 0), (1, 0), "b", 2, 2, 2, 2),
            pair((0, 0), (1, 0), "c", 4, 4, 4, 4),
        ];
        let mut used = Vec::new();
        let mut seq = 0;
        let cells = plan_collections(&pairs, &parsed, &mut used, &mut seq);
        assert_eq!(cells.len(), 1, "must plan one cell");
        assert!(
            cells[0].cut.contains("# allow=many\n"),
            "got: {}",
            cells[0].cut
        );
        assert!(cells[0].cut.contains("- I\n"), "got: {}", cells[0].cut);
        assert!(cells[0].paste.contains("+ M1\n"), "got: {}", cells[0].paste);
        assert_eq!(seq, 1);
        assert_eq!(used, vec!["r1".to_string(), "r2".to_string()]);
    }

    #[test]
    fn two_item_cell_stays_scalar() {
        let parsed = vec![
            section("a.rs", vec![hunk(2, &["-fn a() {}", "-", "-fn b() {}"])]),
            section("b.rs", vec![hunk(2, &["+fn a() {}", "+", "+fn b() {}"])]),
        ];
        let pairs = vec![
            pair((0, 0), (1, 0), "a", 0, 0, 0, 0),
            pair((0, 0), (1, 0), "b", 2, 2, 2, 2),
        ];
        let mut used = Vec::new();
        let mut seq = 0;
        assert!(plan_collections(&pairs, &parsed, &mut used, &mut seq).is_empty());
        assert!(used.is_empty(), "no ids reserved, got: {used:?}");
        assert_eq!(seq, 0);
    }

    #[test]
    fn split_anchor_cell_stays_scalar() {
        // Three items, one minus hunk, but split across two plus hunks:
        // no cell covers its minus hunk, so all scalar.
        let parsed = vec![
            section(
                "a.rs",
                vec![hunk(
                    2,
                    &["-fn a() {}", "-", "-fn b() {}", "-", "-fn c() {}"],
                )],
            ),
            section(
                "b.rs",
                vec![
                    hunk(2, &["+fn a() {}", "+", "+fn b() {}"]),
                    hunk(6, &["+fn c() {}"]),
                ],
            ),
        ];
        let pairs = vec![
            pair((0, 0), (1, 0), "a", 0, 0, 0, 0),
            pair((0, 0), (1, 0), "b", 2, 2, 2, 2),
            pair((0, 0), (1, 1), "c", 4, 4, 0, 0),
        ];
        let mut used = Vec::new();
        let mut seq = 0;
        assert!(plan_collections(&pairs, &parsed, &mut used, &mut seq).is_empty());
    }

    #[test]
    fn fanout_member_vetoes_cell() {
        // `a` fans out to two plus hunks: the (m,p0) cell does not own
        // every pair of its minus hunk — all scalar.
        let parsed = vec![
            section(
                "a.rs",
                vec![hunk(
                    2,
                    &["-fn a() {}", "-", "-fn b() {}", "-", "-fn c() {}"],
                )],
            ),
            section(
                "b.rs",
                vec![
                    hunk(2, &["+fn a() {}", "+", "+fn b() {}", "+", "+fn c() {}"]),
                    hunk(9, &["+fn a() {}"]),
                ],
            ),
        ];
        let pairs = vec![
            pair((0, 0), (1, 0), "a", 0, 0, 0, 0),
            pair((0, 0), (1, 1), "a", 0, 0, 0, 0),
            pair((0, 0), (1, 0), "b", 2, 2, 2, 2),
            pair((0, 0), (1, 0), "c", 4, 4, 4, 4),
        ];
        let mut used = Vec::new();
        let mut seq = 0;
        assert!(plan_collections(&pairs, &parsed, &mut used, &mut seq).is_empty());
    }

    #[test]
    fn evolved_pair_vetoes_cell() {
        let parsed = vec![
            section(
                "a.rs",
                vec![hunk(
                    2,
                    &["-fn a() {}", "-", "-fn b() {}", "-", "-fn c() {}"],
                )],
            ),
            section(
                "b.rs",
                vec![hunk(
                    2,
                    &["+fn a() {}", "+", "+fn b() {}", "+", "+fn c() {}"],
                )],
            ),
        ];
        let mut p = pair((0, 0), (1, 0), "c", 4, 4, 4, 4);
        p.evolved = Some(crate::semantic::move_parts::EvolvedPair {
            kind: crate::semantic::move_parts::EvolvedKind::Fn,
            prefix: String::new(),
            name: "c".to_string(),
            static_mut: false,
            template: None,
        });
        let pairs = vec![
            pair((0, 0), (1, 0), "a", 0, 0, 0, 0),
            pair((0, 0), (1, 0), "b", 2, 2, 2, 2),
            p,
        ];
        let mut used = Vec::new();
        let mut seq = 0;
        assert!(plan_collections(&pairs, &parsed, &mut used, &mut seq).is_empty());
    }

    #[test]
    fn collection_round_trips_live() {
        // Full cut+paste spelling applies against a real pre-image.
        let parsed = vec![
            section(
                "a.rs",
                vec![hunk(
                    2,
                    &["-fn a() {}", "-", "-fn b() {}", "-", "-fn c() {}"],
                )],
            ),
            section(
                "b.rs",
                vec![hunk(
                    2,
                    &["+fn a() {}", "+", "+fn b() {}", "+", "+fn c() {}"],
                )],
            ),
        ];
        let pairs = vec![
            pair((0, 0), (1, 0), "a", 0, 0, 0, 0),
            pair((0, 0), (1, 0), "b", 2, 2, 2, 2),
            pair((0, 0), (1, 0), "c", 4, 4, 4, 4),
        ];
        let mut used = Vec::new();
        let mut seq = 0;
        let cells = plan_collections(&pairs, &parsed, &mut used, &mut seq);
        assert_eq!(cells.len(), 1);
        let patch = format!(
            "diff --git a.rs\n{}\ndiff --git b.rs\n{}",
            cells[0].cut, cells[0].paste
        );
        let mut pre = std::collections::HashMap::new();
        pre.insert(
            "a.rs".to_string(),
            "fn keep() {}\nfn a() {}\n\nfn b() {}\n\nfn c() {}\n".to_string(),
        );
        pre.insert("b.rs".to_string(), "use x;\nuse y;\n".to_string());
        validated_live(&patch, &pre);
    }

    /// Local big-item builder (mirrors the moves.rs harness): a 42-line
    /// fn so the T375 byte-win guard clears on multi-item cells.
    fn cell_big_fn(name: &str) -> String {
        let mut s = format!("fn {name}() {{\n");
        for i in 0..40 {
            s.push_str(&format!("    let v{i} = {i};\n"));
        }
        s.push_str("}\n");
        s
    }

    fn cell_pre_map(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn move_excluded_trailing_blank_degrades_to_prefix() {
        // WI-6 soundness: git's minus hunk excludes the trailing blank
        // after the last moved item (real alignment, probed 2026-10).
        // The last cut would absorb that kept line, so the gate caps to
        // the sound prefix (`one` moves, `two` rides ordinary) instead
        // of emitting a divergent patch.
        let one = cell_big_fn("one");
        let two = cell_big_fn("two");
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
        let pre = cell_pre_map(&[
            (
                "a.rs",
                &format!("fn keep() {{}}\n{one}\n{two}\n\nfn tail() {{}}"),
            ),
            ("b.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = crate::semantic::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("prefix must fire");
        let text = out.join("");
        assert!(
            text.contains("@@ -2,42 @@ rule=r1\n# bind=M1\n- I\n"),
            "got:\n{text}"
        );
        assert!(
            text.contains("-fn two() {"),
            "two stays ordinary, got:\n{text}"
        );
        assert!(
            text.contains("+fn two() {"),
            "two stays ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn move_included_trailing_blank_fires_byte_identical() {
        // WI-6 soundness companion: when git's minus hunk INCLUDES the
        // trailing blank (real alignment, probed 2026-10), the last cut
        // absorbs an in-hunk line and the emission round-trips byte-exact.
        // Three items land in one collection cut+paste here.
        let one = cell_big_fn("one");
        let two = cell_big_fn("two");
        let three = cell_big_fn("three");
        let mut a_minus = String::new();
        for l in one.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        a_minus.push_str("-\n");
        for l in two.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        a_minus.push_str("-\n");
        for l in three.lines() {
            a_minus.push_str(&format!("-{l}\n"));
        }
        a_minus.push_str("-\n");
        let n = 3 * one.lines().count() + 3;
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,{n} +2,0 @@\n{a_minus}");
        let mut b_plus = String::new();
        for l in one.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        b_plus.push_str("+\n");
        for l in two.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        b_plus.push_str("+\n");
        for l in three.lines() {
            b_plus.push_str(&format!("+{l}\n"));
        }
        let m = 3 * one.lines().count() + 2;
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,{m} @@\n{b_plus}");
        let a_old = format!("fn keep() {{}}\n{one}\n{two}\n{three}\nfn tail() {{}}\n");
        let pre = cell_pre_map(&[("a.rs", &a_old), ("b.rs", "use x;\n")]);
        let mut used = Vec::new();
        let out = crate::semantic::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("collection must fire");
        let text = out.join("");
        assert!(
            text.contains("# allow=many\n# bind=M1\n- I\n"),
            "got:\n{text}"
        );
        assert!(text.contains("# use=M1\n+ M1\n"), "got:\n{text}");
        assert!(
            !text.contains("-fn two() {}"),
            "no ordinary leftovers, got:\n{text}"
        );
        let parsed = rsmpl_patch::parse_patch(&text).expect("emitted patch must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
        assert_eq!(
            files["a.rs"], "fn keep() {}\nfn tail() {}\n",
            "got: {:?}",
            files["a.rs"]
        );
        // Paste join: one blank between items, single trailing newline.
        let b_want = format!("use x;\n{one}\n{two}\n{three}");
        assert_eq!(files["b.rs"], b_want, "got: {:?}", files["b.rs"]);
    }
}
