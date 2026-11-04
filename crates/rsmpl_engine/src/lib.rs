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
mod capture_container_body;
mod capture_evolve;
mod capture_impl_body;
mod crate_graph;
mod drift;
mod edits;
mod follow_reasons;
mod lines;
mod matcher_assoc;
mod matcher_attr_delta;
mod matcher_expr;
mod matcher_expr_subst;
mod matcher_fn;
mod matcher_fn_body;
mod matcher_fn_item;
mod matcher_fn_spans;
mod matcher_general;
mod matcher_general_apply;
mod matcher_general_async;
mod matcher_general_binder;
mod matcher_general_ctrl;
mod matcher_general_tails;
mod matcher_impl;
mod matcher_item;
mod matcher_item_joint;
mod matcher_macro;
mod matcher_member;
mod matcher_member_del;
mod matcher_member_evo;
mod matcher_sequence;
mod matcher_spelling;
mod matcher_symbol;
mod matcher_types;
mod matcher_use;
mod matcher_value;
mod ordinary;
mod plan;
mod plan_cut;
mod plan_dispatch;
mod plan_inner;
mod plan_symbol;
mod postcheck;
mod postcheck_assoc;
mod postcheck_attr;
mod postcheck_defs;
mod postcheck_follow;
mod postcheck_item_joint;
mod postcheck_move;
mod postcheck_move_evo;
mod postcheck_move_member;
mod postcheck_namehole;
mod postcheck_synth;
mod postcheck_use;
mod single_site_eq;
mod spell_normalize;
mod suggest;
mod use_graph;
mod use_graph_anchor;
mod use_graph_assoc;
mod use_graph_binder;
mod use_graph_binder_unified;
mod use_graph_follow;
mod use_graph_local;
mod use_graph_local_follow;
mod use_graph_local_orphan;
mod use_graph_macro;
pub(crate) mod use_graph_member;
mod use_graph_method;
mod use_graph_method_calls;
mod use_graph_method_chain;
mod use_graph_method_inherent;
mod use_graph_orphan;
mod use_graph_scope;
mod validate;

use std::collections::HashMap;

use rsmpl_patch::Patch;
use rsmpl_patch::StructuredError;

pub use canonical_tree::canonicalize_with_tree;
pub use crate_graph::CrateGraph;
pub use edits::{apply_edits, TextEdit};
pub use lines::LineIndex;
pub use ordinary::ordinary_hunk_edits;
pub use plan::apply_with_captures;
pub use use_graph_follow::follow_old_ident;
pub use use_graph_local::let_binder_follow_edits;
pub use validate::{ensure_match_count, ZeroMatchHint};

/// Per-rule application outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuleSemantics {
    /// Plus-instantiation probe re-matched in the post-image (rewrite only).
    Instantiated,
    /// Sweep change gate + instantiation both ran (file-scope sweeps),
    /// or a verified structural probe (moves, insertions, cuts,
    /// member additions/evolutions).
    Postcondition,
    /// Only reparse validated (insertions, cuts, pastes, unverified
    /// follow-only extras).
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
    /// WI-2 structured skip evidence (SPEC D.2, 14.8): one entry per
    /// skipped site (`file`, 1-based `line`, `reason` code), sorted by
    /// file/line and deduped. v4 owns the only breaking change of this
    /// round: the `; `-joined `follow_skipped` prose string is removed,
    /// these triples carry the signal alone. Empty when nothing skipped.
    pub follow_skipped_details: Vec<FollowSkipReport>,
    /// Report-only orphan evidence (WI-7c, SPEC 14.8): same-name uses a
    /// definition-only rename leaves behind. Never fails the run.
    pub orphans: Vec<OrphanReport>,
    /// WI-1b: which postcondition verified this rule (SPEC 24.6, D.2).
    pub semantics: RuleSemantics,
    /// WI-16: symbol resolution mode (`"resolved"` when a no-fallback
    /// `# symbol` hunk verified against the crate graph, `"syntax"`
    /// when `# fallback=syntax` carried it; absent otherwise).
    pub symbol: Option<String>,
}

/// WI-2 structured skip evidence (SPEC D.2, 14.8): one assertable
/// triple per skipped site — the file/line plus the machine `reason`
/// code (`shadowed` | `glob-reexport` | `second-hop` | `macro-hidden` |
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

