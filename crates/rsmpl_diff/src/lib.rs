//! Facade for the inverse mapping (`diffrs`): git source → infer →
//! emit RSmPL.
//!
//! The pub API boundary the `diffrs` binary and T341 tests pin:
//! `compress` collects the native diff and returns a validated
//! RSmPL-Git patch (v1: fn/item renames + slot-sweep renames +
//! method-to-function + method-to-method + call-sweep fan-out compressed inside `rsmpl_diff_infer`, rest
//! ordinary passthrough).

use std::path::Path;

use std::collections::HashMap;
use std::process::Command;

pub use rsmpl_diff_infer::{InferError, InferredPatch};
pub use rsmpl_diff_source::{CollectedDiff, GitSource, SourceError};

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Infer(#[from] InferError),
}

/// Pre-image file contents for the T403 totality check (verbatim
/// sweeps only): `.rs` paths in the diff mapped to the old text the
/// hunks apply against — the index blob for pending worktree diffs
/// (`git show :path`, exact whether or not the index is layered), the
/// `rev` blob for staged and rev diffs. Fetch failures omit the entry
/// (that section falls back to section-visible evidence).
fn pre_images(repo: &Path, staged: bool, rev: Option<&str>, files: &[String]) -> HashMap<String, String> {
    // Index blob (`:path`) for pending worktree diffs — exact whether or
    // not the index is layered; the `rev` blob otherwise.
    let base = if staged || rev.is_some() {
        rev.unwrap_or("HEAD").to_string()
    } else {
        String::new()
    };
    let mut out = HashMap::new();
    for path in files {
        if !path.ends_with(".rs") {
            continue;
        }
        let spec = format!("{base}:{path}");
        let got = Command::new("git").arg("-C").arg(repo).args(["show", &spec]).output();
        if let Ok(o) = got {
            if o.status.success() {
                if let Ok(text) = String::from_utf8(o.stdout) {
                    out.insert(path.clone(), text);
                }
            }
        }
    }
    out
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
    let pre = pre_images(repo, staged, rev, &collected.files);
    let inferred: InferredPatch =
        rsmpl_diff_infer::infer_rsmpl_with_pre_images(&collected.raw, &pre)?;
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
