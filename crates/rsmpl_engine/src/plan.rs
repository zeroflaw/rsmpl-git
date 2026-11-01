//! Two-pass application planner (SPEC 23.1).
//!
//! All RSmPL old-side ranges refer to **original-source coordinates**
//! (SPEC 10.2): matching never sees the output of earlier hunks.
//!
//! - Pass 0: parse every targeted file, build line indexes, validate ranges.
//! - Pass 1: evaluate `# bind=` cut hunks (item captures) and record the
//!   captured source text. Forward references work because uses are resolved
//!   only in pass 2 (SPEC 23.2-23.3).
//! - Pass 2: evaluate every other hunk in document order into a single
//!   per-file edit list (original coordinates), resolving `# use=` pastes.
//! - Finally: overlap-check, materialize, reparse-validate, report.

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, Hunk, Patch, Scope, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::capture::CaptureStore;
use crate::edits::{
    apply_edits, check_overlaps, coalesce_deletes, dedupe_identical_edits, edit_spans,
    translate_spans_to_post, TextEdit,
};
use crate::lines::{FileState, LineIndex};
use crate::matcher_fn;
use crate::plan_dispatch::{apply_rsmpl_hunk, range_err};
use crate::postcheck::{
    instantiation_error, is_rewrite_kind, noop_sweep_error, plus_instantiated_parsed,
    sweep_edits_are_noop, sweep_hunk_needs_postcheck,
};
use crate::suggest::candidate_preview;
use crate::use_graph::follow_uses_workspace;
use crate::use_graph_orphan::collect_orphans;
use crate::validate::{ensure_match_count, gate_directives, unbound_plus, ZeroMatchHint};
use crate::{ordinary_hunk_edits, RuleReport, RuleSemantics};

/// Apply `patch` to `files` (path -> contents) in place.
///
/// WI-3b: the engine boundary fills any missing machine `repair`
/// (`fill_generic_repair`) so no conflict is prose-only; specific gates
/// pin exact payloads underneath.
pub fn apply_with_captures(
    files: &mut HashMap<String, String>,
    patch: &Patch,
) -> Result<crate::ApplyReport, StructuredError> {
    apply_inner(files, patch).map_err(crate::validate::fill_generic_repair)
}

