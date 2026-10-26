//! Match-count and range validation (SPEC 24, 25).

use rsmpl_patch::{Allow, ErrorCode, FileSection, StructuredError};

use crate::lines::LineIndex;

/// Validate `allow` / `optional` match counts (SPEC 17.5, 24.3).
pub fn ensure_match_count(
    found: usize,
    allow: Allow,
    optional: bool,
    rule: &str,
    file: &str,
    range: &str,
) -> Result<(), StructuredError> {
    match (found, allow, optional) {
        (0, _, true) => Ok(()),
        (0, _, false) => Err(
            StructuredError::new(ErrorCode::MatchConflict, "no match found")
                .with_rule(rule)
                .with_file(file)
                .with_range(range)
                .with_suggestion(format!(
                    "run `rsmpl-git --inspect {file}` (add `--calls` for call-rewrite ranges) to list valid ranges; check the pattern or add `# optional`"
                )),
        ),
        (1, _, _) => Ok(()),
        (_, Allow::Many, _) => Ok(()),
        (_, Allow::One, _) => Err(StructuredError::new(
            ErrorCode::MatchConflict,
            format!("expected one match, found {found}"),
        )
        .with_rule(rule)
        .with_file(file)
        .with_range(range)
        .with_suggestion("add `# allow=many` if multiple rewrites are intended")),
    }
}

/// Validate that every old-side RSmPL/ordinary range is inside the file.
pub fn ensure_file_range(
    index: &LineIndex,
    section: &FileSection,
    path: &str,
) -> Result<(), StructuredError> {
    let n = index.line_count() as u32;
    for hunk in &section.hunks {
        let h = hunk.header();
        let rule = h.rule.clone().unwrap_or_default();
        if h.old_len == 0 {
            // Insertion anchor may be one past the last line.
            if h.old_start > n + 1 {
                return Err(range_err(&rule, path, h.old_start, h.old_len, n));
            }
            continue;
        }
        if h.old_start == 0 || h.old_start - 1 + h.old_len > n {
            return Err(range_err(&rule, path, h.old_start, h.old_len, n));
        }
    }
    Ok(())
}

fn range_err(rule: &str, file: &str, start: u32, len: u32, n: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("old range {start},{len} exceeds file length ({n} lines)"),
    )
    .with_rule(rule)
    .with_file(file)
    .with_range(format!("{start}..{}", start + len))
    .with_suggestion(format!(
        "run `rsmpl-git --inspect {file}` to list valid ranges"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_match_suggestion_carries_inspect_command() {
        let err = ensure_match_count(
            0,
            rsmpl_patch::Allow::One,
            false,
            "r",
            "src/lib.rs",
            "40..47",
        )
        .unwrap_err();
        let s = err.suggestion.expect("suggestion");
        assert!(s.contains("rsmpl-git --inspect src/lib.rs"), "got: {s}");
        assert!(s.contains("--calls"), "got: {s}");
        assert!(s.contains("# optional"), "got: {s}");
    }

    #[test]
    fn range_suggestion_carries_inspect_command() {
        let err = range_err("r", "src/lib.rs", 90, 8, 10);
        let s = err.suggestion.expect("suggestion");
        assert!(s.contains("rsmpl-git --inspect src/lib.rs"), "got: {s}");
    }
}
