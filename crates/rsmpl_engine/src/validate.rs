//! Match-count and range validation (SPEC 24, 25).

use rsmpl_patch::{Allow, ErrorCode, FileSection, Hunk, Scope, StructuredError};
use rsmpl_pattern::PatternKind;

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

/// WI-3b: fill a missing machine `repair` for engine-boundary errors.
///
/// Specific gates pin exact payloads; this covers the long tail so no
/// `PatternConflict`/`SymbolConflict`/`CaptureConflict` is prose-only:
/// pattern/capture conflicts gain `fix_pattern` with the reason echoed
/// as `write_instead` (reasons already say what to write instead);
/// symbol conflicts gain `add_directive # fallback=syntax` (every v0.1
/// symbol gate's documented fix).
pub fn fill_generic_repair(err: StructuredError) -> StructuredError {
    if err.repair.is_some() {
        return err;
    }
    match err.code {
        ErrorCode::PatternConflict => {
            let write_instead = err.message.clone();
            err.with_repair(StructuredError::fix_pattern_repair(
                "pattern",
                &write_instead,
            ))
        }
        ErrorCode::SymbolConflict => {
            err.with_repair(StructuredError::add_directive_repair("# fallback=syntax"))
        }
        ErrorCode::CaptureConflict => {
            let write_instead = err.message.clone();
            err.with_repair(StructuredError::fix_pattern_repair(
                "capture",
                &write_instead,
            ))
        }
        _ => err,
    }
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
        .with_suggestion(zero_match_suggestion(file, hint.wide))
        .with_repair(zero_match_repair(
            file,
            range,
            hint.wide,
            hint.preview.as_deref(),
        ))),
        (1, _, _) => Ok(()),
        (_, Allow::Many, _) => Ok(()),
        (_, Allow::One, _) => Err(StructuredError::new(
            ErrorCode::MatchConflict,
            format!("expected one match, found {found}"),
        )
        .with_rule(rule)
        .with_file(file)
        .with_range(range)
        .with_suggestion("add `# allow=many` if multiple rewrites are intended")
        .with_repair(serde_json::json!({
            "action": "add_directive",
            "directive": "# allow=many",
        }))),
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

/// WI-3a: machine repair for the zero-match path (SPEC D.2).
///
/// Exactly one whole-file match → `widen_scope` (directive + dummy range
/// the agent applies verbatim, mirroring the suggestion). Otherwise →
/// `read_file` naming the file/range, with the T67 `preview` excerpt
/// attached as `candidate` when present (verbatim string in v0.1; a
/// structured `{line, text}` split defers to WI-3b).
fn zero_match_repair(
    file: &str,
    range: &str,
    wide: Option<usize>,
    preview: Option<&str>,
) -> serde_json::Value {
    if wide == Some(1) {
        return serde_json::json!({
            "action": "widen_scope",
            "directive": "# scope=file",
            "range": "@@ -1,1 @@",
        });
    }
    match preview {
        Some(p) => serde_json::json!({
            "action": "read_file",
            "file": file,
            "range": range,
            "candidate": p,
        }),
        None => serde_json::json!({
            "action": "read_file",
            "file": file,
            "range": range,
        }),
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

/// v0.1 capability gate (SPEC 22-23): only `format=preserve` and AST matching
/// (plus macro token-tree patterns).
pub(crate) fn gate_directives(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    path: &str,
) -> Result<(), StructuredError> {
    if rh.format_mode() != rsmpl_patch::FormatMode::Preserve {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "only `# format=preserve` is supported in v0.1",
        )
        .with_rule(rh.rule_name())
        .with_file(path)
        .with_suggestion("remove the directive and run `cargo fmt` after apply")
        .with_repair(StructuredError::fix_pattern_repair(
            "format",
            "remove the directive and run `cargo fmt` after apply",
        )));
    }
    // WI-6a: `# follow=uses` is legal only on fn and item rename hunks
    // (SPEC 18.6); every other shape keeps the SPEC 14.8 follow-up hunks.
    if rh.follows_uses()
        && !matches!(
            &compiled.kind,
            PatternKind::Fn { .. } | PatternKind::ItemRename { .. }
        )
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "# follow=uses is only supported on fn and item rename hunks in v0.1 (other shapes keep the SPEC 14.8 follow-up hunks)",
        )
        .with_rule(rh.rule_name())
        .with_file(path)
        .with_suggestion("remove `# follow=uses`, or rewrite this hunk as a fn/item rename")
        .with_repair(StructuredError::fix_pattern_repair(
            "C17",
            "remove `# follow=uses`, or rewrite this hunk as a fn/item rename",
        )));
    }
    let is_macro = matches!(
        &compiled.kind,
        PatternKind::Macro { .. }
            | PatternKind::Expr {
                minus: rsmpl_pattern::ExprPatternKind::Macro { .. },
                ..
            }
    ) || rh.macro_mode() == rsmpl_patch::MacroMode::TokenTree;
    if rh.parse_mode() == rsmpl_patch::ParseMode::Tokens && !is_macro {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# mode=tokens` is only supported for macro token-tree patterns in v0.1",
        )
        .with_rule(rh.rule_name())
        .with_file(path)
        .with_repair(StructuredError::fix_pattern_repair(
            "tokens",
            "remove `# mode=tokens`, or write a macro token-tree pattern",
        )));
    }
    Ok(())
}

