//! Facade for the inverse mapping (`diffrs`): git source → infer →
//! emit RSmPL.
//!
//! The pub API boundary the `diffrs` binary and T341 tests pin:
//! `compress` collects the native diff and returns a validated
//! RSmPL-Git patch (v1: fn/item renames + slot-sweep renames +
//! method-to-function + method-to-method + call-sweep fan-out compressed inside `rsmpl_diff_infer`, rest
//! ordinary passthrough).

use std::path::Path;

pub use rsmpl_diff_infer::{InferError, InferredPatch};
pub use rsmpl_diff_source::{CollectedDiff, GitSource, SourceError};

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Infer(#[from] InferError),
}

/// A validated RSmPL-Git patch covering the pending native changes.
pub struct CompressedPatch {
    pub rsmpl: String,
    pub files: Vec<String>,
    /// Binary paths stripped during inference (warned on stderr).
    pub stripped_binary: Vec<String>,
    /// Submodule (gitlink) paths stripped during inference (warned on
    /// stderr).
    pub stripped_gitlink: Vec<String>,
    /// A `.history/` audit log was skipped during collection
    /// (warned on stderr, never embedded).
    pub skipped_history: bool,
    /// Untracked directory entries skipped during collection
    /// (warned on stderr, never embedded).
    pub skipped_untracked_dirs: Vec<String>,
}

pub fn compress(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
) -> Result<CompressedPatch, DiffError> {
    let source = GitSource {
        repo: repo.to_path_buf(),
        staged,
        rev: rev.map(str::to_string),
    };
    let collected: CollectedDiff = source.collect()?;
    let inferred: InferredPatch = rsmpl_diff_infer::infer_rsmpl(&collected.raw)?;
    let files = rsmpl_diff_source::files_in_diff(&inferred.rsmpl);
    Ok(CompressedPatch {
        rsmpl: inferred.rsmpl,
        files,
        stripped_binary: inferred.stripped_binary,
        stripped_gitlink: inferred.stripped_gitlink,
        skipped_history: collected.skipped_history,
        skipped_untracked_dirs: collected.skipped_untracked_dirs,
    })
}