#[cfg(test)]
mod tests {
    /// WI-20 gate (extends WI-22 slice 2b): unit tests stay inline (`mod tests`
    /// at the bottom of the owning file). No `*_tests.rs` file may exist
    /// anywhere in the workspace (excluding `target/`), and no `.rs` file
    /// may carry `#[path = "*_tests.rs"]`. Integration/e2e files under
    /// `tests/` keep their names — only the `*_tests.rs` suffix is banned.
    #[test]
    fn wi22_no_exiled_sibling_tests() {
        // WI-20: allow-list empty — no `*_tests.rs` exists in the workspace
        // (landing scan: `git ls-files | grep _tests.rs` empty). Any future
        // `*_tests.rs` file or `#[path]` exile fails here.
        let allowed: std::collections::HashSet<&str> = [].into_iter().collect();
        let root = workspace_root();
        let mut offenders: Vec<String> = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(top) = stack.pop() {
            let entries = std::fs::read_dir(&top)
                .unwrap_or_else(|e| panic!("WI-22 scan cannot read {}: {e}", top.display()));
            for entry in entries {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    if path
                        .file_name()
                        .is_some_and(|n| n == "target" || n == ".git")
                    {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let rel = path.strip_prefix(&root).unwrap_or(&path);
                let rel_str = rel.display().to_string().replace('\\', "/");
                if allowed.contains(rel_str.as_str()) {
                    continue;
                }
                if path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with("_tests.rs"))
                {
                    offenders.push(format!("sibling file: {}", rel.display()));
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                // Skip the gate's own file (it names the pattern it bans).
                if text.contains("wi22_no_exiled_sibling_tests") {
                    continue;
                }
                if text.contains("#[path = \"") && text.contains("_tests.rs") {
                    offenders.push(format!("#[path] exile: {}", rel.display()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "WI-22: exiled sibling tests found (move them inline):\n{}",
            offenders.join("\n")
        );
    }

    fn workspace_root() -> std::path::PathBuf {
        let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        loop {
            if dir.join("crates").is_dir() && dir.join("bin").is_dir() {
                return dir;
            }
            if !dir.pop() {
                panic!("WI-22 scan cannot locate the workspace root");
            }
        }
    }

    /// WI-19 gate: the 1900-line warning line before the 2000-line hard
    /// cap. Any production `.rs` file (under `crates/*/src/` or
    /// `bin/*/src/`, the same scope as the WI-22 scan) over 1900 lines
    /// fails unless it names its split seam on the allowlist; over 2000
    /// fails unconditionally (no allowlist excuses the hard cap, which
    /// `no_rust_file_exceeds_2000_lines` enforces independently).
    #[test]
    fn wi19_no_file_exceeds_1900_lines() {
        // Landing: allowlist empty — WI-19 split the six watch files
        // (`arg_template` → `arg_template_single`,
        // `generalized` → `generalized_let`, `macros` →
        // `macro_rename_single`, `general_expr` → `general_expr_single`,
        // `methods` → `methods_literal`, `mod` → `item_single`).
        // Entry format when one is ever needed:
        //   "crates/foo/src/big.rs" → "seam: <what moves where>; owner: WI-N".
        let allowed: std::collections::HashMap<&str, &str> = [].into_iter().collect();
        let root = workspace_root();
        let mut offenders: Vec<String> = Vec::new();
        for rel in src_rs_files(&root) {
            if let Some(offense) = wi19_offender(&rel, file_lines(&root.join(&rel)), &allowed) {
                offenders.push(offense);
            }
        }
        assert!(
            offenders.is_empty(),
            "WI-19: production files over the 1900-line watch line (split verbatim onto a seam first):\n{}",
            offenders.join("\n")
        );
    }

    /// Pure WI-19 decision: `None` when the file is within budget,
    /// `Some(reason)` naming the breach. The allowlist excuses the
    /// 1900 watch line only; the 2000 hard cap ignores it.
    fn wi19_offender(
        rel: &str,
        lines: usize,
        allowed: &std::collections::HashMap<&str, &str>,
    ) -> Option<String> {
        if lines > 2000 {
            return Some(format!("{rel} ({lines} lines: over the 2000 hard cap)"));
        }
        if lines > 1900 && !allowed.contains_key(rel) {
            return Some(format!(
                "{rel} ({lines} lines: over 1900 without an allowlist seam)"
            ));
        }
        None
    }

    /// WI-19 logic pin: the allowlist passes a listed 1901-line file
    /// and the hard cap fails even a listed file — no workspace touch.
    #[test]
    fn wi19_allowlist_and_hard_cap_logic() {
        let allowed: std::collections::HashMap<&str, &str> = [(
            "crates/x/src/big.rs",
            "seam: single-core split; owner: WI-99",
        )]
        .into_iter()
        .collect();
        assert!(wi19_offender("crates/x/src/ok.rs", 1900, &allowed).is_none());
        assert!(wi19_offender("crates/x/src/big.rs", 1901, &allowed).is_none());
        assert!(wi19_offender("crates/x/src/new.rs", 1901, &allowed).is_some());
        assert!(wi19_offender("crates/x/src/big.rs", 2001, &allowed).is_some());
    }

    /// Production `.rs` files: `crates/*/src/**` + `bin/*/src/**`,
    /// workspace-relative, `/`-joined. Integration `tests/` trees are
    /// out of scope (WI-20 owns their naming, not their length).
    fn src_rs_files(root: &std::path::Path) -> Vec<String> {
        let mut out = Vec::new();
        for dir in ["crates", "bin"] {
            let base = root.join(dir);
            let mut stack = vec![base];
            while let Some(top) = stack.pop() {
                let entries = std::fs::read_dir(&top)
                    .unwrap_or_else(|e| panic!("WI-19 scan cannot read {}: {e}", top.display()));
                for entry in entries {
                    let path = entry.expect("dir entry").path();
                    if path.is_dir() {
                        if path.file_name().is_some_and(|n| n == "tests") {
                            continue;
                        }
                        stack.push(path);
                        continue;
                    }
                    if path.extension().is_none_or(|e| e != "rs") {
                        continue;
                    }
                    let rel = path.strip_prefix(root).unwrap_or(&path);
                    if !rel.components().any(|c| c.as_os_str() == "src") {
                        continue;
                    }
                    out.push(rel.display().to_string().replace('\\', "/"));
                }
            }
        }
        out.sort();
        out
    }

    fn file_lines(path: &std::path::Path) -> usize {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .count()
    }
}
