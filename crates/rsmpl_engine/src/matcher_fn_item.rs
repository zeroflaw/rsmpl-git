//! WI-18 slice 11 fn item-delete/insertion pass (split from
//! `matcher_fn.rs` per D5 — the parent file sits over the 1600-line gate).
//!
//! Item-capture deletion (`- I`, SPEC 18.2) and literal insertion
//! (SPEC 18.3, 10.5) plus their line-span target machinery. The parent
//! keeps the `matcher_fn::` paths as re-exports so existing callers
//! (`plan_dispatch`, `plan_cut`, `capture`, `matcher_general_apply`,
//! `matcher_sequence`) are untouched.

use syn::spanned::Spanned;

use rsmpl_patch::{ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind};

use super::matcher_fn::{collect_fns, in_scope};
use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// Find item targets (function or other items) for an item-capture hunk.
///
/// v0.1 matches top-level items, fns nested in inline `mod`s, and
/// `impl`/`trait` methods (T466) by line-span containment; only functions
/// are structurally verified elsewhere.
pub(crate) fn find_item_targets(
    parsed: &syn::File,
    scope: Scope,
    old_start: u32,
    old_len: u32,
) -> Vec<(u32, u32)> {
    let mut candidates: Vec<(u32, u32)> = Vec::new();
    for c in collect_fns(parsed) {
        if in_scope(&c, scope, old_start, old_len) {
            candidates.push((c.start_line, c.end_line));
        }
    }
    candidates.extend(collect_non_fn_item_spans(parsed, scope, old_start, old_len));
    collect_impl_method_spans(&parsed.items, scope, old_start, old_len, &mut candidates);
    candidates.sort();
    candidates.dedup();
    candidates
}

/// Delete the single matched item for a bare `- I` hunk (SPEC 18.2).
pub fn apply_item_delete(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    match &compiled.kind {
        PatternKind::ItemCapture { .. } => {}
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected item capture pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "fn-dispatch-capture-template",
                "write a bare `- I` item-capture hunk (a single `- I` line; see SPEC 18.2)",
            )));
        }
    }
    let rule = hunk.rule_name();
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let candidates = find_item_targets(parsed, hunk.scope(), old_start, old_len);
    if candidates.is_empty() {
        return Ok(0);
    }
    // allow=one requires exactly one; for deletion, `many` deletes all
    // contained items.
    let targets: Vec<(u32, u32)> = match hunk.allow() {
        rsmpl_patch::Allow::One => vec![candidates[0]],
        rsmpl_patch::Allow::Many => candidates,
    };
    let n = targets.len();
    for (sl, el) in targets {
        edits.push(delete_lines(original, index, sl, el, &rule)?);
    }
    Ok(n)
}

/// Whole-line deletion edit from `start_line` through `end_line` (inclusive),
/// swallowing the trailing newline.
///
/// To avoid leaving stray blank separator lines, the range additionally
/// absorbs one adjacent blank line: the following blank line when one exists,
/// otherwise one preceding blank line at EOF. Comments and code are never
/// absorbed — only a fully blank line.
pub(crate) fn delete_lines(
    original: &str,
    index: &LineIndex,
    start_line: u32,
    end_line: u32,
    rule: &str,
) -> Result<TextEdit, StructuredError> {
    let mut start = index
        .line_start(start_line)
        .ok_or_else(|| range_err(rule, start_line))?;
    // End at the start of the following line (this swallows the trailing
    // newline); at EOF use EOF.
    let mut end = index
        .line_start(end_line + 1)
        .unwrap_or_else(|| index.eof());
    if end < original.len() {
        // Absorb one following blank line (a lone `\n` right at `end`).
        if original.as_bytes().get(end) == Some(&b'\n') {
            end += 1;
        }
    } else if start > 0 {
        // At EOF: absorb one preceding blank line.
        let before = &original[..start];
        if before.ends_with("\n\n") {
            // Move `start` back over the blank line.
            let trimmed = &before[..before.len() - 1];
            if let Some(pos) = trimmed.rfind('\n') {
                start = pos + 1;
            } else {
                start = 0;
            }
        }
    }
    Ok(TextEdit::new(start, end, String::new()))
}

/// Non-function item spans (struct/enum/mod/use/...) via syn item spans.
fn collect_non_fn_item_spans(
    parsed: &syn::File,
    scope: Scope,
    old_start: u32,
    old_len: u32,
) -> Vec<(u32, u32)> {
    let end = old_start + old_len.saturating_sub(1);
    let mut out = Vec::new();
    for it in &parsed.items {
        if matches!(it, syn::Item::Fn(_)) {
            continue;
        }
        let s = it.span().start().line as u32;
        let e = it.span().end().line as u32;
        let inside = match scope {
            Scope::File => true,
            Scope::Contained => s >= old_start && e <= end.max(old_start),
            Scope::Anchor => s >= old_start && s <= end.max(old_start),
            Scope::Overlap => s <= end.max(old_start) && e >= old_start,
            Scope::Exact => old_len > 0 && s == old_start && e == end,
        };
        if inside {
            out.push((s, e));
        }
    }
    out
}

