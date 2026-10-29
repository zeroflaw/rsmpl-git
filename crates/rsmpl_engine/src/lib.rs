//! Semantic matching and rewrite engine (SPEC sections 14, 17-20, 23-24).
//!
//! Processing model: every hunk is matched against **original-source
//! coordinates** (SPEC 10.2). The two-pass planner (`plan`) resolves `#
//! bind=` captures (including forward references, SPEC 23.2), converts all
//! hunks to byte-range `TextEdit`s over the original text, overlap-checks
//! them, then materializes the final files.

#![allow(clippy::result_large_err)]

mod capture;
mod edits;
mod lines;
mod matcher_expr;
mod matcher_fn;
mod matcher_general;
mod matcher_general_ctrl;
mod matcher_impl;
mod matcher_item;
mod matcher_macro;
mod matcher_use;
mod ordinary;
mod plan;
mod use_graph;
mod validate;

use std::collections::HashMap;

use rsmpl_patch::Patch;
use rsmpl_patch::StructuredError;

pub use edits::{apply_edits, TextEdit};
pub use lines::LineIndex;
pub use ordinary::ordinary_hunk_edits;
pub use plan::apply_with_captures;
pub use validate::{ensure_match_count, ZeroMatchHint};

/// Per-rule application outcome.
#[derive(Debug, Clone, Default)]
pub struct RuleReport {
    pub rule: String,
    pub file: String,
    pub matches: usize,
    /// Original-source 1-based inclusive `[start, end]` line spans, one per
    /// emitted edit in application order. Ident/call/cut/paste rewrites emit
    /// one edit per match (`spans.len() == matches` holds there, empty when
    /// no match); signature-evolution replacements emit one span per edit
    /// (delete+insert pairs). A `# follow=uses` hunk additionally carries
    /// one span per rewritten use site, so `spans.len() >= matches` there
    /// (`matches` counts definitions). Insertions report `[anchor, anchor]`.
    pub spans: Vec<(u32, u32)>,
    /// First-match metavariable bindings (SPEC D.2).
    pub bindings: Vec<(String, String)>,
    /// Omitted-entry count when the CLI truncates the report (SPEC D.2).
    pub bindings_truncated: usize,
}

/// Result of applying a patch to an in-memory file tree.
///
/// Only per-rule match counts live here. The changed-file set is owned
/// by the CLI, derived from old-vs-new contents (diff-based observable
/// truth, SPEC D.2), so text and JSON reports cannot diverge.
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    pub rules: Vec<RuleReport>,
}

/// Apply `patch` to `files` (path -> contents) in place.
///
/// `files` keys are repository-relative paths like `src/lib.rs`.
pub fn apply_patch_to_tree(
    files: &mut HashMap<String, String>,
    patch: &Patch,
) -> Result<ApplyReport, StructuredError> {
    apply_with_captures(files, patch)
}
