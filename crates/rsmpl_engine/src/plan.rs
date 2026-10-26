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

use crate::capture::CaptureStore;
use crate::edits::{apply_edits, TextEdit};
use crate::lines::LineIndex;
use crate::validate::{ensure_match_count, ZeroMatchHint};
use crate::{matcher_expr, matcher_fn, matcher_general, matcher_item, matcher_macro};
use crate::{ordinary_hunk_edits, RuleReport};

/// Parsed original state for one file.
struct FileState {
    original: String,
    index: LineIndex,
    parsed: syn::File,
}

/// Apply `patch` to `files` (path -> contents) in place.
pub fn apply_with_captures(
    files: &mut HashMap<String, String>,
    patch: &Patch,
) -> Result<crate::ApplyReport, StructuredError> {
    let mut report = crate::ApplyReport::default();

    // ---- Pass 0: parse all targeted files. ----
    let mut states: HashMap<String, FileState> = HashMap::new();
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
        })?;
        let index = LineIndex::new(&original);
        crate::validate::ensure_file_range(&index, section, &path)?;
        let parsed: syn::File = syn::parse_str(&original).map_err(|e| {
            StructuredError::new(
                ErrorCode::ParseConflict,
                format!("target file does not parse as Rust: {e}"),
            )
            .with_file(&path)
        })?;
        states.insert(
            path,
            FileState {
                original,
                index,
                parsed,
            },
        );
    }

    let mut captures = CaptureStore::default();
    // Per-file edit lists, built across pass 1 (cuts) and pass 2 (rest).
    let mut file_edits: HashMap<String, Vec<TextEdit>> = HashMap::new();
    for path in states.keys() {
        file_edits.insert(path.clone(), Vec::new());
    }
    // Rule reports staged with their document position (T45): pass 1
    // evaluates cuts first, but the report must read in patch document
    // order (section order, then hunk order), so entries are sorted
    // after pass 2 before being committed to `report.rules`.
    let mut rule_order: Vec<((usize, usize), RuleReport)> = Vec::new();

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
                .with_file(path));
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
                .with_file(path));
            }
            let targets = matcher_fn::find_item_targets(
                &state.parsed,
                rh.scope(),
                rh.header.old_start,
                rh.header.old_len,
            );
            let range_desc = format!(
                "{}..{}",
                rh.header.old_start,
                rh.header.old_start + rh.header.old_len
            );
            if targets.is_empty() && rh.is_optional() {
                continue; // optional cut with no match: no capture, no cut
            }
            // T62: same whole-file probe for cut hunks (span collection,
            // no edits involved).
            let wide = if targets.is_empty() && rh.scope() != Scope::File {
                let wide_targets = matcher_fn::find_item_targets(
                    &state.parsed,
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
                rsmpl_patch::Allow::One,
                false,
                &rh.rule_name(),
                path,
                &range_desc,
                ZeroMatchHint { wide, preview },
            )?;
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
            let cut =
                matcher_fn::delete_lines(&state.original, &state.index, sl, el, &rh.rule_name())
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
                },
            ));
        }
    }

    // ---- Pass 2: every other hunk in document order. ----
    for (sec_idx, section) in patch.files.iter().enumerate() {
        let path = &section.target_path;
        let Some(state) = states.get(path) else {
            continue;
        };
        let edits = file_edits.get_mut(path).expect("file state");
        for (hunk_idx, hunk) in section.hunks.iter().enumerate() {
            match hunk {
                Hunk::Ordinary(oh) => {
                    let mut e = ordinary_hunk_edits(&state.index, &state.original, oh)
                        .map_err(|s| s.with_file(path))?;
                    edits.append(&mut e);
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
                    let found = apply_rsmpl_hunk(state, path, rh, &compiled, &captures, edits)
                        .map_err(|e| e.with_file(path))?;
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
                        .with_file(path));
                    }
                    // T62: on a hard zero-match, probe the same pattern
                    // whole-file (scope overridden, count-only, edits
                    // discarded) so an unambiguous failure can suggest the
                    // zero-discovery `# scope=file` retry. Insertions use
                    // anchors rather than pattern search and are excluded
                    // by the probe itself.
                    let wide = if found == 0 && !rh.is_optional() && rh.scope() != Scope::File {
                        probe_whole_file(state, path, rh, &compiled, &captures)
                    } else {
                        None
                    };
                    // T67: diagnostic-only excerpt; happy/optional paths pay nothing.
                    let preview = if found == 0 && !rh.is_optional() {
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
                    rule_order.push((
                        (sec_idx, hunk_idx),
                        RuleReport {
                            rule,
                            file: path.clone(),
                            matches: found,
                        },
                    ));
                }
            }
        }
    }

    // Commit rule reports in patch document order (T36/T45 contract:
    // section order, then hunk order within a section).
    rule_order.sort_by_key(|(pos, _)| *pos);
    report.rules.extend(rule_order.into_iter().map(|(_, r)| r));

    // ---- Pass 3: overlap check, materialize, reparse-validate. ----
    for (path, state) in &states {
        let edits = file_edits.remove(path).unwrap_or_default();
        if edits.is_empty() {
            continue;
        }
        check_overlaps(&edits, path)?;
        let updated = apply_edits(&state.original, edits).map_err(|e| e.with_file(path))?;
        syn::parse_str::<syn::File>(&updated).map_err(|e| {
            StructuredError::new(
                ErrorCode::ParseConflict,
                format!("rewritten file does not parse as Rust: {e}"),
            )
            .with_file(path)
        })?;
        files.insert(path.clone(), updated);
    }

    Ok(report)
}