/// Associated-function spans inside `impl`/`trait` blocks (T466): top-level
/// and inline-`mod`-nested. `Fn`-items only — the nested-deletion trigger
/// shape; assoc consts/types stay ordinary (no corpus trigger).
fn collect_impl_method_spans(
    items: &[syn::Item],
    scope: Scope,
    old_start: u32,
    old_len: u32,
    out: &mut Vec<(u32, u32)>,
) {
    let end = old_start + old_len.saturating_sub(1);
    let push = |out: &mut Vec<(u32, u32)>, s: u32, e: u32| {
        let inside = match scope {
            Scope::File => true,
            Scope::Contained => s >= old_start && e <= end.max(old_start),
            Scope::Anchor => s >= old_start && s <= end.max(old_start),
            Scope::Overlap => s <= end.max(old_start) && e >= old_start,
            Scope::Exact => old_len > 0 && s == old_start && e == end,
        };
        if inside {
            out.push((s, e));
        }
    };
    for it in items {
        match it {
            syn::Item::Impl(imp) => {
                for item in &imp.items {
                    if let syn::ImplItem::Fn(m) = item {
                        let s = m.span().start().line as u32;
                        let e = m.span().end().line as u32;
                        push(out, s, e);
                    }
                }
            }
            syn::Item::Trait(t) => {
                for item in &t.items {
                    if let syn::TraitItem::Fn(f) = item {
                        let s = f.span().start().line as u32;
                        let e = f.span().end().line as u32;
                        push(out, s, e);
                    }
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_impl_method_spans(inner, scope, old_start, old_len, out);
                }
            }
            _ => {}
        }
    }
}

/// Insert literal text at a zero-length anchor (SPEC 18.3, 10.5).
pub fn apply_insertion(
    _original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (text, _use_name) = match &compiled.kind {
        PatternKind::Insertion { text, use_name } => (text.clone(), use_name.clone()),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected insertion pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "fn-dispatch-insertion-template",
                "write a literal insertion hunk (`@@ -N,0 @@` with `+` lines only; see SPEC 18.3)",
            )));
        }
    };
    let anchor = hunk.header.old_start;
    let pos = if anchor == 0 {
        0
    } else {
        index.line_start(anchor + 1).unwrap_or_else(|| index.eof())
    };
    // Ensure the inserted text ends with a newline so it forms whole lines.
    let mut ins = text;
    if !ins.ends_with('\n') {
        ins.push('\n');
    }
    edits.push(TextEdit::new(pos, pos, ins));
    Ok(1)
}

pub(crate) fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_repair(StructuredError::internal_repair())
}

#[cfg(test)]
mod tests {
    fn wi18_fn_item_setup(
        patch_text: &str,
        src: &str,
    ) -> (
        rsmpl_patch::RsmplHunk,
        rsmpl_pattern::CompiledPattern,
        syn::File,
        crate::lines::LineIndex,
        String,
    ) {
        let mut parsed_patch = rsmpl_patch::parse_patch(patch_text).expect("test patch parses");
        let hunk = match parsed_patch.files.remove(0).hunks.remove(0) {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&hunk).expect("patch compiles");
        let src = src.to_string();
        let parsed: syn::File = syn::parse_str(&src).expect("src parses");
        let index = crate::lines::LineIndex::new(&src);
        (hunk, compiled, parsed, index, src)
    }

    fn wi18_fn_item_repair(err: &rsmpl_patch::StructuredError) -> (String, String) {
        let repair = err
            .repair
            .as_ref()
            .expect("WI-18 slice 11: must carry repair");
        let code = repair
            .get("code")
            .and_then(|v| v.as_str())
            .expect("code str")
            .to_string();
        let write_instead = repair
            .get("write_instead")
            .and_then(|v| v.as_str())
            .expect("template str")
            .to_string();
        assert_ne!(code, "pattern", "must beat the generic filler");
        assert!(
            !write_instead.contains(&err.message),
            "repair must not echo the reason, got: {write_instead}"
        );
        (code, write_instead)
    }

    const DELETE: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=del\n# lang=rust\n# item I\n- I\n";

    #[test]
    fn wi18_slice11_item_delete_dispatch_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_fn_item_setup(DELETE, "fn foo() {}\n");
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err = super::apply_item_delete(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected item capture pattern");
        let (code, w) = wi18_fn_item_repair(&err);
        assert_eq!(code, "fn-dispatch-capture-template");
        assert!(w.contains("`- I`"), "got: {w}");
    }

    const INSERT: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=ins\n# lang=rust\n+fn foo() {}\n";

    #[test]
    fn wi18_slice11_insertion_dispatch_repair_is_specific() {
        let (hunk, mut compiled, _parsed, index, src) = wi18_fn_item_setup(INSERT, "fn foo() {}\n");
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err = super::apply_insertion(&src, &index, &hunk, &compiled, &mut edits)
            .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected insertion pattern");
        let (code, w) = wi18_fn_item_repair(&err);
        assert_eq!(code, "fn-dispatch-insertion-template");
        assert!(w.contains("@@ -N,0 @@"), "got: {w}");
    }
}
