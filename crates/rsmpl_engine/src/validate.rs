//! Match-count and range validation (SPEC 24, 25).

use rsmpl_patch::{Allow, ErrorCode, FileSection, Hunk, Scope, StructuredError};

use crate::lines::LineIndex;

/// Zero-match diagnostic context (SPEC 25.4).
///
/// Bundled so `ensure_match_count` stays at 7 arguments (clippy
/// `too_many_arguments`): both fields are diagnostic-only and computed only
/// on the hard zero-match path.
pub struct ZeroMatchHint {
    /// Whole-file (`# scope=file`) probe count for the same pattern (T62).
    pub wide: Option<usize>,
    /// T67 candidate excerpt, already truncated (e.g.
    /// `fn foo at line 5: 'fn foo() {}'`). `None` keeps the bare reason.
    pub preview: Option<String>,
}

/// Validate `allow` / `optional` match counts (SPEC 17.5, 24.3).
///
/// `hint.wide` is the whole-file (`# scope=file`) probe count for the same
/// pattern, when the caller ran one (T62): `Some(1)` means the pattern is
/// fine but the range is wrong, so the suggestion offers the zero-discovery
/// `# scope=file` retry; `Some(n > 1)` suggests widening + `# allow=many`;
/// `Some(0)`/`None` keep the generic file-read suggestion.
///
/// `hint.preview` is the T67 candidate excerpt, appended to the zero-match
/// reason so the next attempt often needs no discovery roundtrip. `None`
/// keeps the bare `no match found` reason (e.g. empty file).
pub fn ensure_match_count(
    found: usize,
    allow: Allow,
    optional: bool,
    rule: &str,
    file: &str,
    range: &str,
    hint: ZeroMatchHint,
) -> Result<(), StructuredError> {
    match (found, allow, optional) {
        (0, _, true) => Ok(()),
        (0, _, false) => Err(StructuredError::new(
            ErrorCode::MatchConflict,
            zero_match_reason(hint.preview.as_deref()),
        )
        .with_rule(rule)
        .with_file(file)
        .with_range(range)
        .with_suggestion(zero_match_suggestion(file, hint.wide))),
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
        // `# scope=file` searches the whole file; the `@@` range is a
        // dummy (canonical `@@ -1,1 @@`) and skips bounds validation.
        // Insertion anchors (`old_len==0`) still use the header as the
        // anchor position, so scope is moot there but harmless.
        if let Hunk::Rsmpl(rh) = hunk {
            if rh.scope() == Scope::File {
                continue;
            }
        }
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

fn zero_match_reason(preview: Option<&str>) -> String {
    match preview {
        Some(p) => format!("no match found — nearest {p}"),
        None => "no match found".to_string(),
    }
}

fn zero_match_suggestion(file: &str, wide: Option<usize>) -> String {
    let base = format!("read `{file}` to list valid ranges; check the pattern or add `# optional`");
    match wide {
        Some(1) => format!(
            "{base} — exactly one match exists elsewhere in the file; retry with `# scope=file` + `@@ -1,1 @@`"
        ),
        // Note: matchers stop counting at 2 under `# allow=one`, so the
        // wide count saturates — report "multiple", never an exact number.
        Some(n) if n > 1 => format!(
            "{base} — multiple matches exist outside this range; widen the range (or add `# scope=file`) and add `# allow=many` if intended"
        ),
        _ => base,
    }
}

fn range_err(rule: &str, file: &str, start: u32, len: u32, n: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("old range {start},{len} exceeds file length ({n} lines)"),
    )
    .with_rule(rule)
    .with_file(file)
    .with_range(format!("{start}..{}", start + len))
    .with_suggestion(format!("read `{file}` to list valid ranges"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_match_suggestion_names_file_read() {
        let err = ensure_match_count(
            0,
            rsmpl_patch::Allow::One,
            false,
            "r",
            "src/lib.rs",
            "40..47",
            ZeroMatchHint {
                wide: None,
                preview: None,
            },
        )
        .unwrap_err();
        let s = err.suggestion.expect("suggestion");
        assert!(s.contains("read `src/lib.rs`"), "got: {s}");
        assert!(s.contains("# optional"), "got: {s}");
        assert!(!s.contains("--inspect"), "got: {s}");
        assert!(!s.contains("scope=file"), "got: {s}");
    }

    #[test]
    fn wide_hint_single_suggests_scope_file() {
        let err = ensure_match_count(
            0,
            rsmpl_patch::Allow::One,
            false,
            "r",
            "src/lib.rs",
            "1..2",
            ZeroMatchHint {
                wide: Some(1),
                preview: None,
            },
        )
        .unwrap_err();
        let s = err.suggestion.expect("suggestion");
        assert!(s.contains("read `src/lib.rs`"), "got: {s}");
        assert!(s.contains("# scope=file"), "got: {s}");
        assert!(s.contains("@@ -1,1 @@"), "got: {s}");
    }

    #[test]
    fn wide_hint_many_suggests_widen_and_allow_many() {
        let err = ensure_match_count(
            0,
            rsmpl_patch::Allow::One,
            false,
            "r",
            "src/lib.rs",
            "1..2",
            ZeroMatchHint {
                wide: Some(3),
                preview: None,
            },
        )
        .unwrap_err();
        let s = err.suggestion.expect("suggestion");
        assert!(
            s.contains("multiple matches exist outside this range"),
            "got: {s}"
        );
        assert!(s.contains("# allow=many"), "got: {s}");
    }

    #[test]
    fn range_suggestion_names_file_read() {
        let err = range_err("r", "src/lib.rs", 90, 8, 10);
        let s = err.suggestion.expect("suggestion");
        assert!(s.contains("read `src/lib.rs`"), "got: {s}");
        assert!(!s.contains("--inspect"), "got: {s}");
    }

    #[test]
    fn preview_appends_to_reason() {
        let err = ensure_match_count(
            0,
            rsmpl_patch::Allow::One,
            false,
            "r",
            "src/lib.rs",
            "1..2",
            ZeroMatchHint {
                wide: None,
                preview: Some("fn foo at line 5: 'fn foo() {}'".to_string()),
            },
        )
        .unwrap_err();
        let reason = err.message.clone();
        assert!(reason.contains("no match found"), "got: {reason}");
        assert!(reason.contains("nearest fn foo"), "got: {reason}");
        assert!(reason.contains("line 5"), "got: {reason}");
    }

    #[test]
    fn no_preview_keeps_bare_reason() {
        let err = ensure_match_count(
            0,
            rsmpl_patch::Allow::One,
            false,
            "r",
            "src/lib.rs",
            "1..2",
            ZeroMatchHint {
                wide: None,
                preview: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.message, "no match found");
    }

    #[test]
    fn scope_file_skips_range_bounds() {
        use crate::lines::LineIndex;
        // T61: dummy `90,8` in a 10-line file passes with `# scope=file`.
        let text = "fn a() {}\n".repeat(10);
        let index = LineIndex::new(&text);
        let section: FileSection = rsmpl_patch::parse_patch(
            "diff --git src/lib.rs\n@@ -90,8 @@ rule=r\n# scope=file\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
        )
        .unwrap()
        .files
        .remove(0);
        assert!(ensure_file_range(&index, &section, "src/lib.rs").is_ok());
        // Same dummy without `scope=file` fails.
        let section: FileSection = rsmpl_patch::parse_patch(
            "diff --git src/lib.rs\n@@ -90,8 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
        )
        .unwrap()
        .files
        .remove(0);
        assert!(ensure_file_range(&index, &section, "src/lib.rs").is_err());
    }
}