/// Dispatch one pass-2 RSmPL hunk to its matcher (SPEC 23.1 step 7).
///
/// Extracted from the pass-2 loop so the T62 whole-file probe reuses the
/// exact same dispatch (no predicate duplication, no drift). File context
/// (`with_file`) is applied by the caller, not here, so real and probe
/// runs share error shapes.
fn apply_rsmpl_hunk(
    state: &FileState,
    path: &str,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    captures: &CaptureStore,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let rule = rh.rule_name();
    match &compiled.kind {
        PatternKind::Fn { .. } => matcher_fn::apply_fn_hunk(
            &state.parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Expr {
            minus: rsmpl_pattern::ExprPatternKind::Macro { .. },
            ..
        } => matcher_macro::apply_macro_hunk(
            &state.parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Expr { .. } => matcher_expr::apply_expr_hunk(
            &state.parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ItemRename { .. } => matcher_item::apply_item_rename_hunk(
            &state.parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::General { .. } => matcher_general::apply_general_hunk(
            &state.parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ItemCapture { .. } => matcher_fn::apply_item_delete(
            &state.parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Insertion { use_name, .. } => match use_name {
            Some(name) => {
                paste_capture(
                    captures,
                    name,
                    &state.index,
                    rh.header.old_start,
                    &rule,
                    path,
                    edits,
                )?;
                Ok(1)
            }
            None => {
                matcher_fn::apply_insertion(&state.original, &state.index, rh, compiled, edits)
            }
        },
        PatternKind::ImplContext { .. } => matcher_fn::apply_impl_context_hunk(
            &state.parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Macro { .. } => matcher_macro::apply_macro_hunk(
            &state.parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Raw { .. } => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "unsupported pattern shape in v0.1 (only fn rename, struct/enum/trait/union/type/const/static/mod/impl item rename, call rewrite, let-stmt/general-expr/type rewrite, item delete/insert, macro path rewrite)",
        )
        .with_rule(rule)),
    }
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
}

/// T67 candidate preview: nearest same-kind item header (or nearest
/// non-empty source line as fallback) for zero-match diagnostics.
///
/// Diagnostic-only: returns a preformatted, pre-capped excerpt such as
/// `fn foo at line 5: 'fn foo() {}'`. Single line, at most 200 chars of
/// source plus an ellipsis marker. `None` when the file has nothing to
/// show (empty file). Callers fire this only on the hard zero-match path,
/// so successful and `# optional` hunks pay nothing.
fn candidate_preview(
    state: &FileState,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<String> {
    let old_start = rh.header.old_start;
    let kind_specific = matches!(
        &compiled.kind,
        PatternKind::Fn { .. }
            | PatternKind::ImplContext { .. }
            | PatternKind::ItemCapture { .. }
            | PatternKind::ItemRename { .. }
    );
    if kind_specific {
        let mut headers = Vec::new();
        collect_item_headers(&state.parsed.items, &mut headers);
        if let Some((line, label)) = nearest_header(&headers, old_start) {
            if let Some(excerpt) = source_line(state, line) {
                return Some(cap_preview(format!("{label} at line {line}: '{excerpt}'")));
            }
        }
    }
    nearest_source(state, old_start).map(cap_preview)
}

/// `(line, label)` item headers for the T67 kind-aware tier.
fn collect_item_headers(items: &[syn::Item], out: &mut Vec<(u32, String)>) {
    use syn::spanned::Spanned;
    for it in items {
        match it {
            syn::Item::Fn(f) => {
                let line = f.sig.fn_token.span().start().line as u32;
                out.push((line, format!("fn {}", f.sig.ident)));
            }
            syn::Item::Struct(s) => {
                out.push((
                    s.ident.span().start().line as u32,
                    format!("struct {}", s.ident),
                ));
            }
            syn::Item::Enum(e) => {
                out.push((
                    e.ident.span().start().line as u32,
                    format!("enum {}", e.ident),
                ));
            }
            syn::Item::Trait(t) => {
                out.push((
                    t.ident.span().start().line as u32,
                    format!("trait {}", t.ident),
                ));
            }
            syn::Item::Union(u) => {
                out.push((
                    u.ident.span().start().line as u32,
                    format!("union {}", u.ident),
                ));
            }
            syn::Item::Type(t) => {
                out.push((
                    t.ident.span().start().line as u32,
                    format!("type {}", t.ident),
                ));
            }
            syn::Item::Const(c) => {
                out.push((
                    c.ident.span().start().line as u32,
                    format!("const {}", c.ident),
                ));
            }
            syn::Item::Static(s) => {
                out.push((
                    s.ident.span().start().line as u32,
                    format!("static {}", s.ident),
                ));
            }
            syn::Item::Mod(m) => {
                out.push((
                    m.ident.span().start().line as u32,
                    format!("mod {}", m.ident),
                ));
                if let Some((_, inner)) = &m.content {
                    collect_item_headers(inner, out);
                }
            }
            syn::Item::Impl(imp) => {
                let line = imp.self_ty.span().start().line as u32;
                out.push((line, "impl".to_string()));
                for it in &imp.items {
                    if let syn::ImplItem::Fn(m) = it {
                        let line = m.sig.fn_token.span().start().line as u32;
                        out.push((line, format!("fn {}", m.sig.ident)));
                    }
                }
            }
            _ => {}
        }
    }
}

fn nearest_header(headers: &[(u32, String)], old_start: u32) -> Option<(u32, String)> {
    headers
        .iter()
        .min_by_key(|(line, _)| line.abs_diff(old_start))
        .map(|(line, label)| (*line, label.clone()))
}

/// First source line at 1-based `line`, trimmed, capped at 200 chars.
fn source_line(state: &FileState, line: u32) -> Option<String> {
    let start = state.index.line_start(line)?;
    let end = state
        .index
        .line_start(line + 1)
        .unwrap_or_else(|| state.index.eof());
    let raw = state.original.get(start..end.min(state.original.len()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Single-line excerpt: a header line never legitimately spans lines
    // here (`source_line` slices one line); cut at the first newline anyway.
    let first = trimmed.lines().next().unwrap_or("");
    Some(truncate_excerpt(first))
}

fn truncate_excerpt(s: &str) -> String {
    const MAX: usize = 200;
    if s.chars().count() > MAX {
        let mut out: String = s.chars().take(MAX).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

/// Hard bound on the whole preview string: pathological idents (a 300-char
/// fn name) would otherwise blow past the 200-char excerpt cap via the
/// `kind name` label. Single line, token-bounded failure output.
fn cap_preview(s: String) -> String {
    const MAX: usize = 320;
    if s.chars().count() > MAX {
        let mut out: String = s.chars().take(MAX).collect();
        out.push('…');
        out
    } else {
        s
    }
}

/// Fallback tier: nearest non-empty source line to `old_start`.
fn nearest_source(state: &FileState, old_start: u32) -> Option<String> {
    let n = state.index.line_count() as u32;
    let mut best: Option<(u32, u32)> = None; // (distance, line)
    for line in 1..=n {
        let start = state.index.line_start(line)?;
        let end = state
            .index
            .line_start(line + 1)
            .unwrap_or_else(|| state.index.eof());
        let raw = state.original.get(start..end.min(state.original.len()))?;
        if raw.trim().is_empty() {
            continue;
        }
        let d = line.abs_diff(old_start);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, line));
        }
    }
    let (_, line) = best?;
    source_line(state, line).map(|excerpt| format!("source at line {line}: '{excerpt}'"))
}

/// v0.1 capability gate (SPEC 22-23): only `format=preserve` and AST matching
/// (plus macro token-tree patterns).
fn gate_directives(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    path: &str,
) -> Result<(), StructuredError> {
    if rh.format_mode() != rsmpl_patch::FormatMode::Preserve {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "only `# format=preserve` is supported in v0.1",
        )
        .with_rule(rh.rule_name())
        .with_file(path));
    }
    let is_macro = matches!(
        &compiled.kind,
        PatternKind::Macro { .. }
            | PatternKind::Expr {
                minus: rsmpl_pattern::ExprPatternKind::Macro { .. },
                ..
            }
    ) || rh.macro_mode() == rsmpl_patch::MacroMode::TokenTree;
    if rh.parse_mode() == rsmpl_patch::ParseMode::Tokens && !is_macro {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# mode=tokens` is only supported for macro token-tree patterns in v0.1",
        )
        .with_rule(rh.rule_name())
        .with_file(path));
    }
    Ok(())
}

fn unbound_plus(rh: &rsmpl_patch::RsmplHunk, names: &[String], path: &str) -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        format!(
            "metavariable(s) used in `+` pattern without binding: {}",
            names.join(", ")
        ),
    )
    .with_rule(rh.rule_name())
    .with_file(path)
    .with_suggestion("bind them in `-`/context or declare `# use=`")
}

/// Paste captured item text at a zero-length anchor (SPEC 10.5, 20.2).
fn paste_capture(
    captures: &CaptureStore,
    name: &str,
    index: &LineIndex,
    anchor: u32,
    rule: &str,
    file: &str,
    edits: &mut Vec<TextEdit>,
) -> Result<(), StructuredError> {
    let text = captures.get(name, rule, file)?;
    let pos = if anchor == 0 {
        0
    } else {
        index.line_start(anchor + 1).unwrap_or_else(|| index.eof())
    };
    let mut ins = text.trim_end().to_string();
    if !ins.ends_with('\n') {
        ins.push('\n');
    }
    edits.push(TextEdit::new(pos, pos, ins));
    Ok(())
}

/// Reject incompatible overlapping edits (SPEC 10.6, 23.5).
///
/// Same-point insertions are allowed (deterministic stable order, document
/// order). An insertion strictly inside a deleted/replaced range is rejected,
/// as are partially overlapping replacements.
fn check_overlaps(edits: &[TextEdit], path: &str) -> Result<(), StructuredError> {
    for i in 0..edits.len() {
        for j in (i + 1)..edits.len() {
            let (a, b) = (&edits[i], &edits[j]);
            if a.is_insertion() && b.is_insertion() {
                continue; // same-point insertions: deterministic order
            }
            if a.overlaps(b) {
                return Err(StructuredError::new(
                    ErrorCode::GitHunkConflict,
                    "overlapping edits cannot be merged (ordinary hunk overlaps a semantic range?)",
                )
                .with_file(path)
                .with_suggestion("make the ranges disjoint or combine into one hunk kind"));
            }
        }
    }
    Ok(())
}

fn range_err(rule: &str, file: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_file(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_multi(
        patch_text: &str,
        initial: Vec<(&str, &str)>,
    ) -> Result<HashMap<String, String>, String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files: HashMap<String, String> = initial
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        apply_with_captures(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files)
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
    fn preview_truncates_long_lines() {
        // T67 caps: a 300-char call line previews at most 200 chars +
        // ellipsis (fallback tier keeps the label fixed, isolating the
        // excerpt cap; pathological idents are bound by `cap_preview`).
        let long_arg = "a".repeat(300);
        let src = format!("fn a() {{\n    foo({long_arg});\n}}\n");
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- nosuchcall(E)\n+ bar(E)\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src.as_str())]).unwrap_err();
        assert!(err.contains("nearest source at line 2"), "got: {err}");
        let excerpt = err.split("nearest ").nth(1).expect("preview");
        assert!(excerpt.chars().count() <= 330, "got: {err}");
        assert!(excerpt.contains('…'), "got: {err}");
    }
}
