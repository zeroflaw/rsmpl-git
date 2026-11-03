//! WI-20 slice 1: rename+single-expr-body joint application (D5 —
//! `matcher_fn.rs` crossed the 2000-line gate; tests stay inline in the
//! owner per WI-22).

use rsmpl_patch::{ErrorCode, RsmplHunk, StructuredError};
use syn::spanned::Spanned;

use crate::edits::TextEdit;
use crate::lines::LineIndex;

use super::matcher_fn::{check_attrs, collect_fns, in_scope, minus_matches_strict, plan_sig_edits};

/// WI-20 slice 1 + WI-13 slices 1–2: apply a rename+body-pair joint hunk.
///
/// The header pair renames exactly like a plain fn rename (same matching
/// gates); each body pair rewrites one whole line inside the renamed
/// definition's body only (edge-preserving indent, `body`-labelled
/// edit per pair). A body minus occurring zero times, more than once,
/// or outside the renamed body is a hard `RSMPL_MATCH_CONFLICT` —
/// never a sweep.
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
        for (body_minus, body_plus) in &joint.bodies {
            new_edits.push(joint_body_edit(
                original, index, c.item, body_minus, body_plus, rule, &range_err,
            )?);
        }
        edits.append(&mut new_edits);
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break; // count validation reports the error
        }
    }
    Ok(matched)
}

/// Locate one joint body minus as exactly one whole line inside the
/// renamed `ItemFn` block and build the indent-preserving `body` edit.
fn joint_body_edit(
    original: &str,
    index: &LineIndex,
    item: &syn::ItemFn,
    body_minus: &str,
    body_plus: &str,
    rule: &str,
    range_err: &dyn Fn(u32) -> StructuredError,
) -> Result<TextEdit, StructuredError> {
    let line_count = original.lines().count() as u32;
    let mut hits: Vec<u32> = Vec::new();
    for n in 1..=line_count {
        let ls = index.line_start(n).ok_or_else(|| range_err(n))?;
        let le = index.line_start(n + 1).unwrap_or(original.len());
        let raw = &original[ls..le];
        if raw.trim() == body_minus {
            hits.push(n);
        }
    }
    if hits.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "joint body line `{body_minus}` not found in the file (the body minus must occur exactly once as a whole line)"
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
                "joint body line `{body_minus}` matches {} lines (exactly one whole-line occurrence required)",
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
                "joint body line `{body_minus}` occurs outside the renamed function body (the body pair applies only inside that body)"
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
    let end = start + body_minus.len();
    Ok(TextEdit::new(start, end, body_plus.to_string()).with_label("body"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files["src/lib.rs"].clone())
    }

    #[test]
    fn wi13_joint_body_three_pairs_apply_scoped() {
        // WI-13 slice 3: header + three body pairs (two same-binder
        // `let`s + one expr) apply as three span-scoped `body` edits
        // plus the follow calls.
        let pre =
            "fn foo(x: i32) -> i32 {\n    let y = 1;\n    let z = y + 1;\n    z + 1\n}\nfn m() {\n    foo(1);\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=j\n# lang=rust\n# sweep\n# follow=uses\n- fn foo(x: i32) -> i32 { ... }\n+ fn bar(x: i32) -> i32 { ... }\n- let y = 1;\n+ let y = 2;\n- let z = y + 1;\n+ let z = y + 2;\n- z + 1\n+ z + 2\n";
        let out = apply(patch, pre).expect("three-pair joint must apply");
        assert_eq!(
            out,
            "fn bar(x: i32) -> i32 {\n    let y = 2;\n    let z = y + 2;\n    z + 2\n}\nfn m() {\n    bar(1);\n}\n",
            "three-pair joint must rename header, all three body lines, and follow calls"
        );
    }

    #[test]
    fn wi13_joint_body_three_pairs_third_absent_fails_loud() {
        // Per-pair gate: a missing third minus fails loud even when
        // the first two pairs are present.
        let pre =
            "fn foo(x: i32) -> i32 {\n    let y = 1;\n    let z = y + 1;\n    z + 1\n}\nfn m() {\n    foo(1);\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=j\n# lang=rust\n# sweep\n# follow=uses\n- fn foo(x: i32) -> i32 { ... }\n+ fn bar(x: i32) -> i32 { ... }\n- let y = 1;\n+ let y = 2;\n- let z = y + 1;\n+ let z = y + 2;\n- z + 9\n+ z + 2\n";
        let err = apply(patch, pre).expect_err("absent third pair must fail");
        assert!(err.contains("not found in the file"), "got: {err}");
        assert!(err.contains("fn-joint-body-absent-template"), "got: {err}");
    }
}
