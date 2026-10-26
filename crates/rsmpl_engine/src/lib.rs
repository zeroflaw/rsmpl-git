#![allow(clippy::result_large_err)]
//! Semantic matching and rewrite engine (SPEC sections 14, 17-20, 23-24).
//!
//! Processing model: every hunk is matched against **original-source
//! coordinates** (SPEC 10.2). All edits become byte-range `TextEdit`s over the
//! original text, overlap-checked, then materialized at once.

mod edits;
mod lines;
mod matcher_expr;
mod matcher_fn;
mod matcher_macro;
mod ordinary;
mod validate;

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, Hunk, Patch, StructuredError};
use rsmpl_pattern::PatternKind;

pub use edits::{apply_edits, TextEdit};
pub use lines::LineIndex;
pub use ordinary::ordinary_hunk_edits;
pub use validate::ensure_match_count;

/// Per-rule application outcome.
#[derive(Debug, Clone, Default)]
pub struct RuleReport {
    pub rule: String,
    pub file: String,
    pub matches: usize,
}

/// Result of applying a patch to an in-memory file tree.
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    pub rules: Vec<RuleReport>,
    pub files_changed: Vec<String>,
}

/// Apply `patch` to `files` (path -> contents) in place.
///
/// `files` keys are repository-relative paths like `src/lib.rs`.
/// Files not present in `files` but targeted by the patch produce a
/// `ParseConflict` (v0.1 has no file creation; new-file content comes only
/// from ordinary hunks against empty base — supported if caller pre-inserts
/// an empty entry).
pub fn apply_patch_to_tree(
    files: &mut HashMap<String, String>,
    patch: &Patch,
) -> Result<ApplyReport, StructuredError> {
    let mut report = ApplyReport::default();
    // Capture map (T4 moves) is file-local here; cross-hunk binds are
    // resolved in `plan.rs` (T4). T3 rejects bind/use with a clear error.
    for section in &patch.files {
        let path = section.target_path.clone();
        let original = files.get(&path).ok_or_else(|| {
            StructuredError::new(
                ErrorCode::ParseConflict,
                format!("target file not found in tree: {path}"),
            )
        })?;
        let original = original.clone();
        let index = LineIndex::new(&original);
        validate::ensure_file_range(&index, section, &path)?;

        let parsed: syn::File = syn::parse_str(&original).map_err(|e| {
            StructuredError::new(
                ErrorCode::ParseConflict,
                format!("target file does not parse as Rust: {e}"),
            )
            .with_file(&path)
        })?;

        let mut edits: Vec<TextEdit> = Vec::new();
        let mut ordinary_edits: Vec<TextEdit> = Vec::new();

        for hunk in &section.hunks {
            match hunk {
                Hunk::Ordinary(oh) => {
                    let mut e = ordinary_hunk_edits(&index, &original, oh)
                        .map_err(|s| s.with_file(&path))?;
                    ordinary_edits.append(&mut e);
                }
                Hunk::Rsmpl(rh) => {
                    if !rh.binds().is_empty() || !rh.uses().is_empty() {
                        return Err(StructuredError::new(
                            ErrorCode::CaptureConflict,
                            "bind/use moves are resolved in the T4 planning phase (not yet implemented)",
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&path));
                    }
                    let compiled = rsmpl_pattern::compile(rh)
                        .map_err(|e| e.with_rule(rh.rule_name()).with_file(&path))?;
                    if !compiled.unbound_plus.is_empty() {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!(
                                "metavariable(s) used in `+` pattern without binding: {}",
                                compiled.unbound_plus.join(", ")
                            ),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&path)
                        .with_suggestion("bind them in `-`/context or declare `# use=`"));
                    }
                    // v0.1 capability gate: only `format=preserve` and AST
                    // matching are implemented (SPEC 22-23). Fail fast rather
                    // than silently deviating.
                    if rh.format_mode() != rsmpl_patch::FormatMode::Preserve {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            "only `# format=preserve` is supported in v0.1",
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&path));
                    }
                    if rh.parse_mode() == rsmpl_patch::ParseMode::Tokens
                        && !matches!(
                            &compiled.kind,
                            PatternKind::Macro { .. }
                                | PatternKind::Expr {
                                    minus: rsmpl_pattern::ExprPatternKind::Macro { .. },
                                    ..
                                }
                        )
                        && rh.macro_mode() != rsmpl_patch::MacroMode::TokenTree
                    {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            "`# mode=tokens` is only supported for macro token-tree patterns in v0.1",
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&path));
                    }
                    let rule = rh.rule_name();
                    let range_desc = format!(
                        "{}..{}",
                        rh.header.old_start,
                        rh.header.old_start + rh.header.old_len
                    );
                    let found = match &compiled.kind {
                        PatternKind::Fn { .. } => matcher_fn::apply_fn_hunk(
                            &parsed, &original, &index, rh, &compiled, &mut edits,
                        )
                        .map_err(|e| e.with_file(&path))?,
                        PatternKind::Expr { minus, .. }
                            if matches!(minus, rsmpl_pattern::ExprPatternKind::Macro { .. }) =>
                        {
                            matcher_macro::apply_macro_hunk(
                                &parsed, &original, &index, rh, &compiled, &mut edits,
                            )
                            .map_err(|e| e.with_file(&path))?
                        }
                        PatternKind::Expr { .. } => matcher_expr::apply_expr_hunk(
                            &parsed, &original, &index, rh, &compiled, &mut edits,
                        )
                        .map_err(|e| e.with_file(&path))?,
                        PatternKind::ItemCapture { .. } => {
                            // Bare `- I` without bind = delete matched item (SPEC 18.2).
                            matcher_fn::apply_item_delete(
                                &parsed, &original, &index, rh, &compiled, &mut edits,
                            )
                            .map_err(|e| e.with_file(&path))?
                        }
                        PatternKind::Insertion { use_name, .. } if use_name.is_some() => {
                            // `+ Helper` with `# use=Helper` needs the T4
                            // capture plan; never insert the name literally.
                            return Err(StructuredError::new(
                                ErrorCode::CaptureConflict,
                                "capture references (`# use=`) are resolved in the T4 planning phase (not yet implemented)",
                            )
                            .with_rule(rh.rule_name())
                            .with_file(&path));
                        }
                        PatternKind::Insertion { .. } => matcher_fn::apply_insertion(
                            &original, &index, rh, &compiled, &mut edits,
                        )
                        .map_err(|e| e.with_file(&path))?,
                        PatternKind::ImplContext { .. } => matcher_fn::apply_impl_context_hunk(
                            &parsed, &original, &index, rh, &compiled, &mut edits,
                        )
                        .map_err(|e| e.with_file(&path))?,
                        PatternKind::Macro { .. } => matcher_macro::apply_macro_hunk(
                            &parsed, &original, &index, rh, &compiled, &mut edits,
                        )
                        .map_err(|e| e.with_file(&path))?,
                        PatternKind::Raw { .. } => {
                            return Err(StructuredError::new(
                                ErrorCode::PatternConflict,
                                "unsupported pattern shape in v0.1 (only fn rename, call rewrite, item delete/insert, macro path rewrite)",
                            )
                            .with_rule(rule.clone())
                            .with_file(&path));
                        }
                    };
                    ensure_match_count(
                        found,
                        rh.allow(),
                        rh.is_optional(),
                        &rule,
                        &path,
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

        // Ordinary-vs-semantic overlap check (SPEC 10.6, 23.5): reject unless
        // disjoint. v0.1 has no "textual refinement" merge.
        for o in &ordinary_edits {
            for s in &edits {
                if o.overlaps(s) {
                    return Err(StructuredError::new(
                        ErrorCode::GitHunkConflict,
                        "ordinary hunk overlaps a semantic hunk range (v0.1 rejects mixed overlap)",
                    )
                    .with_file(&path)
                    .with_suggestion("make the ranges disjoint or combine into one hunk kind"));
                }
            }
        }
        edits.extend(ordinary_edits);

        if edits.is_empty() {
            continue;
        }
        let updated = apply_edits(&original, edits).map_err(|e| e.with_file(&path))?;
        // Reparse validation (SPEC 24.2.3).
        syn::parse_str::<syn::File>(&updated).map_err(|e| {
            StructuredError::new(
                ErrorCode::ParseConflict,
                format!("rewritten file does not parse as Rust: {e}"),
            )
            .with_file(&path)
        })?;
        files.insert(path.clone(), updated);
        report.files_changed.push(path);
    }
    Ok(report)
}
