//! WI-5c repeated-block statement sequences: match K consecutive `Block`
//! statements against K plain-`let` pattern statements with shared
//! bindings, replace them with the K plus-side templates instantiated
//! under the same bindings.
//!
//! Matching is a sliding window over every `syn::Block` in the target
//! file (function bodies and nested blocks; file top-level items are
//! not `Block` members, so sequences never span items). Each window
//! position matches with the existing structural `match_local`; the
//! `MatchCx` is shared across the K positions, so a repeated hole
//! (`P0` in two statements) must bind structurally-equal text on both
//! (SPEC 15.7 cross-statement correspondence) — exactly like repeated
//! holes inside one statement. Windows never overlap (SPEC 17.6
//! posture): a window contained in an accepted span skips, a partial
//! overlap fails loud.
//!
//! Replacement preserves each target line's indent: plus templates are
//! single-line lets (compile gate), instantiated per position and
//! prefixed with that position's original indent.

use std::cmp::Reverse;
use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::edits::TextEdit;
use crate::lines::LineIndex;

use super::matcher_fn::delete_lines;
use super::matcher_general::{in_scope, MatchCx};
use super::matcher_general_ctrl::match_local;
use super::matcher_general_tails::substitute_general_plus;

/// One `syn::Block` with its byte range (for outer-first ordering).
struct SeqBlock<'ast> {
    node: &'ast syn::Block,
    start: usize,
    end: usize,
}

struct BlockCollector<'ast, 'b> {
    index: &'b LineIndex,
    blocks: Vec<SeqBlock<'ast>>,
}

impl<'ast, 'b> BlockCollector<'ast, 'b> {
    fn range_of(&self, span: proc_macro2::Span) -> Option<(usize, usize)> {
        let s = span.start();
        let e = span.end();
        let start = self.index.offset(s.line as u32, s.column)?;
        let end = self.index.offset(e.line as u32, e.column)?;
        Some((start, end))
    }
}

impl<'ast, 'b> Visit<'ast> for BlockCollector<'ast, 'b> {
    fn visit_block(&mut self, node: &'ast syn::Block) {
        if let Some((start, end)) = self.range_of(node.span()) {
            self.blocks.push(SeqBlock { node, start, end });
        }
        visit::visit_block(self, node);
    }
}

