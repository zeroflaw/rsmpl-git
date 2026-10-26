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
mod matcher_item;
mod matcher_macro;
mod ordinary;
mod plan;
mod validate;

use std::collections::HashMap;

use rsmpl_patch::Patch;
use rsmpl_patch::StructuredError;

pub use edits::{apply_edits, TextEdit};
pub use lines::LineIndex;
pub use ordinary::ordinary_hunk_edits;
pub use plan::apply_with_captures;
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
pub fn apply_patch_to_tree(
    files: &mut HashMap<String, String>,
    patch: &Patch,
) -> Result<ApplyReport, StructuredError> {
    apply_with_captures(files, patch)
}
