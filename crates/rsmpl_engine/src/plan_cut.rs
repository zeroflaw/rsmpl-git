//! Pass-1 `# bind=` cut loop (SPEC 23.1 step 5).
//!
//! D5 sibling of `plan.rs`: evaluates item-capture cut hunks and records
//! the captured source text. Forward references work because uses resolve
//! only in pass 2. No planner state lives here; the caller owns `states`,
//! `captures`, `file_edits`, `move_cuts`, and `rule_order`.

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, Hunk, Patch, Scope, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::capture::CaptureStore;
use crate::edits::TextEdit;
use crate::lines::FileState;
use crate::matcher_fn;
use crate::plan_dispatch::range_err;
use crate::suggest::candidate_preview;
use crate::validate::{ensure_match_count, gate_directives, ZeroMatchHint};
use crate::{RuleReport, RuleSemantics};

/// Evaluate every `# bind=` cut hunk in document order (pass 1).
///
/// Verbatim move of the `plan.rs` pass-1 loop: same fail-fast order,
/// same reports, same edits. The caller passes the planner's maps by
/// mutable reference; this function holds no state of its own.
pub(crate) fn run_pass1(
    patch: &Patch,
    states: &HashMap<String, FileState>,
    captures: &mut CaptureStore,
    file_edits: &mut HashMap<String, Vec<TextEdit>>,
    move_cuts: &mut Vec<crate::postcheck_move::MoveCut>,
    rule_order: &mut Vec<((usize, usize), RuleReport)>,
) -> Result<(), StructuredError> {
    for (sec_idx, section) in patch.files.iter().enumerate() {
        let path = &section.target_path;
        let Some(state) = states.get(path) else {
            continue;
        };
        for (hunk_idx, hunk) in section.hunks.iter().enumerate() {
            let Hunk::Rsmpl(rh) = hunk else { continue };
            let binds = rh.binds();
            if binds.is_empty() {
                continue;
            }
            if binds.len() > 1 {
                return Err(StructuredError::new(
                    ErrorCode::CaptureConflict,
                    "only one `# bind=<Name>` per hunk is supported in v0.1",
                )
                .with_rule(rh.rule_name())
                .with_file(path)
                .with_repair(StructuredError::fix_pattern_repair(
                    "bind",
                    "use one `# bind=<Name>` per hunk (one collection per hunk)",
                )));
            }
            let compiled = rsmpl_pattern::compile(rh)
                .map_err(|e| e.with_rule(rh.rule_name()).with_file(path))?;
            gate_directives(rh, &compiled, path)?;
            if !matches!(&compiled.kind, PatternKind::ItemCapture { .. }) {
                return Err(StructuredError::new(
                    ErrorCode::CaptureConflict,
                    "v0.1 `# bind=` is only supported on item-capture cut hunks (`- I`)",
                )
                .with_rule(rh.rule_name())
                .with_file(path)
                .with_repair(StructuredError::fix_pattern_repair(
                    "bind",
                    "write an item-capture cut hunk (`- I` with `# bind=<Name>`)",
                )));
            }
            let parsed = state.parsed.as_ref().expect(
                "T137: RSmPL cut hunk on a file with no parsed AST (ordinary-only files carry no semantic hunks)",
            );
            let targets = matcher_fn::find_item_targets(
                parsed,
                rh.scope(),
                rh.header.old_start,
                rh.header.old_len,
            );
            let range_desc = format!(
                "{}..{}",
                rh.header.old_start,
                rh.header.old_start + rh.header.old_len
            );
            // WI-6: an optional collection cut with no match binds the
            // empty collection (the paste then splices nothing). This
            // check precedes the scalar-optional `continue` below.
            if targets.is_empty() && rh.allow() == rsmpl_patch::Allow::Many && rh.is_optional() {
                let report = crate::capture::eval_empty_collection_cut(
                    &rh.rule_name(),
                    path,
                    binds[0],
                    captures,
                )?;
                rule_order.push(((sec_idx, hunk_idx), report));
                continue;
            }
            if targets.is_empty() && rh.is_optional() {
                continue; // optional cut with no match: no capture, no cut
            }
            // T62: same whole-file probe for cut hunks (span collection,
            // no edits involved).
            let wide = if targets.is_empty() && rh.scope() != Scope::File {
                let wide_targets = matcher_fn::find_item_targets(
                    parsed,
                    Scope::File,
                    rh.header.old_start,
                    rh.header.old_len,
                );
                Some(wide_targets.len())
            } else {
                None
            };
            // T67: nearest-item excerpt so the retry often needs no read.
            let preview = if targets.is_empty() {
                candidate_preview(state, rh, &compiled)
            } else {
                None
            };
            ensure_match_count(
                targets.len(),
                rh.allow(),
                false,
                &rh.rule_name(),
                path,
                &range_desc,
                ZeroMatchHint { wide, preview },
            )?;
            // WI-6: a collection cut captures and deletes every in-scope
            // item in source order (helper in `capture.rs`, D5); the
            // scalar path keeps `targets[0]`.
            if rh.allow() == rsmpl_patch::Allow::Many {
                let report = crate::capture::eval_collection_cut(
                    state,
                    path,
                    &rh.rule_name(),
                    binds[0],
                    &targets,
                    captures,
                    file_edits.get_mut(path).expect("file state"),
                )?;
                // WI-7b: collection cuts join the move-pair probe per
                // item (same capture name links cut to bare paste);
                // the empty-collection cut binds nothing to move.
                if !targets.is_empty() {
                    move_cuts.push(crate::postcheck_move::MoveCut {
                        rule: rh.rule_name(),
                        file: path.clone(),
                        spans: targets.clone(),
                        name: binds[0].to_string(),
                    });
                }
                rule_order.push(((sec_idx, hunk_idx), report));
            } else {
                let (sl, el) = targets[0];
                // Capture the exact item span (no blank-line absorption); the
                // delete edit may absorb an adjacent blank separator line.
                let cap_start = state
                    .index
                    .line_start(sl)
                    .ok_or_else(|| range_err(&rh.rule_name(), path, sl))?;
                let cap_end = state
                    .index
                    .line_start(el + 1)
                    .unwrap_or_else(|| state.index.eof());
                let cut = matcher_fn::delete_lines(
                    &state.original,
                    &state.index,
                    sl,
                    el,
                    &rh.rule_name(),
                )
                .map_err(|e| e.with_file(path))?;
                let captured = state
                    .original
                    .get(cap_start..cap_end)
                    .unwrap_or("")
                    .to_string();
                captures.define(binds[0], captured, &rh.rule_name(), path)?;
                file_edits.get_mut(path).expect("file state").push(cut);
                move_cuts.push(crate::postcheck_move::MoveCut {
                    rule: rh.rule_name(),
                    file: path.clone(),
                    spans: vec![(sl, el)],
                    name: binds[0].to_string(),
                });
                rule_order.push((
                    (sec_idx, hunk_idx),
                    RuleReport {
                        rule: rh.rule_name(),
                        file: path.clone(),
                        matches: 1,
                        spans: vec![(sl, el)],
                        span_labels: Vec::new(),
                        sites: Vec::new(),
                        follow_skipped_details: Vec::new(),
                        orphans: Vec::new(),
                        semantics: RuleSemantics::ReparseOnly,
                        symbol: None,
                    },
                ));
            }
        }
    }
    Ok(())
}