fn apply_inner(
    files: &mut HashMap<String, String>,
    patch: &Patch,
) -> Result<crate::ApplyReport, StructuredError> {
    let has_ordinary = patch
        .files
        .iter()
        .flat_map(|s| &s.hunks)
        .any(|h| matches!(h, Hunk::Ordinary(_)));
    let mut report = crate::ApplyReport {
        has_ordinary,
        ..Default::default()
    };

    // ---- Pass 0: parse all targeted files. ----
    // T137: ordinary-only files skip the Rust parse entirely (original +
    // rewritten). The unit is the file: any RSmPL hunk in any section with
    // the same target path means Rust.
    // WI-6b1: a patch containing `# follow=uses` searches the whole loaded
    // workspace (every `.rs` file in the `files` map), not just patch
    // targets — the CLI loads the repo files when follow is present.
    let patch_has_follow = patch
        .files
        .iter()
        .flat_map(|s| &s.hunks)
        .any(|h| matches!(h, Hunk::Rsmpl(rh) if rh.follows_uses()));
    // WI-7c: plain renames need the same workspace for the report-only
    // orphan index (compile-probe; failures validate later as usual).
    let patch_has_rename = patch.files.iter().flat_map(|s| &s.hunks).any(|h| match h {
        Hunk::Rsmpl(rh) => matches!(
            rsmpl_pattern::compile(rh).map(|c| c.kind),
            Ok(PatternKind::Fn { .. }
                | PatternKind::ItemRename { .. }
                | PatternKind::FieldRename { .. }
                | PatternKind::VariantRename { .. }
                | PatternKind::AssocRename { .. })
        ),
        Hunk::Ordinary(_) => false,
    });
    // WI-16: a bare `# symbol` hunk verifies against the crate graph,
    // which needs the same workspace search as follow/orphans.
    let patch_has_symbol = crate::plan_symbol::patch_has_symbol(patch);
    let mut states: HashMap<String, FileState> = HashMap::new();
    // Files that look like Rust but do not parse: the follow step applies
    // the `raw_mentions` tripwire (veto/skip when the text names `old`).
    let mut unparseable: Vec<(String, String)> = Vec::new();
    for section in &patch.files {
        let path = section.target_path.clone();
        if states.contains_key(&path) {
            continue;
        }
        let original = files.get(&path).cloned().ok_or_else(|| {
            StructuredError::new(
                ErrorCode::ParseConflict,
                format!("target file not found in tree: {path}"),
            )
            .with_file(&path)
            .with_repair(StructuredError::fix_pattern_repair(
                "paths",
                "fix the `diff --git a/.. b/..` paths to name an existing tree file (new files need an empty entry in the tree)",
            ))
        })?;
        let index = LineIndex::new(&original);
        let has_semantic = patch
            .files
            .iter()
            .filter(|s| s.target_path == path)
            .any(|s| s.hunks.iter().any(|h| matches!(h, Hunk::Rsmpl(_))));
        let parsed: Option<syn::File> = if has_semantic {
            Some(syn::parse_str(&original).map_err(|e| {
                StructuredError::new(
                    ErrorCode::ParseConflict,
                    format!("target file does not parse as Rust: {e}"),
                )
                .with_file(&path)
                .with_repair(StructuredError::fix_pattern_repair(
                    "syntax",
                    "fix the Rust syntax error in the target file first (ordinary hunks bypass the Rust parse)",
                ))
            })?)
        } else {
            None
        };
        states.insert(
            path,
            FileState {
                original,
                index,
                parsed,
            },
        );
    }
    if patch_has_follow || patch_has_rename || patch_has_symbol {
        // Workspace file set: every other loaded `.rs` file is searchable.
        // Parsed files join `states` (empty edit lists below); unparseable
        // ones are recorded for the follow-step tripwire. Non-`.rs` files
        // are never searched (SPEC 14.8). Sorted for deterministic reports.
        // WI-7c: plain renames load the set for the orphan index too.
        let mut extra: Vec<String> = files
            .keys()
            .filter(|p| !states.contains_key(*p) && p.ends_with(".rs"))
            .cloned()
            .collect();
        extra.sort();
        for path in extra {
            let original = files.get(&path).cloned().unwrap_or_default();
            match syn::parse_str::<syn::File>(&original) {
                Ok(parsed) => {
                    states.insert(
                        path.clone(),
                        FileState {
                            index: LineIndex::new(&original),
                            original,
                            parsed: Some(parsed),
                        },
                    );
                }
                Err(_) => unparseable.push((path, original)),
            }
        }
    }

    // WI-16: crate graph over the pre-image snapshot (only when a
    // bare `# symbol` hunk needs verification; zero cost otherwise).
    let symbol_graph: Option<crate::CrateGraph> =
        patch_has_symbol.then(|| crate::plan_symbol::build_symbol_graph(&states));

    // T348: every section shares original coordinates — validate all
    // sections per path, not just the first (duplicate-path sections
    // skipped bounds checks and surfaced OOB as GIT_HUNK_CONFLICT).
    for section in &patch.files {
        let path = &section.target_path;
        let state = states.get(path).expect("file state");
        crate::validate::ensure_file_range(&state.index, section, path)?;
    }

    let mut captures = CaptureStore::default();
    // Per-file edit lists, built across pass 1 (cuts) and pass 2 (rest).
    let mut file_edits: HashMap<String, Vec<TextEdit>> = HashMap::new();
    // WI-1.2: materialized per-file edits (post-coalesce), retained so
    // the plus-instantiation postcondition can translate reported
    // original-source spans to post-image coordinates.
    let mut full_edits: HashMap<String, Vec<TextEdit>> = HashMap::new();
    // T249: same-anchor capture-paste edit indices per file, in document
    // order. Used after pass 2 to join multi-pastes with one blank line.
    let mut paste_marks: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for path in states.keys() {
        file_edits.insert(path.clone(), Vec::new());
        paste_marks.insert(path.clone(), Vec::new());
    }
    // Rule reports staged with their document position (T45): pass 1
    // evaluates cuts first, but the report must read in patch document
    // order (section order, then hunk order), so entries are sorted
    // after pass 2 before being committed to `report.rules`.
    let mut rule_order: Vec<((usize, usize), RuleReport)> = Vec::new();
    // WI-1b: rewrite hunks awaiting the plus-instantiation check, run
    // after pass-3 materialization against the post-images.
    type ProbeJob = (
        (usize, usize),
        rsmpl_patch::RsmplHunk,
        rsmpl_pattern::CompiledPattern,
        usize,
        Vec<HashMap<String, String>>,
    );
    let mut probe_pending: Vec<ProbeJob> = Vec::new();
    // WI-6: follow-only entries awaiting the span-local ident probe (new
    // ident at each rewritten use span). Verified post-materialization;
    // failures are loud, abstentions stay `reparse-only`. (Type lives in
    // `postcheck_follow` so `plan.rs` stays under the file-size gate.)
    let mut follow_probe_pending: Vec<crate::postcheck_follow::FollowJob> = Vec::new();
    // WI-6b: same-file `follow` bytes per (rule, file); map type + helpers
    // live in `postcheck_follow` so this file stays under the size gate.
    let mut def_follow_pending: crate::postcheck_follow::DefFollowMap = HashMap::new();
    // WI-7a: scalar cuts + bare pastes awaiting the move-pair probe (new
    // module `postcheck_move` so this file stays under the size gate).
    let mut move_cuts: Vec<crate::postcheck_move::MoveCut> = Vec::new();
    let mut move_pastes: Vec<crate::postcheck_move::MovePaste> = Vec::new();

    // ---- Pass 1: `# bind=` cut hunks (v0.1: item captures only). ----
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
                    &mut captures,
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
                    &mut captures,
                    file_edits.get_mut(path).expect("file state"),
                )?;
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

    // ---- Pass 2: every other hunk in document order. ----
    // WI-6b1: borrows stay scoped per hunk so the workspace follow step can
    // take `&states` + `&mut file_edits` with nothing live.
    for (sec_idx, section) in patch.files.iter().enumerate() {
        let path = &section.target_path;
        if !states.contains_key(path) {
            continue;
        }
        for (hunk_idx, hunk) in section.hunks.iter().enumerate() {
            match hunk {
                Hunk::Ordinary(oh) => {
                    let mut e = {
                        let state = states.get(path).expect("file state");
                        ordinary_hunk_edits(&state.index, &state.original, oh, path)
                            .map_err(|s| s.with_file(path))?
                    };
                    file_edits.get_mut(path).expect("file state").append(&mut e);
                }
                Hunk::Rsmpl(rh) => {
                    if !rh.binds().is_empty() {
                        continue; // handled in pass 1
                    }
                    let compiled = rsmpl_pattern::compile(rh)
                        .map_err(|e| e.with_rule(rh.rule_name()).with_file(path))?;
                    gate_directives(rh, &compiled, path)?;
                    if !compiled.unbound_plus.is_empty() {
                        return Err(unbound_plus(rh, &compiled.unbound_plus, path));
                    }
                    let rule = rh.rule_name();
                    let range_desc = format!(
                        "{}..{}",
                        rh.header.old_start,
                        rh.header.old_start + rh.header.old_len
                    );
                    let edits_before = file_edits.get(path).map(|v| v.len()).unwrap_or(0);
                    let (mut found, mut def_bindings) = {
                        let state = states.get(path).expect("file state");
                        let edits = file_edits.get_mut(path).expect("file state");
                        // Definition matching only; follow runs below.
                        apply_rsmpl_hunk(
                            state,
                            path,
                            rh,
                            &compiled,
                            &captures,
                            edits,
                            symbol_graph.as_ref(),
                        )
                        .map_err(|e| e.with_file(path))?
                    };
                    // WI-10b drift fallback (SPEC 10.8): a stale cited range
                    // retries once file-scoped, anchored by the hunk's body
                    // context lines. Exactly one anchored candidate splices
                    // its edits/bindings into the normal path below (spans,
                    // sites, follow, postconditions all run unchanged);
                    // anything else keeps the original zero-match error.
                    if found == 0 && crate::drift::is_drift_eligible(rh, &compiled) {
                        let state = states.get(path).expect("file state");
                        if let Some(outcome) = crate::drift::try_drift_apply(
                            state,
                            path,
                            rh,
                            &compiled,
                            &captures,
                            symbol_graph.as_ref(),
                        ) {
                            let edits = file_edits.get_mut(path).expect("file state");
                            edits.extend(outcome.edits);
                            found = 1;
                            def_bindings = outcome.bindings;
                            report.drifts.push(crate::DriftReport {
                                rule: rule.clone(),
                                file: path.clone(),
                                cited: (
                                    rh.header.old_start,
                                    rh.header.old_start + rh.header.old_len.saturating_sub(1),
                                ),
                                actual: outcome.actual,
                            });
                        }
                    }
                    // WI-2c sweep change gate (SPEC 24.6): a file-scope
                    // `# allow=many` hunk that matched but changes no bytes
                    // (identical replacements) fails loud here, before
                    // follow extras land. Ranged/optional/insertion/cut
                    // hunks never owe it (see `postcheck`).
                    let def_len = file_edits
                        .get(path)
                        .map(|v| v.len())
                        .unwrap_or(edits_before);
                    if found > 0 && sweep_hunk_needs_postcheck(rh, &compiled) {
                        let state = states.get(path).expect("file state");
                        let edits = file_edits.get(path).expect("file state");
                        if sweep_edits_are_noop(&state.original, &edits[edits_before..def_len]) {
                            return Err(noop_sweep_error(&rule, path, &range_desc));
                        }
                    }
                    // T249: record capture-paste edits for blank-join. A
                    // paste hunk pushes exactly one insertion at
                    // `edits_before`, so its index is stable.
                    // WI-7a: bare paste (`+ N` with `# use=N`, template trims
                    // to the name) records a move probe; evolving templates
                    // abstain (WI-7c).
                    let paste_bare_name: Option<String> = match &compiled.kind {
                        PatternKind::Insertion {
                            text,
                            use_name: Some(n),
                        } if text.trim() == n.as_str() => Some(n.clone()),
                        _ => None,
                    };
                    if matches!(
                        &compiled.kind,
                        PatternKind::Insertion {
                            use_name: Some(_),
                            ..
                        }
                    ) && found == 1
                        && file_edits.get(path).map(|v| v.len()).unwrap_or(0) == edits_before + 1
                    {
                        let pos = file_edits.get(path).expect("file state")[edits_before].start;
                        debug_assert_eq!(
                            file_edits.get(path).expect("file state")[edits_before].end,
                            pos
                        );
                        if let Some(marks) = paste_marks.get_mut(path) {
                            marks.push((pos, edits_before));
                        }
                        if let Some(name) = paste_bare_name {
                            move_pastes.push(crate::postcheck_move::MovePaste {
                                rule: rule.clone(),
                                file: path.clone(),
                                name,
                                pos,
                                bare: true,
                            });
                        }
                    }
                    // WI-6b1 workspace follow before spans: def-file spans
                    // cover definition + same-file uses (SPEC 14.8), and
                    // other files' uses land in their own edit lists.
                    let (extra_reports, follow_details) = if rh.follows_uses() {
                        follow_uses_workspace(
                            rh,
                            &compiled,
                            &states,
                            &mut file_edits,
                            &unparseable,
                            path,
                            edits_before,
                            found,
                        )?
                    } else {
                        (Vec::new(), Vec::new())
                    };
                    let (spans, span_labels) = {
                        let state = states.get(path).expect("file state");
                        let edits = file_edits.get(path).expect("file state");
                        let new_edits = &edits[edits_before..];
                        (
                            edit_spans(&state.index, new_edits),
                            crate::edits::edit_span_labels(new_edits),
                        )
                    };
                    // WI-6b: record `follow` bytes for the byte-exact probe.
                    {
                        let edits = file_edits.get(path).expect("file state");
                        crate::postcheck_follow::record_def_follow(
                            &mut def_follow_pending,
                            &rule,
                            path,
                            &edits[edits_before..],
                        );
                    }
                    // A non-insertion hunk carrying `# use=` has no v0.1 meaning.
                    if !rh.uses().is_empty()
                        && !matches!(
                            &compiled.kind,
                            PatternKind::Insertion {
                                use_name: Some(_),
                                ..
                            }
                        )
                    {
                        return Err(StructuredError::new(
                            ErrorCode::CaptureConflict,
                            "v0.1 `# use=` is only supported on pure insertion hunks (`+ Name`)",
                        )
                        .with_rule(rule.clone())
                        .with_file(path)
                        .with_repair(StructuredError::fix_pattern_repair(
                            "use",
                            "paste with a pure insertion hunk (`+ Name` with `# use=<Name>`)",
                        )));
                    }
                    // T62: on a hard zero-match, probe the same pattern
                    // whole-file (scope overridden, count-only, edits
                    // discarded) so an unambiguous failure can suggest the
                    // zero-discovery `# scope=file` retry. Insertions use
                    // anchors rather than pattern search and are excluded
                    // by the probe itself.
                    let wide = if found == 0 && !rh.is_optional() && rh.scope() != Scope::File {
                        let state = states.get(path).expect("file state");
                        crate::drift::probe_whole_file(
                            state,
                            path,
                            rh,
                            &compiled,
                            &captures,
                            symbol_graph.as_ref(),
                        )
                    } else {
                        None
                    };
                    // T67: diagnostic-only excerpt; happy/optional paths pay nothing.
                    let preview = if found == 0 && !rh.is_optional() {
                        let state = states.get(path).expect("file state");
                        candidate_preview(state, rh, &compiled)
                    } else {
                        None
                    };
                    ensure_match_count(
                        found,
                        rh.allow(),
                        rh.is_optional(),
                        &rule,
                        path,
                        &range_desc,
                        ZeroMatchHint { wide, preview },
                    )?;
                    // WI-2b/2c: per-site bindings in span order (v2 evidence).
                    // `def_bindings` is one map per accepted match.
                    let mut sites: Vec<crate::SiteReport> = Vec::new();
                    for (i, span) in spans.iter().enumerate() {
                        if let Some(m) = def_bindings.get(i) {
                            if m.is_empty() {
                                continue;
                            }
                            let mut b: Vec<(String, String)> =
                                m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                            b.sort_by(|a, b| a.0.cmp(&b.0));
                            sites.push(crate::SiteReport {
                                span: *span,
                                bindings: b,
                            });
                        }
                    }
                    // WI-7c: plain renames report the uses they leave behind.
                    // Follow hunks never carry orphans (uses rewrite there).
                    // WI-28: a definition-only assoc hunk whose intent
                    // sibling (same rule, any file) carries the follow
                    // reports no orphans of its own — the follow hunk
                    // owns the file-wide use set, and per-hunk orphans
                    // would duplicate sites it already rewrote.
                    let sibling_follow = matches!(
                        &compiled.kind,
                        PatternKind::AssocRename { .. } | PatternKind::ImplContext { .. }
                    ) && patch.files.iter().any(|sec| {
                        sec.hunks.iter().any(|h| match h {
                            Hunk::Rsmpl(srh) => srh.rule_name() == rule && srh.follows_uses(),
                            _ => false,
                        })
                    });
                    let orphans = if !rh.follows_uses() && !sibling_follow && found > 0 {
                        match &compiled.kind {
                            PatternKind::Fn { minus_name, .. } => {
                                // WI-2d: a same-name header with one renamed
                                // param owns fn-body orphans, not fn-name uses.
                                if let Some((old, _)) =
                                    crate::use_graph_binder::fn_param_binder_rename_from_hunk(rh)
                                {
                                    crate::use_graph_scope::fn_param_orphans(
                                        &states,
                                        &file_edits,
                                        path,
                                        edits_before,
                                        &old,
                                    )
                                } else {
                                    let renamed: Vec<(usize, usize)> = file_edits
                                        .get(path)
                                        .map(|v| {
                                            v[edits_before..]
                                                .iter()
                                                .map(|e| (e.start, e.end))
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> =
                                        Vec::new();
                                    let mut paths: Vec<&String> = states.keys().collect();
                                    paths.sort();
                                    for p in paths {
                                        if let Some(st) = states.get(p) {
                                            if let Some(parsed) = st.parsed.as_ref() {
                                                refs.push((
                                                    p.clone(),
                                                    parsed,
                                                    st.original.as_str(),
                                                    &st.index,
                                                ));
                                            }
                                        }
                                    }
                                    collect_orphans(&refs, minus_name, path, &renamed, false)
                                }
                            }
                            PatternKind::FieldRename {
                                struct_name, old, ..
                            } => crate::use_graph_member::member_orphans(
                                &states,
                                &file_edits,
                                path,
                                edits_before,
                                crate::use_graph_member::MemberKind::Field,
                                struct_name,
                                old,
                            ),
                            PatternKind::VariantRename { enum_name, old, .. } => {
                                crate::use_graph_member::member_orphans(
                                    &states,
                                    &file_edits,
                                    path,
                                    edits_before,
                                    crate::use_graph_member::MemberKind::Variant,
                                    enum_name,
                                    old,
                                )
                            }
                            PatternKind::AssocRename {
                                impl_header,
                                want_trait,
                                is_type,
                                minus_name,
                                ..
                            } => crate::use_graph_assoc::assoc_orphans(
                                &states,
                                &file_edits,
                                path,
                                edits_before,
                                impl_header,
                                *want_trait,
                                *is_type,
                                minus_name,
                            ),
                            PatternKind::AssocEvolution { .. } => Vec::new(),
                            PatternKind::ImplContext { .. } => {
                                crate::use_graph_method::method_orphans_for(
                                    &states,
                                    &file_edits,
                                    path,
                                    edits_before,
                                    rh,
                                    &compiled,
                                )
                            }
                            PatternKind::ItemRename {
                                minus_name, kind, ..
                            } => {
                                let renamed: Vec<(usize, usize)> = file_edits
                                    .get(path)
                                    .map(|v| {
                                        v[edits_before..].iter().map(|e| (e.start, e.end)).collect()
                                    })
                                    .unwrap_or_default();
                                let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> =
                                    Vec::new();
                                let mut paths: Vec<&String> = states.keys().collect();
                                paths.sort();
                                for p in paths {
                                    if let Some(st) = states.get(p) {
                                        if let Some(parsed) = st.parsed.as_ref() {
                                            refs.push((
                                                p.clone(),
                                                parsed,
                                                st.original.as_str(),
                                                &st.index,
                                            ));
                                        }
                                    }
                                }
                                collect_orphans(
                                    &refs,
                                    minus_name,
                                    path,
                                    &renamed,
                                    *kind == rsmpl_pattern::ItemKind::MacroRules,
                                )
                            }
                            _ => crate::use_graph_local::general_orphans(
                                &states,
                                &file_edits,
                                path,
                                edits_before,
                                &compiled,
                            ),
                        }
                    } else {
                        Vec::new()
                    };
                    // WI-1b provisional semantics (confirmed by the
                    // post-materialization instantiation check below).
                    // Optional zero-edit: not-applicable. Non-rewrites
                    // (insert/cut/raw): reparse-only. Rewrites: instantiated
                    // or postcondition (sweep gate ran).
                    let sweep_gate_ran = found > 0 && sweep_hunk_needs_postcheck(rh, &compiled);
                    let (provisional, needs_probe) = if found == 0 {
                        (RuleSemantics::NotApplicable, false)
                    } else if !is_rewrite_kind(&compiled)
                        || (!orphans.is_empty()
                            && (crate::use_graph_local::is_local_binder_rename(&compiled)
                                || crate::use_graph_binder::fn_param_binder_rename_from_hunk(rh)
                                    .is_some()))
                    {
                        (RuleSemantics::ReparseOnly, false)
                    } else if sweep_gate_ran {
                        (RuleSemantics::Postcondition, true)
                    } else {
                        (RuleSemantics::Instantiated, true)
                    };
                    if needs_probe {
                        probe_pending.push((
                            (sec_idx, hunk_idx),
                            rh.clone(),
                            compiled.clone(),
                            found,
                            def_bindings.clone(),
                        ));
                    }
                    rule_order.push((
                        (sec_idx, hunk_idx),
                        RuleReport {
                            rule: rule.clone(),
                            file: path.clone(),
                            matches: found,
                            spans,
                            span_labels,
                            sites,
                            follow_skipped_details: follow_details,
                            orphans,
                            semantics: provisional,
                            symbol: crate::plan_symbol::symbol_mode(rh, &compiled),
                        },
                    ));
                    // WI-6b1: one follow-only entry per non-definition file
                    // touched (same rule name, `matches` = use-site count,
                    // `spans.len() == matches`). Shared position key keeps
                    // them adjacent to the definition entry in document
                    // order (the sort below is stable); file order inside
                    // is sorted at the source.
                    // WI-6: follow-only extras verify at their spans (SPEC
                    // 24.6 ident probe), so they start `reparse-only` and
                    // the post-materialization probe upgrades or fails
                    // loud. Labels ride along: use sites `follow`, WI-1
                    // impl-definition claims `rename`.
                    for (file, matches, spans, labels, byte_edits) in extra_reports {
                        let span_labels = labels.into_iter().map(Some).collect();
                        let method_new = match &compiled.kind {
                            PatternKind::ImplContext { .. } => {
                                crate::use_graph_method::method_follow_new_name(rh, &compiled)
                            }
                            _ => None,
                        };
                        follow_probe_pending.push((
                            (sec_idx, hunk_idx),
                            rule.clone(),
                            compiled.clone(),
                            file.clone(),
                            spans.clone(),
                            method_new,
                            byte_edits,
                        ));
                        rule_order.push((
                            (sec_idx, hunk_idx),
                            RuleReport {
                                rule: rule.clone(),
                                file,
                                matches,
                                spans,
                                span_labels,
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
        }
    }

    // T249: join same-anchor multi-pastes with one blank line. Each paste
    // pushes `captured.trim_end() + "\n"` (no separators by construction,
    // since the cut absorbs the blank — SPEC 10.5). A lone paste is left
    // alone (surrounding context unchanged); for N>1 pastes at the same
    // byte position, every paste but the last in document order gains one
    // extra "\n" so the document-order materialization (SPEC 23.5) yields
    // exactly one blank line between items. Ordinary literal insertions
    // are untouched — the agent controls their blanks explicitly.
    for (path, marks) in &paste_marks {
        if marks.len() < 2 {
            continue;
        }
        let mut by_pos: HashMap<usize, Vec<usize>> = HashMap::new();
        for (pos, idx) in marks {
            by_pos.entry(*pos).or_default().push(*idx);
        }
        if let Some(edits) = file_edits.get_mut(path) {
            for indices in by_pos.values() {
                if indices.len() < 2 {
                    continue;
                }
                let mut ordered = indices.clone();
                ordered.sort_unstable();
                // All but the last in document order get the separator.
                for idx in &ordered[..ordered.len() - 1] {
                    let e = &mut edits[*idx];
                    debug_assert!(e.is_insertion());
                    if !e.replacement.ends_with("\n\n") {
                        e.replacement.push('\n');
                    }
                }
            }
        }
    }

    // Commit rule reports in patch document order (T36/T45 contract:
    // section order, then hunk order within a section).
    rule_order.sort_by_key(|(pos, _)| *pos);
    probe_pending.sort_by_key(|(pos, _, _, _, _)| *pos);
    follow_probe_pending.sort_by_key(|(pos, _, _, _, _, _, _)| *pos);
    report.rules.extend(rule_order.into_iter().map(|(_, r)| r));

    // ---- Pass 3: overlap check, materialize, reparse-validate. ----
    // T137: ordinary-only files (`parsed.is_none()`) skip the Rust
    // reparse — they are plain text, not Rust.
    for (path, state) in &states {
        let edits = file_edits.remove(path).unwrap_or_default();
        if edits.is_empty() {
            continue;
        }
        // T464: overlapping pure deletions union-merge (adjacent item
        // deletes double-claim their shared separator blank); every
        // other overlap pair still fails in `check_overlaps` below.
        // WI-1: identical rewrites of the same span (a follow definition
        // claim + a sibling definition-only hunk owning the same impl
        // item, same replacement) collapse to one edit — same bytes,
        // same outcome, never a conflict.
        let edits = dedupe_identical_edits(edits);
        let edits = coalesce_deletes(edits);
        check_overlaps(&edits, path)?;
        full_edits.insert(path.clone(), edits.clone());
        // WI-2c: materialization is a pure function of original + the
        // recorded edits (byte-identity outside spans holds by
        // construction, SPEC 24.6); the trust gates are reparse +
        // postcondition below, both of which fail loud on real input.
        let updated = apply_edits(&state.original, edits).map_err(|e| e.with_file(path))?;
        if state.parsed.is_some() {
            syn::parse_str::<syn::File>(&updated).map_err(|e| {
                StructuredError::new(
                    ErrorCode::ParseConflict,
                    format!("rewritten file does not parse as Rust: {e}"),
                )
                .with_file(path)
                .with_repair(StructuredError::fix_pattern_repair(
                    "syntax",
                    "adjust the plus pattern so the rewritten file parses as Rust (see reason)",
                ))
            })?;
        }
        files.insert(path.clone(), updated);
    }

    // WI-1b/WI-1.5: plus-pattern postcondition (SPEC 24.6).
    // Every rewrite hunk's plus template, compiled to a pattern, must
    // re-match in the post-image at the reported spans (translated to
    // post-image coordinates: line-changing rewrites shift later sites).
    // Renames verify the new ident by syn ident equality at overlapping
    // spans (WI-1.4); Expr/General/Macro re-run the existing matchers
    // over a synthetic plus-as-minus hunk with SPEC-15.7 binding
    // equality (WI-1.5); ImplContext verifies `fn {name}` span-locally
    // by ident equality.
    // Occurrence elsewhere never counts. Failures are loud
    // `RSMPL_MATCH_CONFLICT`; unverifiable plus shapes (insertions,
    // cuts, `Raw`, unextractable impl names, plus-side compile/match
    // failures) downgrade to `reparse-only` (honest, never over-claimed).
    // `probe_pending` and the provisional `instantiated|postcondition`
    // entries are both in document order, so zip in order.
    {
        // WI-1.5: parse each touched post-image once plus its line
        // index; the plus-pattern postcondition reuses both instead of
        // re-splitting text per rule (reparse already validated above,
        // so a missing parse only means `reparse-only` inside the check).
        let mut post_parsed: HashMap<String, syn::File> = HashMap::new();
        let mut post_index: HashMap<String, crate::lines::LineIndex> = HashMap::new();
        for (path, text) in files.iter() {
            if let Ok(f) = syn::parse_str::<syn::File>(text) {
                post_parsed.insert(path.clone(), f);
            }
            post_index.insert(path.clone(), crate::lines::LineIndex::new(text));
        }
        let mut jobs = probe_pending.into_iter();
        // WI-6: follow-only entries verify at their spans (helper owns the
        // loop so `plan.rs` stays under the file-size gate).
        let follow_keys = crate::postcheck_follow::upgrade_follow_only_entries(
            &mut report.rules,
            &follow_probe_pending,
            &states,
            &full_edits,
            files,
            &post_parsed,
        )?;
        for r in report.rules.iter_mut() {
            if follow_keys.contains(&(r.rule.clone(), r.file.clone())) {
                continue;
            }
            if r.semantics != RuleSemantics::Instantiated
                && r.semantics != RuleSemantics::Postcondition
            {
                continue;
            }
            let Some((_, rh, compiled, found, bindings_list)) = jobs.next() else {
                // Count mismatch (should not happen): stay honest.
                r.semantics = RuleSemantics::ReparseOnly;
                continue;
            };
            let range_desc = format!(
                "{}..{}",
                rh.header.old_start,
                rh.header.old_start + rh.header.old_len
            );
            let post_text = files.get(&r.file).cloned().unwrap_or_default();
            // WI-1.2: `r.spans` are original-source lines; line-changing
            // rewrites shift later sites, so translate to post-image
            // coordinates before the span-local check. The report keeps
            // original spans (SPEC D.2); only the check moves.
            let post_spans: Vec<(u32, u32)> = states
                .get(&r.file)
                .map(|st| {
                    translate_spans_to_post(
                        &st.original,
                        &st.index,
                        full_edits.get(&r.file).map(Vec::as_slice).unwrap_or(&[]),
                        &r.spans,
                    )
                })
                .unwrap_or_else(|| r.spans.clone());
            // WI-2b: binder renames carry `follow` use-spans alongside the
            // definition; the plus (`let new = …`) only proves the
            // definition spans — follows verify byte-exact below.
            // Translate only the definition originals so indices cannot
            // drift when follows interleave.
            let plus_spans: Vec<(u32, u32)> =
                if crate::use_graph_local::is_let_binder_rename(&compiled) {
                    let def_orig: Vec<(u32, u32)> = r
                        .spans
                        .iter()
                        .zip(r.span_labels.iter())
                        .filter(|(_, l)| l.as_deref() != Some("follow"))
                        .map(|(s, _)| *s)
                        .collect();
                    states
                        .get(&r.file)
                        .map(|st| {
                            translate_spans_to_post(
                                &st.original,
                                &st.index,
                                full_edits.get(&r.file).map(Vec::as_slice).unwrap_or(&[]),
                                &def_orig,
                            )
                        })
                        .unwrap_or(def_orig)
                } else {
                    post_spans.clone()
                };
            match plus_instantiated_parsed(
                &compiled,
                &rh,
                &bindings_list,
                &post_text,
                post_parsed.get(&r.file),
                post_index.get(&r.file),
                &plus_spans,
                r.matches,
            ) {
                Some(true) => {}
                Some(false) => {
                    let _ = found;
                    return Err(instantiation_error(&r.rule, &r.file, &range_desc));
                }
                None => {
                    r.semantics = RuleSemantics::ReparseOnly;
                    continue;
                }
            }
            // WI-6: same-file follow spans verify identically (helper owns
            // the subset probe; `rename` impl-def claims stay under WI-1).
            crate::postcheck_follow::verify_definition_follows_recorded(
                &rh,
                &r.rule,
                &r.file,
                &r.spans,
                &r.span_labels,
                &compiled,
                &states,
                &full_edits,
                &post_text,
                post_parsed.get(&r.file),
                &range_desc,
                &def_follow_pending,
            )?;
        }
        // WI-7a: verified cut+paste pairs earn `postcondition` (helper owns
        // the loop so `plan.rs` stays under the size gate).
        crate::postcheck_move::upgrade_verified_moves(
            &move_cuts,
            &move_pastes,
            &captures,
            &states,
            &full_edits,
            files,
            &post_parsed,
            &mut report.rules,
        )?;
    }

    // WI-2c: the semantic leg beyond reparse is the pass-2 sweep change
    // gate above (fails loud as `RSMPL_MATCH_CONFLICT` on real input,
    // never true by construction). Byte-identity outside spans holds by
    // construction (edits are the only mutation, SPEC 24.6).

    Ok(report)
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
