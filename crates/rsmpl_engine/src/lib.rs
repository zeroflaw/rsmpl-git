//! Semantic matching and rewrite engine (SPEC sections 14, 17-20, 23-24).
//!
//! Processing model: every hunk is matched against **original-source
//! coordinates** (SPEC 10.2). The two-pass planner (`plan`) resolves `#
//! bind=` captures (including forward references, SPEC 23.2), converts all
//! hunks to byte-range `TextEdit`s over the original text, overlap-checks
//! them, then materializes the final files.

#![allow(clippy::result_large_err)]

mod canonical_tree;
mod capture;
mod edits;
mod lines;
mod matcher_expr;
mod matcher_fn;
mod matcher_general;
mod matcher_general_apply;
mod matcher_general_async;
mod matcher_general_ctrl;
mod matcher_general_tails;
mod matcher_impl;
mod matcher_item;
mod matcher_macro;
mod matcher_sequence;
mod matcher_spelling;
mod matcher_types;
mod matcher_use;
mod ordinary;
mod plan;
mod postcheck;
mod suggest;
mod use_graph;
mod use_graph_macro;
mod use_graph_orphan;
mod validate;

use std::collections::HashMap;

use rsmpl_patch::Patch;
use rsmpl_patch::StructuredError;

pub use canonical_tree::canonicalize_with_tree;
pub use edits::{apply_edits, TextEdit};
pub use lines::LineIndex;
pub use ordinary::ordinary_hunk_edits;
pub use plan::apply_with_captures;
pub use validate::{ensure_match_count, ZeroMatchHint};

/// Per-rule application outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuleSemantics {
    /// Plus-instantiation probe re-matched in the post-image (rewrite only).
    Instantiated,
    /// Sweep change gate + instantiation both ran (file-scope sweeps).
    Postcondition,
    /// Only reparse validated (insertions, cuts, pastes, follow-only extras).
    #[default]
    ReparseOnly,
    /// Optional zero-edit hunks (no edits, nothing to verify).
    NotApplicable,
}

impl RuleSemantics {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Instantiated => "instantiated",
            Self::Postcondition => "postcondition",
            Self::ReparseOnly => "reparse-only",
            Self::NotApplicable => "not-applicable",
        }
    }
}

/// Per-rule application outcome.
#[derive(Debug, Clone, Default)]
pub struct RuleReport {
    pub rule: String,
    pub file: String,
    pub matches: usize,
    /// Original-source 1-based inclusive `[start, end]` line ranges, one per
    /// emitted edit in application order. Ident/call/cut/paste rewrites emit
    /// one edit per match (`spans.len() == matches` holds there, empty when
    /// no match); signature-evolution replacements emit one span per edit
    /// (delete+insert pairs). A `# follow=uses` hunk additionally carries
    /// one span per rewritten use site, so `spans.len() >= matches` there
    /// (`matches` counts definitions). Insertions report `[anchor, anchor]`.
    pub spans: Vec<(u32, u32)>,
    /// Per-site evidence (WI-2b, SPEC D.2): one entry per emitted edit in
    /// span order; empty for shapes that bind nothing (omitted from JSON).
    pub sites: Vec<SiteReport>,
    /// `# fallback=syntax` skip notes from `# follow=uses` hunks (SPEC
    /// 14.8): `; `-joined, present only when sites were skipped.
    pub follow_skipped: Option<String>,
    /// Report-only orphan evidence (WI-7c, SPEC 14.8): same-name uses a
    /// definition-only rename leaves behind. Never fails the run.
    pub orphans: Vec<OrphanReport>,
    /// WI-1b: which postcondition verified this rule (SPEC 24.6, D.2).
    pub semantics: RuleSemantics,
}

/// One orphaned use site (WI-7c): repo-relative `file`, 1-based `line`,
/// and the trimmed source `text` of that line.
#[derive(Debug, Clone, Default)]
pub struct OrphanReport {
    pub file: String,
    pub line: u32,
    pub text: String,
}

/// One emitted edit's evidence (WI-2b): its span plus its own bindings,
/// sorted by name (full text; the CLI truncates at report time).
#[derive(Debug, Clone, Default)]
pub struct SiteReport {
    pub span: (u32, u32),
    pub bindings: Vec<(String, String)>,
}

/// Result of applying a patch to an in-memory file tree.
///
/// Only per-rule match counts live here. The changed-file set is owned
/// by the CLI, derived from old-vs-new contents (diff-based observable
/// truth, SPEC D.2), so text and JSON reports cannot diverge.
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    pub rules: Vec<RuleReport>,
    /// True when the patch carries any ordinary (non-semantic) hunk.
    /// Ordinary hunks are reparse-validated only and force the global
    /// `checked.semantics` to `partial` (WI-1b, SPEC 24.6).
    pub has_ordinary: bool,
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
