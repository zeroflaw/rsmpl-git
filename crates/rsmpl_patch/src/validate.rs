//! Whole-patch validation for RSmPL-Git patch files (SPEC 20.4, 24.x).
//!
//! Split from `parser.rs` (WI-3c): cross-hunk checks that need the
//! whole parsed patch live here; line-oriented parsing stays in
//! `parser.rs`. Pure move, no behavior change.

use super::parser::Result;
use crate::errors::{ErrorCode, StructuredError};
use crate::model::*;

/// Early validation: reject `` r#`type` `` (split raw-ident quoting).
/// Valid: `` `r#type` `` or bare `r#type`. Invalid: `r#` followed by backtick.
pub(crate) fn validate_no_broken_raw_quote_pattern(text: &str) -> Result<()> {
    if text.contains("r#`") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "invalid raw-identifier quoting `r#` + backtick in pattern `{text}` (use `` `r#...` ``)"
            ),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "quote",
            "use `` `r#...` `` (backticks around the whole raw identifier)",
        ))
        .into());
    }
    // Unbalanced backticks.
    let count = text.chars().filter(|&c| c == '`').count();
    if count % 2 != 0 {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("unbalanced backtick quoting in pattern `{text}`"),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "quote",
            "balance the backticks in the pattern (every `` ` `` needs a pair)",
        ))
        .into());
    }
    Ok(())
}

pub(crate) fn validate_no_broken_raw_quote_directive(line: &str) -> Result<()> {
    if line.contains("r#`") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid raw-identifier quoting in directive `{line}`"),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "quote",
            "use `` `r#...` `` (backticks around the whole raw identifier)",
        ))
        .into());
    }
    Ok(())
}

/// Cross-hunk validation that needs the whole patch (SPEC 20.4, 24.x).
pub(crate) fn validate_patch(patch: &Patch) -> Result<()> {
    use std::collections::{HashMap, HashSet};
    // Collect binds; check duplicates; check bind+allow=many (SPEC 20.4).
    let mut bind_to_hunk: HashMap<String, (String, String)> = HashMap::new();
    let mut use_refs: Vec<(String, String, String)> = Vec::new(); // (capture, file, rule)
    for f in &patch.files {
        for h in &f.hunks {
            if let Hunk::Rsmpl(rh) = h {
                for b in rh.binds() {
                    if bind_to_hunk.contains_key(b) {
                        return Err(StructuredError::new(
                            ErrorCode::CaptureConflict,
                            format!("duplicate `# bind={b}` (capture names must be unique)"),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .with_repair(StructuredError::fix_name_repair(
                            b,
                            &bind_to_hunk.keys().cloned().collect::<Vec<_>>(),
                        ))
                        .into());
                    }
                    // WI-6: `# bind=` + `# allow=many` on an item-capture cut
                    // hunk binds an ordered collection (SPEC 20.3); the kind
                    // gate lives in the engine (non-item binds stay loud).
                    bind_to_hunk.insert(b.to_string(), (f.target_path.clone(), rh.rule_name()));
                }
                for u in rh.uses() {
                    use_refs.push((u.to_string(), f.target_path.clone(), rh.rule_name()));
                }
                // Hunk with neither - nor + nor context? Already rejected empty.
                // use without bind resolved at engine stage (forward refs allowed),
                // but unknown refs are reported there with full-patch view.
                let _ = &use_refs;
            }
        }
    }
    // Cycle check placeholder: single bind->use edges cannot cycle by
    // construction in v0.1 (uses don't define new values), so any cycle
    // would require bind depending on use of itself. Detect self-dependency:
    // hunk with both bind=X and use=X is suspicious but allowed only if
    // independent. We reject direct self-use to keep v0.1 simple.
    for f in &patch.files {
        for h in &f.hunks {
            if let Hunk::Rsmpl(rh) = h {
                let binds: HashSet<&str> = rh.binds().into_iter().collect();
                for u in rh.uses() {
                    if binds.contains(u) {
                        return Err(StructuredError::new(
                            ErrorCode::CaptureConflict,
                            format!("hunk both binds and uses `{u}` (self-cycle)"),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .with_repair(StructuredError::fix_pattern_repair(
                            "bind",
                            "split the cut and the paste into two hunks",
                        ))
                        .into());
                    }
                }
                // T20: `# repeat` folds nested matches into a rewrite hunk.
                // It is meaningless on captures (verbatim moves) and on
                // insertions/deletions (which already cover nested matches).
                if rh.repeats() {
                    if !rh.binds().is_empty() || !rh.uses().is_empty() {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            "`# repeat` cannot be combined with `# bind=`/`# use=` (captures move verbatim)".to_string(),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .with_suggestion("remove `# repeat` from the cut/paste hunk".to_string())
                        .with_repair(StructuredError::fix_pattern_repair(
                            "repeat",
                            "remove `# repeat` from the cut/paste hunk",
                        ))
                        .into());
                    }
                    if rh.minus_text().trim().is_empty() || rh.plus_text().trim().is_empty() {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            "`# repeat` requires both `-` and `+` patterns (insertions/deletions already cover nested matches)".to_string(),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .with_suggestion("remove `# repeat`, or write a rewrite hunk".to_string())
                        .with_repair(StructuredError::fix_pattern_repair(
                            "repeat",
                            "remove `# repeat`, or write a rewrite hunk",
                        ))
                        .into());
                    }
                }
            }
        }
    }
    Ok(())
}
