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
mod drift;
mod edits;
mod follow_reasons;
mod lines;
mod matcher_attr_delta;
mod matcher_expr;
mod matcher_expr_subst;
mod matcher_fn;
mod matcher_general;
mod matcher_general_apply;
mod matcher_general_async;
mod matcher_general_ctrl;
mod matcher_general_tails;
mod matcher_impl;
mod matcher_item;
mod matcher_macro;
mod matcher_member;
mod matcher_sequence;
mod matcher_spelling;
mod matcher_types;
mod matcher_use;
mod ordinary;
mod plan;
mod postcheck;
mod postcheck_attr;
mod postcheck_namehole;
mod suggest;
mod use_graph;
mod use_graph_macro;
pub(crate) mod use_graph_member;
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
    /// WI-2 span edit labels, parallel to `spans` (`None` = unlabeled).
    /// Multi-edit rules (signature evolution, follow-augmented hunks) tag
    /// each span (`generics`/`params`/`return`/`rename`/`follow`);
    /// single-edit rules leave every entry `None` (omitted from JSON).
    pub span_labels: Vec<Option<String>>,
    /// Per-site evidence (WI-2b, SPEC D.2): one entry per emitted edit in
    /// span order; empty for shapes that bind nothing (omitted from JSON).
    pub sites: Vec<SiteReport>,
    /// `# fallback=syntax` skip notes from `# follow=uses` hunks (SPEC
    /// 14.8): `; `-joined, present only when sites were skipped.
    pub follow_skipped: Option<String>,
    /// WI-2 structured skip evidence (SPEC D.2, 14.8): one entry per
    /// skipped site (`file`, 1-based `line`, `reason` code), sorted by
    /// file/line and deduped. Additive beside `follow_skipped` (WI-20
    /// owns the string's removal); empty when nothing was skipped.
    pub follow_skipped_details: Vec<FollowSkipReport>,
    /// Report-only orphan evidence (WI-7c, SPEC 14.8): same-name uses a
    /// definition-only rename leaves behind. Never fails the run.
    pub orphans: Vec<OrphanReport>,
    /// WI-1b: which postcondition verified this rule (SPEC 24.6, D.2).
    pub semantics: RuleSemantics,
}

/// WI-2 structured skip evidence (SPEC D.2, 14.8): the file/line the
/// `follow_skipped` prose names, plus the machine `reason` code
/// (`shadowed` | `glob-reexport` | `second-hop` | `macro-hidden` |
/// `off-target`). File-level skips (unparseable files) report `line: 1`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FollowSkipReport {
    pub file: String,
    pub line: u32,
    pub reason: String,
}

/// One orphaned use site (WI-7c): repo-relative `file`, 1-based `line`,
/// and the trimmed source `text` of that line. WI-2 `reason` is one of
/// `off-target` (names `old`, left because the rename is definition-only)
/// or `macro-hidden` (macro/attribute/`qself` interiors the resolver
/// cannot attribute).
#[derive(Debug, Clone, Default)]
pub struct OrphanReport {
    pub file: String,
    pub line: u32,
    pub text: String,
    pub reason: String,
}

/// One emitted edit's evidence (WI-2b): its span plus its own bindings,
/// sorted by name (full text; the CLI truncates at report time).
#[derive(Debug, Clone, Default)]
pub struct SiteReport {
    pub span: (u32, u32),
    pub bindings: Vec<(String, String)>,
}

/// One drifted hunk application (WI-10b, SPEC 10.8): the cited range was
/// stale, so the engine retried once file-scoped, anchored by the hunk's
/// body context lines. `cited` is the stale header range, `actual` the
/// bounding box of the applied spans (both original-source 1-based
/// inclusive line ranges).
#[derive(Debug, Clone, Default)]
pub struct DriftReport {
    pub rule: String,
    pub file: String,
    pub cited: (u32, u32),
    pub actual: (u32, u32),
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
    /// WI-10b drift evidence, in application order (empty when no hunk
    /// drifted; omitted from reports then).
    pub drifts: Vec<DriftReport>,
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
