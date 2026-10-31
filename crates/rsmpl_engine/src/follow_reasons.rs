//! WI-2 structured follow-skip evidence (SPEC D.2, 14.8).
//!
//! Every `# fallback=syntax` skip already emits a human prose `note`
//! (unchanged — existing tests pin its bytes); this module pairs each
//! note with the machine triple the JSON report asserts on: `file`,
//! 1-based `line` (`0` = whole-file skip, used only for unparseable
//! workspace files that name `old`), and `reason`, one of the SPEC 14.8
//! vocabulary codes below. New sibling module (D5: `use_graph.rs` is
//! past the 1600-line watch line, so no new code lands there).

/// SPEC 14.8 skip-reason vocabulary.
pub mod reason {
    /// Another container defines the same name; no site is provable.
    pub const SHADOWED: &str = "shadowed";
    /// A `use …::*` glob may import a different `old`.
    pub const GLOB_REEXPORT: &str = "glob-reexport";
    /// A re-export chain passes through a second re-exporter.
    pub const SECOND_HOP: &str = "second-hop";
    /// Macro/attribute/`qself` interiors or unsearchable files.
    pub const MACRO_HIDDEN: &str = "macro-hidden";
    /// A re-export/naming that does not point at our definition.
    pub const OFF_TARGET: &str = "off-target";
}

/// One skipped use site: the existing prose `note` plus the assertable
/// triple. `file` is `""` for same-file `resolve()` outcomes — the
/// caller fills the definition file when the workspace step runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowSkip {
    pub note: String,
    pub file: String,
    pub line: u32,
    pub reason: &'static str,
}

impl FollowSkip {
    pub fn new(note: String, file: &str, line: u32, reason: &'static str) -> Self {
        Self {
            note,
            file: file.to_string(),
            line,
            reason,
        }
    }

    /// Same-file `resolve()` skip: the workspace step fills `file`.
    pub fn same_file(note: String, line: u32, reason: &'static str) -> Self {
        Self {
            note,
            file: String::new(),
            line,
            reason,
        }
    }

    /// Whole-file skip (unparseable workspace file): no single line.
    pub fn whole_file(note: String, file: &str, reason: &'static str) -> Self {
        Self {
            note,
            file: file.to_string(),
            line: 0,
            reason,
        }
    }

    /// Prose notes for the compat `follow_skipped` string (unchanged order).
    pub fn notes(skips: &[FollowSkip]) -> Vec<String> {
        skips.iter().map(|s| s.note.clone()).collect()
    }

    /// Report triples, sorted by (file, line) and deduped.
    pub fn details(skips: &[FollowSkip], def_file: &str) -> Vec<crate::FollowSkipReport> {
        let mut out: Vec<crate::FollowSkipReport> = skips
            .iter()
            .map(|s| crate::FollowSkipReport {
                file: if s.file.is_empty() {
                    def_file.to_string()
                } else {
                    s.file.clone()
                },
                line: s.line,
                reason: s.reason.to_string(),
            })
            .collect();
        out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
        out.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.reason == b.reason);
        out
    }
}