/// Apply a WI-5c `GeneralShape::Sequence` hunk. Returns the window-match
/// count plus per-window bindings (shared map per window, in span order).
#[allow(clippy::too_many_lines)]
pub fn apply_sequence_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (minus_sub, plus_raw, placeholders, sigiled) = match &compiled.kind {
        rsmpl_pattern::PatternKind::General {
            shape: rsmpl_pattern::GeneralShape::Sequence,
            minus_sub,
            plus_raw,
            placeholders,
            ..
        } => (
            minus_sub.clone(),
            plus_raw.clone(),
            placeholders.clone(),
            compiled.table.sigiled,
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected statement-sequence pattern".to_string(),
            ))
        }
    };
    let rule = hunk.rule_name();
    let reverse: HashMap<String, String> =
        placeholders.into_iter().map(|(n, ph)| (ph, n)).collect();

    let minus_block = syn::parse_str::<syn::Block>(&format!("{{ {minus_sub} }}")).map_err(|e| {
        StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid statement-sequence pattern: {e}"),
        )
        .with_rule(&rule)
    })?;
    let mut pat_locals: Vec<syn::Local> = Vec::with_capacity(minus_block.stmts.len());
    for stmt in &minus_block.stmts {
        match stmt {
            syn::Stmt::Local(l) => pat_locals.push(l.clone()),
            _ => {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "statement-sequence members must be plain `let` statements in v1".to_string(),
                )
                .with_rule(&rule))
            }
        }
    }
    let k = pat_locals.len();
    if !(2..=4).contains(&k) {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "statement-sequence members must be plain `let` statements in v1".to_string(),
        )
        .with_rule(&rule));
    }
    let deleting = plus_raw.trim().is_empty();
    let plus_lines: Vec<String> = if deleting {
        Vec::new()
    } else {
        // Artifact-stripped plus body (one-space diff-prefix artifact per
        // line, SPEC T153): the compile gate guarantees K single-line
        // plain lets, so a line split recovers the per-position
        // templates.
        let body: Vec<String> = plus_raw
            .lines()
            .map(|l| l.strip_prefix(' ').unwrap_or(l).to_string())
            .collect();
        // Drop leading `#[...]` attribute lines (none in v1 sequences —
        // the compile gate rejects attrs — but keep the Stmt shape
        // posture: split, don't guess).
        let body: Vec<String> = body
            .into_iter()
            .skip_while(|l| l.trim_start().starts_with("#["))
            .collect();
        if body.len() != k {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "statement-sequence plus side must hold {k} single-line `let` statements (got {})",
                    body.len()
                ),
            )
            .with_rule(&rule));
        }
        body
    };

    let mut collector = BlockCollector {
        index,
        blocks: Vec::new(),
    };
    collector.visit_file(parsed);
    // Outer-first on shared starts (outermost wins); disjoint blocks keep
    // file order via the stable sort.
    collector.blocks.sort_by_key(|b| (b.start, Reverse(b.end)));

    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut matched = 0;
    let mut all_bindings: Vec<HashMap<String, String>> = Vec::new();

    for block in &collector.blocks {
        let stmts = &block.node.stmts;
        if stmts.len() < k {
            continue;
        }
        for start in 0..=(stmts.len() - k) {
            let window = &stmts[start..start + k];
            // Every window member must be a plain `let` (the pattern can
            // only match locals); anything else skips the window.
            let mut tgt_locals: Vec<&syn::Local> = Vec::with_capacity(k);
            let mut ok = true;
            for stmt in window {
                match stmt {
                    syn::Stmt::Local(l) => tgt_locals.push(l),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            let mut cx = MatchCx::with_reverse(original, index, reverse.clone());
            for (pat, tgt) in pat_locals.iter().zip(tgt_locals.iter()) {
                if !match_local(pat, tgt, &mut cx) {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            // Window byte span: first statement start through last
            // statement end (attribute envelopes excluded — sequences
            // carry no attrs in v1).
            let (wstart, wend, wsline, weline) = match window_span(index, original, window) {
                Some(v) => v,
                None => continue,
            };
            if !in_scope(
                wsline,
                weline,
                hunk.scope(),
                hunk.header.old_start,
                hunk.header.old_len,
            ) {
                continue;
            }
            if accepted.iter().any(|&(s, e)| s <= wstart && wend <= e) {
                continue;
            }
            if deleting {
                edits.push(delete_lines(original, index, wsline, weline, &rule)?);
            } else {
                // The window span starts at the first statement (the
                // target line's own indent is outside the span and
                // rides verbatim, like the single-statement arms), so
                // only continuation lines re-apply their indent after
                // the emitted newline.
                let mut out = String::new();
                for (j, plus) in plus_lines.iter().enumerate() {
                    if j > 0 {
                        out.push('\n');
                        out.push_str(indent_of(index, original, window[j].span()));
                    }
                    out.push_str(&substitute_general_plus(plus, &cx.bindings, &[], sigiled)?);
                }
                edits.push(TextEdit::new(wstart, wend, out));
            }
            for &(s, e) in &accepted {
                if wstart < e && s < wend && !(s <= wstart && wend <= e) {
                    return Err(crate::validate::partial_overlap_error(&rule));
                }
            }
            accepted.push((wstart, wend));
            all_bindings.push(cx.bindings.clone());
            matched += 1;
            if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                break;
            }
        }
    }
    Ok((matched, all_bindings))
}

/// Byte + line span of a statement window (first-statement start to
/// last-statement end, newlines excluded — same convention as the
/// single-statement arms).
fn window_span(
    index: &LineIndex,
    original: &str,
    window: &[syn::Stmt],
) -> Option<(usize, usize, u32, u32)> {
    let first = window.first()?;
    let last = window.last()?;
    let s = first.span().start();
    let e = last.span().end();
    // A trailing `;` may sit outside an expression-statement span:
    // extend through it so the replacement keeps its terminator.
    let mut end_lc = e;
    if let syn::Stmt::Expr(_, Some(semi)) = last {
        let se = semi.span.end();
        if (se.line as u32, se.column) > (end_lc.line as u32, end_lc.column) {
            end_lc = se;
        }
    }
    let start = index.offset(s.line as u32, s.column)?;
    let end = index.offset(end_lc.line as u32, end_lc.column)?;
    if start > end || end > original.len() {
        return None;
    }
    Some((start, end, s.line as u32, end_lc.line as u32))
}

/// Original indent (line start through statement start) of one target
/// statement; empty when spans fall outside the line index.
fn indent_of<'a>(index: &LineIndex, original: &'a str, span: proc_macro2::Span) -> &'a str {
    let s = span.start();
    let line_start = match index.line_start(s.line as u32) {
        Some(v) => v,
        None => return "",
    };
    let stmt_start = match index.offset(s.line as u32, s.column) {
        Some(v) => v,
        None => return "",
    };
    if line_start > stmt_start || stmt_start > original.len() {
        return "";
    }
    original.get(line_start..stmt_start).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rsmpl_patch::parse_patch;

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        let patch = parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files["src/lib.rs"].clone())
    }

    const HEAD: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";

    const TWO_BLOCKS: &str = "fn first() {\n    let alpha = compute_first(input);\n    let beta = wrap(alpha);\n    let gamma = finish(beta);\n}\n\nfn second() {\n    let delta = compute_second(other);\n    let epsilon = wrap(delta);\n    let zeta = finish(epsilon);\n}\n";

    fn seq_patch() -> String {
        format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- let P1 = E1;\n- let P2 = E2;\n+ let P0: u32 = E0;\n+ let P1: u32 = E1;\n+ let P2: u32 = E2;\n"
        )
    }

    #[test]
    fn sequence_rewrites_both_windows() {
        let out = apply(&seq_patch(), TWO_BLOCKS).unwrap();
        assert!(
            out.contains("let alpha: u32 = compute_first(input);"),
            "got: {out}"
        );
        assert!(out.contains("let beta: u32 = wrap(alpha);"), "got: {out}");
        assert!(out.contains("let gamma: u32 = finish(beta);"), "got: {out}");
        assert!(
            out.contains("let delta: u32 = compute_second(other);"),
            "got: {out}"
        );
        assert!(
            out.contains("let epsilon: u32 = wrap(delta);"),
            "got: {out}"
        );
        assert!(
            out.contains("let zeta: u32 = finish(epsilon);"),
            "got: {out}"
        );
    }

    #[test]
    fn sequence_zero_match_fails_loud() {
        // No plain-`let` windows at all: the sweep fails instead of
        // silently rewriting nothing.
        let src = "fn f() {\n    оказывается_call();\n}\n";
        assert!(apply(&seq_patch(), src).is_err());
    }

    #[test]
    fn sequence_second_window_mismatch_rewrites_first_only() {
        // `allow=many` is per-window: a structurally divergent second
        // block simply does not match (the detector's totality probe
        // would have vetoed the emission; the engine stays local).
        let src = "fn first() {\n    let alpha = compute_first(input);\n    let beta = wrap(alpha);\n    let gamma = finish(beta);\n}\n\nfn second() {\n    let delta = compute_second(other);\n    let epsilon = wrap(delta);\n    done(epsilon);\n}\n";
        let out = apply(&seq_patch(), src).unwrap();
        assert!(
            out.contains("let alpha: u32 = compute_first(input);"),
            "got: {out}"
        );
        assert!(out.contains("    done(epsilon);"), "got: {out}");
        assert!(!out.contains("let delta: u32"), "got: {out}");
    }

    #[test]
    fn sequence_overlapping_windows_fail_loud() {
        // Three consecutive matching lets with K = 2: the second window
        // partially overlaps the first — SPEC 17.6 posture, loud error
        // (the detector's probe counts the shifted twin and vetoes, so
        // this only fires on hand-authored over-broad sweeps).
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- let P1 = E1;\n+ let P0: u32 = E0;\n+ let P1: u32 = E1;\n"
        );
        let src = "fn f() {\n    let a = g(1);\n    let b = g(2);\n    let c = g(3);\n}\n";
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn sequence_deletion_removes_windows() {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- let P1 = E1;\n- let P2 = E2;\n"
        );
        let out = apply(&patch, TWO_BLOCKS).unwrap();
        assert!(!out.contains("compute_first"), "got: {out}");
        assert!(!out.contains("compute_second"), "got: {out}");
        assert!(out.contains("fn first() {\n}"), "got: {out}");
    }

    #[test]
    fn sequence_plus_count_mismatch_is_c18() {
        // Minus K = 2 with a 3-line plus side: compile-time C18, never a
        // silent partial rewrite.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- let P1 = E1;\n+ let P0: u32 = E0;\n+ let P1: u32 = E1;\n+ let P2: u32 = E2;\n"
        );
        let err = apply(&patch, TWO_BLOCKS).unwrap_err();
        assert!(err.contains("C18"), "got: {err}");
    }

    #[test]
    fn sequence_matches_inside_nested_blocks() {
        // Windows enumerate every `Block`, including bodies nested in
        // control flow — a repeated pair inside two `if` bodies rewrites.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- let P1 = E1;\n+ let P0: u32 = E0;\n+ let P1: u32 = E1;\n"
        );
        let src = "fn f(c: bool, d: bool) {\n    if c {\n        let a = g(1);\n        let b = g(2);\n    }\n    if d {\n        let e = g(3);\n        let h = g(4);\n    }\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let a: u32 = g(1);"), "got: {out}");
        assert!(out.contains("let e: u32 = g(3);"), "got: {out}");
    }

    #[test]
    fn sequence_ellipsis_is_c1() {
        // `...` has no trailing-block position inside a sequence member.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- ...\n+ let P0: u32 = E0;\n"
        );
        let err = apply(&patch, TWO_BLOCKS).unwrap_err();
        assert!(err.contains("C1"), "got: {err}");
    }
}
