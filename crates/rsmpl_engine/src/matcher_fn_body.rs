//! WI-20 slice 1: rename+single-expr-body joint application (D5 —
//! `matcher_fn.rs` crossed the 2000-line gate; tests stay inline in the
//! owner per WI-22).

use rsmpl_patch::{ErrorCode, RsmplHunk, StructuredError};
use syn::spanned::Spanned;

use crate::edits::TextEdit;
use crate::lines::LineIndex;

use super::matcher_fn::{check_attrs, collect_fns, in_scope, minus_matches_strict, plan_sig_edits};

/// WI-20 slice 1: apply a rename+single-expr-body joint hunk.
///
/// The header pair renames exactly like a plain fn rename (same matching
/// gates); the body pair rewrites one whole line inside the renamed
/// definition's body only (edge-preserving indent, `body`-labelled
/// edit). A body minus occurring zero times, more than once, or outside
/// the renamed body is a hard `RSMPL_MATCH_CONFLICT` — never a sweep.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_fn_body_joint(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    joint: &rsmpl_pattern::FnBodyJoint,
    minus_name: &str,
    plus_name: &str,
    rule: &str,
    file: &str,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let range_err = |line: u32| {
        StructuredError::new(
            ErrorCode::RangeConflict,
            format!("span line {line} outside file"),
        )
        .with_rule(rule)
        .with_repair(StructuredError::internal_repair())
    };
    let mut matched = 0;
    for c in collect_fns(parsed) {
        if !in_scope(&c, hunk.scope(), hunk.header.old_start, hunk.header.old_len) {
            continue;
        }
        if c.item.sig.ident != minus_name {
            continue;
        }
        if !joint.minus.matches_item_fn(c.item) {
            continue;
        }
        if !minus_matches_strict(&joint.minus, &c.item.vis, &c.item.sig, original, index) {
            continue;
        }
        if hunk.returns_none() && !matches!(c.item.sig.output, syn::ReturnType::Default) {
            continue;
        }
        // Written attributes constrain like any other slot mismatch: a
        // site lacking them is a non-match (the joint helper admits no
        // written attributes, so the empty gate preserves by default and
        // `# attrs=absent` still rejects attributed targets).
        check_attrs(&c, hunk.attrs_policy(), &[], rule, file)?;
        let mut new_edits = plan_sig_edits(
            &c.item.vis,
            &c.item.sig,
            &joint.minus,
            &joint.plus,
            minus_name,
            plus_name,
            original,
            index,
            rule,
            false,
        )?;
        new_edits.push(joint_body_edit(
            original, index, c.item, joint, rule, &range_err,
        )?);
        edits.append(&mut new_edits);
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break; // count validation reports the error
        }
    }
    Ok(matched)
}

/// Locate the joint body minus as exactly one whole line inside the
/// renamed `ItemFn` block and build the indent-preserving `body` edit.
fn joint_body_edit(
    original: &str,
    index: &LineIndex,
    item: &syn::ItemFn,
    joint: &rsmpl_pattern::FnBodyJoint,
    rule: &str,
    range_err: &dyn Fn(u32) -> StructuredError,
) -> Result<TextEdit, StructuredError> {
    let line_count = original.lines().count() as u32;
    let mut hits: Vec<u32> = Vec::new();
    for n in 1..=line_count {
        let ls = index.line_start(n).ok_or_else(|| range_err(n))?;
        let le = index.line_start(n + 1).unwrap_or(original.len());
        let raw = &original[ls..le];
        if raw.trim() == joint.body_minus {
            hits.push(n);
        }
    }
    if hits.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "joint body line `{}` not found in the file (the body minus must occur exactly once as a whole line)",
                joint.body_minus
            ),
        )
        .with_rule(rule)
        .with_repair(StructuredError::fix_pattern_repair(
            "fn-joint-body-absent-template",
            "write the body minus line exactly as it appears in the file (one whole line; leading indent is ignored)",
        )));
    }
    if hits.len() > 1 {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "joint body line `{}` matches {} lines (exactly one whole-line occurrence required)",
                joint.body_minus,
                hits.len()
            ),
        )
        .with_rule(rule)
        .with_repair(StructuredError::fix_pattern_repair(
            "fn-joint-body-multi-template",
            "narrow the body pair so the minus line occurs exactly once in the file (whole-line match), or split the change into ordinary hunks",
        )));
    }
    let span = item.block.span();
    let (b_first, b_last) = (span.start().line as u32, span.end().line as u32);
    if hits[0] <= b_first || hits[0] >= b_last {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "joint body line `{}` occurs outside the renamed function body (the body pair applies only inside that body)",
                joint.body_minus
            ),
        )
        .with_rule(rule)
        .with_repair(StructuredError::fix_pattern_repair(
            "fn-joint-body-outside-template",
            "the body pair applies only inside the renamed definition's body — move the minus line into that body or use an ordinary hunk",
        )));
    }
    let ls = index
        .line_start(hits[0])
        .ok_or_else(|| range_err(hits[0]))?;
    let le = index.line_start(hits[0] + 1).unwrap_or(original.len());
    let raw = &original[ls..le];
    let indent = raw.len() - raw.trim_start().len();
    let start = ls + indent;
    let end = start + joint.body_minus.len();
    Ok(TextEdit::new(start, end, joint.body_plus.clone()).with_label("body"))
}