pub(crate) fn unbound_plus(
    rh: &rsmpl_patch::RsmplHunk,
    names: &[String],
    path: &str,
) -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        format!(
            "metavariable(s) used in `+` pattern without binding: {}",
            names.join(", ")
        ),
    )
    .with_rule(rh.rule_name())
    .with_file(path)
    .with_suggestion("bind them in `-`/context or declare `# use=`")
    .with_repair(StructuredError::fix_pattern_repair(
        "unbound",
        "bind them in `-`/context or declare `# use=`",
    ))
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

    #[test]
    fn zero_match_single_wide_carries_widen_scope_repair() {
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
        let repair = err.repair.expect("zero-match must carry repair");
        assert_eq!(repair["action"], serde_json::json!("widen_scope"));
        assert_eq!(repair["directive"], serde_json::json!("# scope=file"));
        assert_eq!(repair["range"], serde_json::json!("@@ -1,1 @@"));
    }

    #[test]
    fn zero_match_plain_carries_read_file_repair() {
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
        let repair = err.repair.expect("zero-match must carry repair");
        assert_eq!(repair["action"], serde_json::json!("read_file"));
        assert_eq!(repair["file"], serde_json::json!("src/lib.rs"));
        assert_eq!(repair["range"], serde_json::json!("1..2"));
        assert!(repair.get("candidate").is_none(), "got: {repair}");
    }

    #[test]
    fn zero_match_preview_attaches_candidate() {
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
        let repair = err.repair.expect("zero-match must carry repair");
        assert_eq!(repair["action"], serde_json::json!("read_file"));
        assert_eq!(
            repair["candidate"],
            serde_json::json!("fn foo at line 5: 'fn foo() {}'")
        );
    }

    #[test]
    fn generic_repair_fills_bare_conflicts_and_preserves_specific() {
        // Bare pattern conflict gains generic fix_pattern.
        let bare = StructuredError::new(ErrorCode::PatternConflict, "some shape rejected");
        let filled = fill_generic_repair(bare);
        let repair = filled.repair.expect("must carry repair");
        assert_eq!(repair["action"], serde_json::json!("fix_pattern"));
        assert_eq!(repair["code"], serde_json::json!("pattern"));
        // Bare symbol conflict gains the fallback directive.
        let bare = StructuredError::new(ErrorCode::SymbolConflict, "unresolved");
        let filled = fill_generic_repair(bare);
        let repair = filled.repair.expect("must carry repair");
        assert_eq!(repair["action"], serde_json::json!("add_directive"));
        assert_eq!(repair["directive"], serde_json::json!("# fallback=syntax"));
        // Bare capture conflict gains generic fix_pattern.
        let bare = StructuredError::new(ErrorCode::CaptureConflict, "not bound");
        let filled = fill_generic_repair(bare);
        assert_eq!(
            filled.repair.expect("must carry repair")["action"],
            serde_json::json!("fix_pattern")
        );
        // Specific payloads are never overwritten.
        let specific = StructuredError::new(ErrorCode::PatternConflict, "c3").with_repair(
            StructuredError::fix_pattern_repair("C3", "write a `let` statement pattern"),
        );
        let kept = fill_generic_repair(specific);
        assert_eq!(
            kept.repair.expect("must keep repair")["code"],
            serde_json::json!("C3")
        );
        // Non-conflict errors stay repair-free.
        let io = StructuredError::new(ErrorCode::IoConflict, "bad flag");
        assert!(fill_generic_repair(io).repair.is_none());
    }

    #[test]
    fn too_many_carries_add_directive_repair() {
        let err = ensure_match_count(
            3,
            rsmpl_patch::Allow::One,
            false,
            "r",
            "src/lib.rs",
            "1..10",
            ZeroMatchHint {
                wide: None,
                preview: None,
            },
        )
        .unwrap_err();
        let repair = err.repair.expect("too-many must carry repair");
        assert_eq!(repair["action"], serde_json::json!("add_directive"));
        assert_eq!(repair["directive"], serde_json::json!("# allow=many"));
    }
}
