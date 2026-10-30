//! WI-2c semantic postcondition (SPEC 24.6).
//!
//! Replaces the tautological `check_untouched` re-derivation (which could
//! only fail on engine non-determinism, never on real user input) with a
//! sweep change gate that fails loud on real patches:
//!
//! every non-`optional` file-scope `# allow=many` hunk that matched must
//! have at least one byte-changing definition edit. A sweep claims every
//! site in scope; a sweep that changes nothing (e.g. `foo(E) -> foo(E)`)
//! is a loud `RSMPL_MATCH_CONFLICT`, never an empty lowering with a clean
//! report.
//!
//! Deliberately NOT a "minus must be zero in the post-image" check:
//! widening rewrites legitimately re-match their own output (`let P: T =
//! E` -> `let P: T = E.wrapping_add(1)` still matches `let P: T = E` on
//! the wrapped init; `foo(E)` -> `foo(foo(E))` likewise). Re-matching
//! was probe-verified against the `24-pat-sweep` fixture and rejected
//! for exactly this reason. Ranged singles are skipped (other sites may
//! legitimately remain, and byte-identical fixed-point instantiations
//! per SPEC 18.6 R4 `P @` stay valid matches). `optional` zero-edit
//! hunks are skipped by design (no edits expected).

use rsmpl_patch::{Allow, ErrorCode, Scope, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::edits::TextEdit;

/// Whether this hunk owes the sweep change gate: non-`optional`,
/// file-scope `# allow=many`, countable rewrite kind (insertions, cuts
/// and `Raw` shapes never participate).
pub(crate) fn sweep_hunk_needs_postcheck(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> bool {
    if rh.is_optional() {
        return false;
    }
    if rh.allow() != Allow::Many || rh.scope() != Scope::File {
        return false;
    }
    !matches!(
        &compiled.kind,
        PatternKind::Insertion { .. } | PatternKind::ItemCapture { .. } | PatternKind::Raw { .. }
    )
}

/// True when the hunk's definition edits changed no bytes (every
/// replacement is byte-identical to the original slice it covers).
pub(crate) fn sweep_edits_are_noop(original: &str, def_edits: &[TextEdit]) -> bool {
    if def_edits.is_empty() {
        return false;
    }
    def_edits.iter().all(|e| {
        original
            .get(e.start..e.end)
            .is_some_and(|slice| slice == e.replacement)
    })
}

/// Postcondition failure for a sweep whose edits change nothing
/// (SPEC 24.6, code `RSMPL_MATCH_CONFLICT` so agents reuse the
/// match-conflict repair path).
pub(crate) fn noop_sweep_error(rule: &str, file: &str, range_desc: &str) -> StructuredError {
    StructuredError::new(
        ErrorCode::MatchConflict,
        "postcondition: sweep matched but changed no bytes (plus still matches minus)",
    )
    .with_rule(rule)
    .with_file(file)
    .with_range(range_desc)
    .with_suggestion(
        "change the plus side so the rewrite alters the matched text, or restrict the hunk range",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(start: usize, end: usize, replacement: &str) -> TextEdit {
        TextEdit::new(start, end, replacement.to_string())
    }

    #[test]
    fn noop_when_every_replacement_is_identical() {
        let original = "fn a() {\n    foo(1);\n}\n";
        // `foo(1)` at bytes 13..19 replaced with itself.
        assert!(sweep_edits_are_noop(original, &[edit(13, 19, "foo(1)")]));
    }

    #[test]
    fn change_when_any_replacement_differs() {
        let original = "fn a() {\n    foo(1);\n}\n";
        assert!(!sweep_edits_are_noop(original, &[edit(13, 19, "bar(1)")]));
        assert!(!sweep_edits_are_noop(
            original,
            &[edit(13, 19, "foo(1)"), edit(13, 19, "bar(1)")]
        ));
    }

    #[test]
    fn empty_edits_are_not_noop() {
        assert!(!sweep_edits_are_noop("fn a() {}\n", &[]));
    }

    #[test]
    fn deletions_change_bytes() {
        let original = "fn a() {}\nfn b() {}\n";
        assert!(!sweep_edits_are_noop(original, &[edit(0, 10, "")]));
    }
}
