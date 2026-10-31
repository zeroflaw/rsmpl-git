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

use rsmpl_patch::{Directive, ErrorCode, Hunk, Patch, Scope, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::capture::{paste_capture, CaptureStore, PasteTarget};
use crate::edits::{
    apply_edits, check_overlaps, coalesce_deletes, edit_spans, translate_spans_to_post, TextEdit,
};
use crate::lines::{FileState, LineIndex};
use crate::postcheck::{
    instantiation_error, is_rewrite_kind, noop_sweep_error, plus_instantiated_parsed,
    sweep_edits_are_noop, sweep_hunk_needs_postcheck,
};
use crate::suggest::candidate_preview;
use crate::use_graph::follow_uses_workspace;
use crate::use_graph_orphan::collect_orphans;
use crate::validate::{ensure_match_count, gate_directives, unbound_plus, ZeroMatchHint};
use crate::{
    matcher_expr, matcher_fn, matcher_general_apply, matcher_impl, matcher_item, matcher_macro,
    matcher_member, matcher_use,
};
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
                | PatternKind::VariantRename { .. })
        ),
        Hunk::Ordinary(_) => false,
    });
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
    if patch_has_follow || patch_has_rename {
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
                rule_order.push((
                    (sec_idx, hunk_idx),
                    RuleReport {
                        rule: rh.rule_name(),
                        file: path.clone(),
                        matches: 1,
                        spans: vec![(sl, el)],
                        span_labels: Vec::new(),
                        sites: Vec::new(),
                        follow_skipped: None,
                        follow_skipped_details: Vec::new(),
                        orphans: Vec::new(),
                        semantics: RuleSemantics::ReparseOnly,
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
                        apply_rsmpl_hunk(state, path, rh, &compiled, &captures, edits)
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
                        if let Some(outcome) =
                            try_drift_apply(state, path, rh, &compiled, &captures)
                        {
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
                    }
                    // WI-6b1 workspace follow before spans: def-file spans
                    // cover definition + same-file uses (SPEC 14.8), and
                    // other files' uses land in their own edit lists.
                    let (follow_bindings, extra_reports, follow_details) = if rh.follows_uses() {
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
                        (HashMap::new(), Vec::new(), Vec::new())
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
                        probe_whole_file(state, path, rh, &compiled, &captures)
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
                    // `def_bindings` is one map per accepted match;
                    // `follow_skipped` stays hunk-level (no span).
                    let follow_skipped = follow_bindings.get("follow_skipped").cloned();
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
                    let orphans = if !rh.follows_uses() && found > 0 {
                        match &compiled.kind {
                            PatternKind::Fn { minus_name, .. } => {
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
                                collect_orphans(&refs, minus_name, path, &renamed, false)
                            }
                            PatternKind::FieldRename {
                                struct_name, old, ..
                            } => member_orphans(
                                &states,
                                &file_edits,
                                path,
                                edits_before,
                                crate::use_graph_member::MemberKind::Field,
                                struct_name,
                                old,
                            ),
                            PatternKind::VariantRename { enum_name, old, .. } => member_orphans(
                                &states,
                                &file_edits,
                                path,
                                edits_before,
                                crate::use_graph_member::MemberKind::Variant,
                                enum_name,
                                old,
                            ),
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
                            _ => Vec::new(),
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
                    } else if !is_rewrite_kind(&compiled) {
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
                            follow_skipped,
                            follow_skipped_details: follow_details,
                            orphans,
                            semantics: provisional,
                        },
                    ));
                    // WI-6b1: one follow-only entry per non-definition file
                    // touched (same rule name, `matches` = use-site count,
                    // `spans.len() == matches`). Shared position key keeps
                    // them adjacent to the definition entry in document
                    // order (the sort below is stable); file order inside
                    // is sorted at the source.
                    // WI-1b: follow-only extras are conservatively
                    // reparse-only (the definition probe covers the rename;
                    // per-use plus shapes differ). Every extra span is a
                    // rewritten use site: WI-2 `follow` label throughout.
                    for (file, matches, spans) in extra_reports {
                        let span_labels = vec![Some("follow".to_string()); spans.len()];
                        rule_order.push((
                            (sec_idx, hunk_idx),
                            RuleReport {
                                rule: rule.clone(),
                                file,
                                matches,
                                spans,
                                span_labels,
                                sites: Vec::new(),
                                follow_skipped: None,
                                follow_skipped_details: Vec::new(),
                                orphans: Vec::new(),
                                semantics: RuleSemantics::ReparseOnly,
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
        for r in report.rules.iter_mut() {
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
            match plus_instantiated_parsed(
                &compiled,
                &rh,
                &bindings_list,
                &post_text,
                post_parsed.get(&r.file),
                post_index.get(&r.file),
                &post_spans,
                r.matches,
            ) {
                Some(true) => {}
                Some(false) => {
                    let _ = found;
                    return Err(instantiation_error(&r.rule, &r.file, &range_desc));
                }
                None => {
                    r.semantics = RuleSemantics::ReparseOnly;
                }
            }
        }
    }

    // WI-2c: the semantic leg beyond reparse is the pass-2 sweep change
    // gate above (fails loud as `RSMPL_MATCH_CONFLICT` on real input,
    // never true by construction). Byte-identity outside spans holds by
    // construction (edits are the only mutation, SPEC 24.6).

    Ok(report)
}

/// Dispatch one pass-2 RSmPL hunk to its matcher (SPEC 23.1 step 7).
///
/// Extracted from the pass-2 loop so the T62 whole-file probe reuses the
/// exact same dispatch (no predicate duplication, no drift). File context
/// (`with_file`) is applied by the caller, not here, so real and probe
/// runs share error shapes.
/// WI-9a: orphan evidence for definition-only member renames (SPEC
/// 14.8). Mirrors the scalar orphan block: the definition hunk's own
/// ident-span edits plus every loaded `.rs` file feed the member
/// collector.
#[allow(clippy::too_many_arguments)]
fn member_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    kind: crate::use_graph_member::MemberKind,
    container: &str,
    old: &str,
) -> Vec<crate::OrphanReport> {
    let renamed: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> = Vec::new();
    let mut paths: Vec<&String> = states.keys().collect();
    paths.sort();
    for p in paths {
        if let Some(st) = states.get(p) {
            if let Some(parsed) = st.parsed.as_ref() {
                refs.push((p.clone(), parsed, st.original.as_str(), &st.index));
            }
        }
    }
    crate::use_graph_member::collect_member_orphans(&refs, kind, container, old, path, &renamed)
}

fn apply_rsmpl_hunk(
    state: &FileState,
    path: &str,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    captures: &CaptureStore,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let rule = rh.rule_name();
    let parsed = state.parsed.as_ref().expect(
        "T137: RSmPL hunk on a file with no parsed AST (ordinary-only files carry no semantic hunks)",
    );
    // WI-2b: shapes without metavariables report empty per-site maps; call /
    // general-expr / macro shapes thread their per-site maps through.
    let empty = || HashMap::new();
    match &compiled.kind {
        // WI-6b1: definition matching only — `# follow=uses` runs as a
        // workspace step in the pass-2 loop (needs every file's state).
        PatternKind::Fn { .. } => matcher_fn::apply_fn_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::Expr {
            minus: rsmpl_pattern::ExprPatternKind::Macro { .. },
            ..
        } => matcher_macro::apply_macro_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Expr { .. } => matcher_expr::apply_expr_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ItemRename { .. } => matcher_item::apply_item_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::FieldRename { .. } => matcher_member::apply_field_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::VariantRename { .. } => matcher_member::apply_variant_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::UseRename { .. } => matcher_use::apply_use_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::General { .. } => matcher_general_apply::apply_general_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ItemCapture { .. } => matcher_fn::apply_item_delete(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::Insertion { text, use_name } => match use_name {
            Some(name) => {
                paste_capture(
                    captures,
                    PasteTarget {
                        name,
                        template: Some(text),
                        anchor: rh.header.old_start,
                    },
                    &state.index,
                    &rule,
                    path,
                    edits,
                )?;
                Ok((1, vec![empty()]))
            }
            None => {
                matcher_fn::apply_insertion(&state.original, &state.index, rh, compiled, edits)
                    .map(|n| (n, vec![empty(); n]))
            }
        },
        PatternKind::ImplContext { .. } => matcher_impl::apply_impl_context_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::Macro { .. } => matcher_macro::apply_macro_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Raw { .. } => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "unsupported pattern shape in v0.1 (only fn rename, struct/enum/trait/union/type/const/static/mod/impl item rename, `use`-path prefix rename, call rewrite, let-stmt/general-expr/type rewrite, item delete/insert, macro path rewrite)",
        )
        .with_rule(rule)),
    }
}

/// WI-10b drift retry (SPEC 10.8): re-run the same matcher with scope
/// overridden to `File`, keeping the edits. Succeeds only when the
/// file-wide run yields exactly one match whose span the hunk's body
/// context anchors satisfy (leading/trailing ` ` lines in order, see
/// `drift`). Probe errors, zero matches, multiple matches, missing
/// anchors, or an unsatisfied candidate all yield `None` — the caller
/// keeps the original zero-match error (never a too-many or a guess).
fn try_drift_apply(
    state: &FileState,
    path: &str,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    captures: &CaptureStore,
) -> Option<crate::drift::DriftOutcome> {
    let (leading, trailing) = crate::drift::drift_anchors(rh);
    if leading.is_empty() && trailing.is_empty() {
        return None;
    }
    let mut wide_hunk = rh.clone();
    wide_hunk.directives.push(Directive::Scope(Scope::File));
    let mut probe_edits = Vec::new();
    let (n, bindings) = apply_rsmpl_hunk(
        state,
        path,
        &wide_hunk,
        compiled,
        captures,
        &mut probe_edits,
    )
    .ok()?;
    if n != 1 {
        return None;
    }
    let spans = edit_spans(&state.index, &probe_edits);
    let actual = crate::drift::bounding_box(&spans)?;
    // SPEC 10.8: the stale range must share no line with the drifted
    // item — an overlapping range is a scope/filter statement about
    // that item (SPEC 17.3), not line drift.
    let cited = (
        rh.header.old_start,
        rh.header.old_start + rh.header.old_len.saturating_sub(1),
    );
    let region = state
        .parsed
        .as_ref()
        .and_then(|p| crate::drift::enclosing_top_item_span(p, actual))
        .unwrap_or(actual);
    if !crate::drift::cited_is_disjoint(cited, region) {
        return None;
    }
    let file_lines: Vec<&str> = state.original.lines().collect();
    if !crate::drift::anchors_satisfied(&file_lines, actual, &leading, &trailing) {
        return None;
    }
    Some(crate::drift::DriftOutcome {
        edits: probe_edits,
        bindings,
        actual,
    })
}

/// T62 whole-file probe: re-run the same matcher with scope overridden to
/// `File` and count matches without keeping edits.
///
/// Returns `None` when the probe itself errors (keep the original error's
/// generic hint) or the hunk kind is not probe-eligible: insertions use
/// anchors rather than pattern search, and `Raw` shapes never reach here
/// (rejected before dispatch). The probe fires only on the zero-match
/// path, so the happy path pays no extra matching cost.
fn probe_whole_file(
    state: &FileState,
    path: &str,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    captures: &CaptureStore,
) -> Option<usize> {
    if matches!(
        &compiled.kind,
        PatternKind::Insertion { .. } | PatternKind::Raw { .. }
    ) {
        return None;
    }
    let mut wide_hunk = rh.clone();
    wide_hunk.directives.push(Directive::Scope(Scope::File));
    let mut probe_edits = Vec::new();
    apply_rsmpl_hunk(
        state,
        path,
        &wide_hunk,
        compiled,
        captures,
        &mut probe_edits,
    )
    .ok()
    .map(|(n, _)| n)
}

pub(crate) fn range_err(rule: &str, file: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_file(file)
    .with_repair(StructuredError::internal_repair())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_multi(
        patch_text: &str,
        initial: Vec<(&str, &str)>,
    ) -> Result<HashMap<String, String>, String> {
        apply_multi_report(patch_text, initial).map(|(files, _)| files)
    }

    fn apply_multi_report(
        patch_text: &str,
        initial: Vec<(&str, &str)>,
    ) -> Result<(HashMap<String, String>, crate::ApplyReport), String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files: HashMap<String, String> = initial
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let report = apply_with_captures(&mut files, &patch).map_err(|e| e.render())?;
        Ok((files, report))
    }

    const UPWARD_MOVE: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +2,0 @@ rule=paste_helper_near_top\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -6,4 +6,0 @@ rule=cut_helper_from_bottom\n# lang=rust\n# item I\n# bind=Helper\n- I\n";

    #[test]
    fn upward_move_with_forward_ref() {
        // Paste hunk (anchor after line 2) precedes the cut hunk in document
        // order; the engine must still resolve `Helper` (SPEC 23.3).
        let src = "fn top() {}\nfn mid() {}\nfn keep() {}\nfn other() {}\nfn tail() {}\nfn helper(x: i32) -> i32 {\n    x + 1\n}\n";
        // helper spans lines 6-8; cut range 6,4 exceeds (8 lines: 6+4-1=9 > 8). Fix range below.
        let patch = UPWARD_MOVE.replace("@@ -6,4 +6,0 @@", "@@ -6,3 +6,0 @@");
        let files = apply_multi(&patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        // helper moved to after line 2.
        let helper_pos = out.find("fn helper").unwrap();
        let top_pos = out.find("fn top").unwrap();
        let mid_pos = out.find("fn mid").unwrap();
        let keep_pos = out.find("fn keep").unwrap();
        assert!(
            top_pos < mid_pos && mid_pos < helper_pos && helper_pos < keep_pos,
            "got:\n{out}"
        );
        assert_eq!(out.matches("fn helper").count(), 1);
    }

    #[test]
    fn cross_file_move_with_import_hunk() {
        let patch = "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -1,1 +1,0 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n\ndiff --git a/src/new.rs b/src/new.rs\n--- a/src/new.rs\n+++ b/src/new.rs\n@@ -1,0 +1,0 @@ rule=paste_helper\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -1,1 +1,2 @@\n use crate::common;\n+use crate::old_dependency;\n";
        let files = apply_multi(
            patch,
            vec![
                ("src/old.rs", "fn helper() {}\nfn stays() {}\n"),
                ("src/new.rs", "use crate::common;\n"),
            ],
        )
        .unwrap();
        assert!(
            !files["src/old.rs"].contains("fn helper"),
            "got: {}",
            files["src/old.rs"]
        );
        assert!(
            files["src/old.rs"].contains("fn stays"),
            "got: {}",
            files["src/old.rs"]
        );
        assert!(
            files["src/new.rs"].contains("fn helper"),
            "got: {}",
            files["src/new.rs"]
        );
        assert!(
            files["src/new.rs"].contains("use crate::old_dependency;"),
            "got: {}",
            files["src/new.rs"]
        );
    }

    #[test]
    fn unresolved_use_fails() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,0 @@ rule=paste\n# lang=rust\n# use=Missing\n+ Missing\n";
        let err = apply_multi(patch, vec![("src/lib.rs", "fn a() {}\n")]).unwrap_err();
        assert!(err.contains("RSMPL_CAPTURE_CONFLICT"), "got: {err}");
        assert!(err.contains("Missing"), "got: {err}");
    }

    #[test]
    fn second_hunk_uses_original_lines() {
        // SPEC 29.11: the second semantic hunk keeps its original line number
        // even though the first hunk inserts a larger block before it.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,1 +3,4 @@ rule=wrap_call\n# lang=rust\n# allow=many\n# expr E\n- old_function(E)\n+ new_function_that_spans(\n+     E,\n+ )\n\n@@ -5,1 +5,1 @@ rule=rename_second_function\n# lang=rust\n- fn second_function(...) {\n+ fn renamed_function(...) {\n      ...\n  }\n";
        let src =
            "fn top() {}\nfn old_wrapper() {\n    old_function(1);\n}\nfn second_function() {}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("new_function_that_spans("), "got: {out}");
        assert!(out.contains("fn renamed_function()"), "got: {out}");
    }

    #[test]
    fn use_on_non_insertion_hunk_rejected() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# expr E\n# use=Helper\n- foo(E)\n+ bar(E)\n";
        let err =
            apply_multi(patch, vec![("src/lib.rs", "fn a() {\n    foo(1);\n}\n")]).unwrap_err();
        assert!(err.contains("RSMPL_CAPTURE_CONFLICT"), "got: {err}");
    }

    #[test]
    fn non_preserve_format_rejected_with_cargo_fmt_pointer() {
        // T17: `# format=rustfmt` is rejected, not deferred — the
        // suggestion must name the permanent answer (`cargo fmt`).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# format=rustfmt\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let err = apply_multi(patch, vec![("src/lib.rs", "fn foo() {}\n")]).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("cargo fmt"), "got: {err}");
    }

    #[test]
    fn scope_file_dummy_range_finds_late_item() {
        // T61: `@@ -1,1 @@` + `# scope=file` renames a fn on lines 5-7
        // with no file-read roundtrip.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn foo() {}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("fn bar()"), "got: {out}");
        assert!(!out.contains("fn foo()"), "got: {out}");
    }

    #[test]
    fn zero_match_unique_elsewhere_hints_scope_file() {
        // T62: tight wrong range (lines 1-1) but the only `foo` is on
        // line 5 — the suggestion must offer the zero-discovery retry.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn foo() {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("no match found"), "got: {err}");
        assert!(err.contains("# scope=file"), "got: {err}");
        assert!(err.contains("@@ -1,1 @@"), "got: {err}");
    }

    #[test]
    fn zero_match_many_elsewhere_hints_widen() {
        // T62: zero in range, several matches elsewhere — suggest widening
        // plus `# allow=many` instead of the single-match retry.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src =
            "fn a() {}\nfn b() {\n    foo(1);\n}\nfn c() {\n    foo(2);\n}\nfn d() {\n    foo(3);\n}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("multiple matches exist"), "got: {err}");
        assert!(err.contains("# allow=many"), "got: {err}");
    }

    #[test]
    fn zero_match_nowhere_keeps_generic_hint() {
        // T62: pattern matches nothing anywhere — no scope=file retry.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn nosuchfn(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "fn a() {}\nfn b() {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("read `src/lib.rs`"), "got: {err}");
        assert!(!err.contains("scope=file"), "got: {err}");
    }

    #[test]
    fn scope_file_whole_file_call_rewrite() {
        // T61: `allow=many` + `scope=file` rewrites calls anywhere.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert_eq!(out.matches("bar(").count(), 2, "got: {out}");
        assert!(!out.contains("foo("), "got: {out}");
    }

    #[test]
    fn zero_match_fn_preview_names_nearest() {
        // T67: wrong-name fn rename names the real fn + line in `reason`,
        // so the retry often needs no file read.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn nosuchfn(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "fn a() {}\nfn b() {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("no match found"), "got: {err}");
        assert!(err.contains("nearest"), "got: {err}");
        assert!(err.contains("line 1"), "got: {err}");
    }

    #[test]
    fn zero_match_block_level_preview_names_local_item() {
        // T141: a typo'd block-level rename previews the same-kind local,
        // not the enclosing fn — the retry needs no file read.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=r\n# lang=rust\n- struct Lcoal\n+ struct Fixed\n";
        let src = "fn outer() {\n    struct Local;\n    let _ = Local;\n}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("nearest struct Local at line 2"), "got: {err}");
    }

    #[test]
    fn zero_match_typo_prefers_similar_name_over_nearer() {
        // T142: a typo'd rename previews the most similar same-kind
        // header, not the nearest one — the retry needs no file read.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- struct Betta\n+ struct Gamma\n";
        let src = "struct Alpha;\nstruct Beta;\nfn outer() {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("nearest struct Beta at line 2"), "got: {err}");
    }

    #[test]
    fn zero_match_similarity_tie_breaks_on_distance() {
        // T142: equal edit distance falls back to line distance — both
        // `Zetta` and `Beta` are 1 edit from `Betta`, so the nearer line
        // (old_start=1) wins.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- struct Betta\n+ struct Gamma\n";
        let src = "struct Zetta;\nstruct Beta;\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("nearest struct Zetta at line 1"), "got: {err}");
    }

    #[test]
    fn zero_match_call_preview_shows_source_line() {
        // T67 fallback tier: a call rewrite with no same-kind header walk
        // still previews the nearest non-empty source line.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# expr E\n- nosuchcall(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1);\n}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("nearest source at line"), "got: {err}");
    }

    #[test]
    fn zero_match_impl_preview_names_trait_and_self_ty() {
        // T19b: an impl rename with the wrong name previews the real
        // header with both sides, so the retry needs no file read.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,1 @@ rule=r\n# lang=rust\n- impl Nosuch\n+ impl New\n";
        let src = "trait Old {}\nstruct Foo;\nimpl Old for Foo {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("nearest impl Old for Foo"), "got: {err}");
    }

    #[test]
    fn report_carries_per_match_spans() {
        // T27: `spans` mirrors `matches` with original-source 1-based
        // inclusive line ranges — agents verify without re-reading.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,6 +1,6 @@ rule=r\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\n";
        let (files, report) = apply_multi_report(patch, vec![("src/lib.rs", src)]).unwrap();
        assert_eq!(report.rules.len(), 1);
        let r = &report.rules[0];
        assert_eq!(r.matches, 2);
        assert_eq!(r.spans, vec![(2, 2), (5, 5)], "spans: {:?}", r.spans);
        assert!(files["src/lib.rs"].contains("bar(1)"));
        assert!(files["src/lib.rs"].contains("bar(2)"));
    }

    #[test]
    fn report_carries_per_site_bindings_in_span_order() {
        // WI-2b/2c: `sites` carries one entry per emitted edit with its own
        // bindings — sites 2..N are no longer evidence-free. V2 has no
        // first-site `bindings` compat (removed with `version: 2`).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,9 +1,9 @@ rule=r\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\nfn c() {\n    foo(3);\n}\n";
        let (_, report) = apply_multi_report(patch, vec![("src/lib.rs", src)]).unwrap();
        assert_eq!(report.rules.len(), 1);
        let r = &report.rules[0];
        assert_eq!(r.matches, 3);
        assert_eq!(r.spans, vec![(2, 2), (5, 5), (8, 8)]);
        assert_eq!(r.sites.len(), 3, "sites: {:?}", r.sites);
        for (i, site) in r.sites.iter().enumerate() {
            assert_eq!(site.span, r.spans[i]);
            let e = site
                .bindings
                .iter()
                .find(|(k, _)| k == "E")
                .expect("E binding");
            assert_eq!(e.1, format!("{}", i + 1), "site {i}: {:?}", site);
        }
        // Rename shapes bind nothing: sites omitted.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- fn foo\n+ fn bar\n";
        let (_, report) = apply_multi_report(patch, vec![("src/lib.rs", "fn foo() {}\n")]).unwrap();
        assert!(report.rules[0].sites.is_empty());
        assert!(report.rules[0].follow_skipped.is_none());
    }

    #[test]
    fn report_spans_cover_cut_and_paste() {
        // T27: cut hunks span the removed item lines; paste hunks span the
        // anchor line ([L, L] for zero-width insertions).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -4,1 +4,1 @@ rule=paste_h\n# lang=rust\n# use=H\n+ H\n\n@@ -6,3 +6,0 @@ rule=cut_h\n# lang=rust\n# item I\n# bind=H\n- I\n";
        let src = "fn top() {}\nfn mid() {}\nfn keep() {}\nfn other() {}\nfn tail() {}\nfn helper(x: i32) -> i32 {\n    x + 1\n}\n";
        let (_, report) = apply_multi_report(patch, vec![("src/lib.rs", src)]).unwrap();
        assert_eq!(report.rules.len(), 2);
        // Document order: paste first, cut second (T45). The paste anchor
        // `-4,1` inserts after line 4 (SPEC 10.5), i.e. at line 5.
        assert_eq!(report.rules[0].rule, "paste_h");
        assert_eq!(report.rules[0].spans, vec![(5, 5)]);
        assert_eq!(report.rules[1].rule, "cut_h");
        assert_eq!(report.rules[1].spans, vec![(6, 8)]);
        for r in &report.rules {
            assert_eq!(r.spans.len(), r.matches, "rule {}", r.rule);
        }
    }

    #[test]
    fn preview_truncates_long_lines() {
        // T67 caps: a 300-char call line previews at most 200 chars +
        // ellipsis (fallback tier keeps the label fixed, isolating the
        // excerpt cap; pathological idents are bound by `cap_preview`).
        let long_arg = "a".repeat(300);
        let src = format!("fn a() {{\n    foo({long_arg});\n}}\n");
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- nosuchcall(E)\n+ bar(E)\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src.as_str())]).unwrap_err();
        assert!(err.contains("nearest source at line 2"), "got: {err}");
        // WI-3a: the rendered diagnostic now carries a trailing
        // `repair:` line duplicating the excerpt as `candidate` — measure
        // only the reason's first line (the T67 cap is per-excerpt).
        let tail = err.split("nearest ").nth(1).expect("preview");
        let excerpt = tail.lines().next().expect("reason line");
        assert!(excerpt.chars().count() <= 330, "got: {err}");
        assert!(excerpt.contains('…'), "got: {err}");
    }

    #[test]
    fn ordinary_only_markdown_applies_without_rust_parse() {
        // T137: ordinary-only sections are language-agnostic — no Rust
        // parse of the original or the rewritten file.
        let patch = "diff --git a/SPEC.md b/SPEC.md\n--- a/SPEC.md\n+++ b/SPEC.md\n@@ -1,2 +1,2 @@\n # Title\n-old line\n+new line\n";
        let src = "# Title\nold line\n";
        let files = apply_multi(patch, vec![("SPEC.md", src)]).unwrap();
        assert_eq!(files["SPEC.md"], "# Title\nnew line\n");
    }

    #[test]
    fn mixed_rust_and_markdown_sections() {
        // T137: one Rust section with an RSmPL hunk + one markdown
        // ordinary-only section apply together.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n\ndiff --git a/NOTES.md b/NOTES.md\n--- a/NOTES.md\n+++ b/NOTES.md\n@@ -1,1 +1,1 @@\n-old note\n+new note\n";
        let files = apply_multi(
            patch,
            vec![("src/lib.rs", "fn foo() {}\n"), ("NOTES.md", "old note\n")],
        )
        .unwrap();
        assert!(
            files["src/lib.rs"].contains("fn bar()"),
            "got: {}",
            files["src/lib.rs"]
        );
        assert_eq!(files["NOTES.md"], "new note\n");
    }

    #[test]
    fn ordinary_only_invalid_rust_applies() {
        // T137 negative shape: the skip is per-file semantic presence,
        // not extension sniffing — invalid Rust with only ordinary hunks
        // still applies.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-this is not rust {{{\n+fixed line\n";
        let files = apply_multi(patch, vec![("src/lib.rs", "this is not rust {{{\n")]).unwrap();
        assert_eq!(files["src/lib.rs"], "fixed line\n");
    }

    #[test]
    fn semantic_hunk_on_non_rust_still_parse_conflicts() {
        // T137 negative control: an RSmPL hunk on markdown still fails
        // with RSMPL_PARSE_CONFLICT — the skip covers ordinary-only
        // files, not a blanket removal of the Rust gate.
        let patch = "diff --git a/NOTES.md b/NOTES.md\n--- a/NOTES.md\n+++ b/NOTES.md\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let err = apply_multi(patch, vec![("NOTES.md", "# notes\n")]).unwrap_err();
        assert!(err.contains("RSMPL_PARSE_CONFLICT"), "got: {err}");
        assert!(err.contains("does not parse as Rust"), "got: {err}");
    }

    #[test]
    fn same_anchor_multi_paste_keeps_document_order_with_blanks() {
        // T249: three pastes at one anchor land in document order
        // (first hunk topmost), joined with exactly one blank line.
        // Mirrors the R2 stdin-trio move: pastes [one, two, three] must
        // read [one, two, three], not reversed, with blanks restored.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=paste_one\n# use=One\n+ One\n\n@@ -1,0 @@ rule=paste_two\n# use=Two\n+ Two\n\n@@ -1,0 @@ rule=paste_three\n# use=Three\n+ Three\n\n@@ -2,1 @@ rule=cut_one\n# item I\n# bind=One\n- I\n\n@@ -4,1 @@ rule=cut_two\n# item I\n# bind=Two\n- I\n\n@@ -6,1 @@ rule=cut_three\n# item I\n# bind=Three\n- I\n";
        let src =
            "fn top() {}\nfn one() {}\nfn mid() {}\nfn two() {}\nfn keep() {}\nfn three() {}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        let p1 = out.find("fn one() {}").expect("one");
        let p2 = out.find("fn two() {}").expect("two");
        let p3 = out.find("fn three() {}").expect("three");
        assert!(p1 < p2 && p2 < p3, "document order, got:\n{out}");
        assert!(
            out.contains("fn one() {}\n\nfn two() {}\n\nfn three() {}"),
            "blank-joined, got:\n{out}"
        );
        assert_eq!(out.matches("fn one() {}").count(), 1);
        assert_eq!(out.matches("fn two() {}").count(), 1);
        assert_eq!(out.matches("fn three() {}").count(), 1);
    }

    #[test]
    fn identity_sweep_fails_via_change_gate() {
        // WI-2c: an identity sweep (`foo -> foo`) changes no bytes —
        // loud, not an empty lowering with a clean report. (Ranged
        // fixed-points stay valid matches; only file sweeps owe the
        // change gate.)
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# expr E\n- foo(E)\n+ foo(E)\n";
        let err =
            apply_multi(patch, vec![("src/lib.rs", "fn a() {\n    foo(1);\n}\n")]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("postcondition"), "got: {err}");
    }

    #[test]
    fn widening_sweep_passes_change_gate() {
        // WI-2c: a file sweep whose plus reintroduces the minus
        // (`foo(E)` -> `foo(foo(E))`, like the `24-pat-sweep` widening
        // `E` -> `E.wrapping_add(1)`) still changes bytes, so the change
        // gate passes — minus-zero re-matching would wrongly reject it.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# expr E\n- foo(E)\n+ foo(foo(E))\n";
        let files = apply_multi(patch, vec![("src/lib.rs", "fn a() {\n    foo(1);\n}\n")]).unwrap();
        assert!(
            files["src/lib.rs"].contains("foo(foo(1))"),
            "got: {}",
            files["src/lib.rs"]
        );
    }

    #[test]
    fn single_paste_gains_no_extra_blanks() {
        // T249: the blank-join fires only for N>1 pastes at one anchor.
        // A lone paste keeps the historical `trim_end + "\n"` bytes.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=paste_h\n# lang=rust\n# use=H\n+ H\n\n@@ -3,1 @@ rule=cut_h\n# lang=rust\n# item I\n# bind=H\n- I\n";
        let src = "fn top() {}\nfn mid() {}\nfn helper() {}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert_eq!(out, "fn top() {}\nfn helper() {}\nfn mid() {}\n");
    }

    // WI-6b1: unique-definition cross-file `# follow=uses`. The `files`
    // map is the workspace: the def hunk touches only `a.rs`, uses in
    // `b.rs` rewrite under the same rule with a follow-only entry.

    const XFOLLOW_PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";

    #[test]
    fn cross_file_follow_rewrites_other_file_uses() {
        let (files, report) = apply_multi_report(
            XFOLLOW_PATCH,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                (
                    "src/b.rs",
                    "use crate::Old;\nfn f(x: Old) -> Old {\n    Old\n}\n",
                ),
            ],
        )
        .unwrap();
        assert_eq!(files["src/a.rs"], "pub struct New;\n");
        assert_eq!(
            files["src/b.rs"],
            "use crate::New;\nfn f(x: New) -> New {\n    New\n}\n"
        );
        // One def entry (matches = definitions) plus one follow-only
        // entry for `b.rs` (matches = use-site count, spans agree).
        assert_eq!(report.rules.len(), 2, "report: {:?}", report.rules);
        let def = &report.rules[0];
        assert_eq!(
            (def.rule.as_str(), def.file.as_str()),
            ("rename", "src/a.rs")
        );
        assert_eq!(def.matches, 1);
        let extra = &report.rules[1];
        assert_eq!(
            (extra.rule.as_str(), extra.file.as_str()),
            ("rename", "src/b.rs")
        );
        assert_eq!(extra.matches, 4);
        assert_eq!(extra.spans.len(), 4);
        assert!(extra.sites.is_empty());
        assert!(extra.follow_skipped.is_none());
    }

    #[test]
    fn cross_file_follow_second_definition_vetoes() {
        let err = apply_multi_report(
            XFOLLOW_PATCH,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                ("src/b.rs", "pub struct Old;\n"),
            ],
        )
        .unwrap_err();
        assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
        assert!(err.contains("shadowed"), "got: {err}");
        assert!(err.contains("src/b.rs"), "got: {err}");
    }

    #[test]
    fn cross_file_follow_second_definition_skips_with_fallback() {
        let patch = XFOLLOW_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
        let (files, report) = apply_multi_report(
            &patch,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                (
                    "src/b.rs",
                    "pub struct Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
                ),
            ],
        )
        .unwrap();
        // Definition still renames; ambiguous uses stay stale and report.
        assert_eq!(files["src/a.rs"], "pub struct New;\n");
        assert!(
            files["src/b.rs"].contains("x: Old"),
            "got: {}",
            files["src/b.rs"]
        );
        assert_eq!(report.rules.len(), 1, "report: {:?}", report.rules);
        let skipped = report.rules[0].follow_skipped.as_deref().unwrap_or("");
        assert!(skipped.contains("src/b.rs"), "report: {:?}", report.rules);
    }

    #[test]
    fn cross_file_follow_unparseable_tripwire() {
        // `b.rs` does not parse and names `old`: without fallback the
        // hunk vetoes (never a silent stale site); with it the def still
        // renames and the file is listed.
        let err = apply_multi_report(
            XFOLLOW_PATCH,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                ("src/b.rs", "fn broken( {{{ old\n"),
            ],
        )
        .unwrap_err();
        assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
        assert!(err.contains("src/b.rs"), "got: {err}");
        let patch = XFOLLOW_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
        let (files, report) = apply_multi_report(
            &patch,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                ("src/b.rs", "fn broken( {{{\n"),
            ],
        )
        .unwrap();
        assert_eq!(files["src/a.rs"], "pub struct New;\n");
        assert_eq!(report.rules.len(), 1, "report: {:?}", report.rules);
    }

    #[test]
    fn cross_file_follow_glob_vetoes_then_skips_only_that_file() {
        // WI-6b2-glob: `src/b.rs` pairs `use other::*;` with `Old` uses —
        // unprovable without fallback; with it only `b.rs` stays stale.
        let err = apply_multi_report(
            XFOLLOW_PATCH,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                (
                    "src/b.rs",
                    "use other::*;\nfn f(x: Old) -> Old {\n    x\n}\n",
                ),
                ("src/c.rs", "fn g(x: Old) -> Old {\n    x\n}\n"),
            ],
        )
        .unwrap_err();
        assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
        assert!(err.contains("glob"), "got: {err}");
        assert!(err.contains("src/b.rs"), "got: {err}");
        let patch = XFOLLOW_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
        let (files, report) = apply_multi_report(
            &patch,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                (
                    "src/b.rs",
                    "use other::*;\nfn f(x: Old) -> Old {\n    x\n}\n",
                ),
                ("src/c.rs", "fn g(x: Old) -> Old {\n    x\n}\n"),
            ],
        )
        .unwrap();
        assert_eq!(files["src/a.rs"], "pub struct New;\n");
        assert!(
            files["src/b.rs"].contains("x: Old"),
            "got: {}",
            files["src/b.rs"]
        );
        assert!(
            files["src/c.rs"].contains("x: New"),
            "got: {}",
            files["src/c.rs"]
        );
        let joined: String = report
            .rules
            .iter()
            .filter_map(|r| r.follow_skipped.clone())
            .collect::<Vec<_>>()
            .join(";");
        assert!(joined.contains("src/b.rs"), "report: {:?}", report.rules);
    }

    // WI-7c: plain renames report the uses they leave behind.

    const XORPHAN_PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=rename\n- struct Old\n+ struct New\n";

    #[test]
    fn plain_rename_reports_cross_file_orphans() {
        let (files, report) = apply_multi_report(
            XORPHAN_PATCH,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                (
                    "src/b.rs",
                    "use crate::Old;\nfn f(x: Old) -> Old {\n    Old\n}\n",
                ),
            ],
        )
        .unwrap();
        assert_eq!(files["src/a.rs"], "pub struct New;\n");
        assert!(files["src/b.rs"].contains("Old"), "orphans stay");
        assert_eq!(report.rules.len(), 1);
        let orphans = &report.rules[0].orphans;
        assert_eq!(orphans.len(), 3, "orphans: {orphans:?}");
        assert!(orphans.iter().all(|o| o.file == "src/b.rs"));
        assert!(report.rules[0].follow_skipped.is_none());
    }

    #[test]
    fn plain_rename_without_uses_reports_no_orphans() {
        let (_, report) =
            apply_multi_report(XORPHAN_PATCH, vec![("src/a.rs", "pub struct Old;\n")]).unwrap();
        assert!(report.rules[0].orphans.is_empty());
    }

    #[test]
    fn follow_hunk_carries_no_orphans() {
        let (_, report) = apply_multi_report(
            XFOLLOW_PATCH,
            vec![
                ("src/a.rs", "pub struct Old;\n"),
                ("src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
            ],
        )
        .unwrap();
        for r in &report.rules {
            assert!(r.orphans.is_empty(), "follow rewrites: {r:?}");
        }
    }
}
