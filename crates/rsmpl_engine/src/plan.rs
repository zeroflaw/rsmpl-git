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

use rsmpl_patch::{ErrorCode, Hunk, Patch, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::capture::CaptureStore;
use crate::edits::{apply_edits, TextEdit};
use crate::lines::LineIndex;
use crate::validate::ensure_match_count;
use crate::{matcher_expr, matcher_fn, matcher_macro};
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

    // ---- Pass 1: `# bind=` cut hunks (v0.1: item captures only). ----
    for section in &patch.files {
        let path = &section.target_path;
        let Some(state) = states.get(path) else {
            continue;
        };
        for hunk in &section.hunks {
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
            ensure_match_count(
                targets.len(),
                rsmpl_patch::Allow::One,
                false,
                &rh.rule_name(),
                path,
                &range_desc,
            )?;
            let (sl, el) = targets[0];
            let start = state
                .index
                .line_start(sl)
                .ok_or_else(|| range_err(&rh.rule_name(), path, sl))?;
            let end = state
                .index
                .line_start(el + 1)
                .unwrap_or_else(|| state.index.eof());
            let captured = state.original.get(start..end).unwrap_or("").to_string();
            captures.define(binds[0], captured, &rh.rule_name(), path)?;
            file_edits
                .get_mut(path)
                .expect("file state")
                .push(TextEdit::new(start, end, String::new()));
            report.rules.push(RuleReport {
                rule: rh.rule_name(),
                file: path.clone(),
                matches: 1,
            });
        }
    }

    // ---- Pass 2: every other hunk in document order. ----
    for section in &patch.files {
        let path = &section.target_path;
        let Some(state) = states.get(path) else {
            continue;
        };
        let edits = file_edits.get_mut(path).expect("file state");
        for hunk in &section.hunks {
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
                    let found = match &compiled.kind {
                        PatternKind::Fn { .. } => matcher_fn::apply_fn_hunk(
                            &state.parsed,
                            &state.original,
                            &state.index,
                            rh,
                            &compiled,
                            edits,
                        )
                        .map_err(|e| e.with_file(path))?,
                        PatternKind::Expr { minus, .. }
                            if matches!(minus, rsmpl_pattern::ExprPatternKind::Macro { .. }) =>
                        {
                            matcher_macro::apply_macro_hunk(
                                &state.parsed,
                                &state.original,
                                &state.index,
                                rh,
                                &compiled,
                                edits,
                            )
                            .map_err(|e| e.with_file(path))?
                        }
                        PatternKind::Expr { .. } => matcher_expr::apply_expr_hunk(
                            &state.parsed,
                            &state.original,
                            &state.index,
                            rh,
                            &compiled,
                            edits,
                        )
                        .map_err(|e| e.with_file(path))?,
                        PatternKind::ItemCapture { .. } => matcher_fn::apply_item_delete(
                            &state.parsed,
                            &state.original,
                            &state.index,
                            rh,
                            &compiled,
                            edits,
                        )
                        .map_err(|e| e.with_file(path))?,
                        PatternKind::Insertion { use_name, .. } => match use_name {
                            Some(name) => {
                                paste_capture(
                                    &captures,
                                    name,
                                    &state.index,
                                    rh.header.old_start,
                                    &rule,
                                    path,
                                    edits,
                                )?;
                                1
                            }
                            None => matcher_fn::apply_insertion(
                                &state.original,
                                &state.index,
                                rh,
                                &compiled,
                                edits,
                            )
                            .map_err(|e| e.with_file(path))?,
                        },
                        PatternKind::ImplContext { .. } => matcher_fn::apply_impl_context_hunk(
                            &state.parsed,
                            &state.original,
                            &state.index,
                            rh,
                            &compiled,
                            edits,
                        )
                        .map_err(|e| e.with_file(path))?,
                        PatternKind::Macro { .. } => matcher_macro::apply_macro_hunk(
                            &state.parsed,
                            &state.original,
                            &state.index,
                            rh,
                            &compiled,
                            edits,
                        )
                        .map_err(|e| e.with_file(path))?,
                        PatternKind::Raw { .. } => {
                            return Err(StructuredError::new(
                                ErrorCode::PatternConflict,
                                "unsupported pattern shape in v0.1 (only fn rename, call rewrite, item delete/insert, macro path rewrite)",
                            )
                            .with_rule(rule.clone())
                            .with_file(path));
                        }
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
                        .with_file(path));
                    }
                    ensure_match_count(
                        found,
                        rh.allow(),
                        rh.is_optional(),
                        &rule,
                        path,
                        &range_desc,
                    )?;
                    report.rules.push(RuleReport {
                        rule,
                        file: path.clone(),
                        matches: found,
                    });
                }
            }
        }
    }

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
        report.files_changed.push(path.clone());
    }

    Ok(report)
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
            &patch,
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
        let err = apply_multi(&patch, vec![("src/lib.rs", "fn a() {}\n")]).unwrap_err();
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
        let files = apply_multi(&patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("new_function_that_spans("), "got: {out}");
        assert!(out.contains("fn renamed_function()"), "got: {out}");
    }

    #[test]
    fn use_on_non_insertion_hunk_rejected() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# expr E\n# use=Helper\n- foo(E)\n+ bar(E)\n";
        let err =
            apply_multi(&patch, vec![("src/lib.rs", "fn a() {\n    foo(1);\n}\n")]).unwrap_err();
        assert!(err.contains("RSMPL_CAPTURE_CONFLICT"), "got: {err}");
    }
}
