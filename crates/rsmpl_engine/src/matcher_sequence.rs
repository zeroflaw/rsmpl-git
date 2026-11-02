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
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "sequence-template",
                "write 2-4 single-line plain `let` statements on the `-` side, e.g. `- let P0 = E0;` / `- let P1 = E1;` with a matching `+` side (empty `+` deletes); sequences never span items",
            )));
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
        .with_repair(StructuredError::fix_pattern_repair(
            "sequence-let-syntax",
            "write valid Rust `let` statements on the `-` side, e.g. `let P0 = E0;` (2-4 single-line plain lets, no `if`/`for`/`return`)",
        ))
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
                .with_rule(&rule)
                .with_repair(StructuredError::fix_pattern_repair(
                    "sequence-let-only",
                    "write only plain `let` statements (no `if`/`for`/`return`/items), 2-4 lines, one per line",
                )))
            }
        }
    }
    let k = pat_locals.len();
    if !(2..=4).contains(&k) {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "statement-sequence members must be plain `let` statements in v1".to_string(),
        )
        .with_rule(&rule)
        .with_repair(StructuredError::fix_pattern_repair(
            "sequence-let-only",
            "write only plain `let` statements (no `if`/`for`/`return`/items), 2-4 lines, one per line",
        )));
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
            .with_rule(&rule)
            .with_repair(StructuredError::fix_pattern_repair(
                "sequence-plus-count",
                "write exactly K single-line `let` statements on the `+` side (same count as `-`, empty `+` deletes the window)",
            )));
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
                let follows =
                    sequence_binder_follows(parsed, original, index, compiled, &tgt_locals, wend);
                if follows.is_some() {
                    edits.push(TextEdit::new(wstart, wend, out).with_label("rename"));
                } else {
                    edits.push(TextEdit::new(wstart, wend, out));
                }
                edits.extend(follows.unwrap_or_default());
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

/// Sequence-contained binder completion (residue slices 1-3):
/// when the compiled sequence renames exactly one `let` binder
/// (plain or single-leaf destructuring, with optional authored
/// init-substitution positions spelling later in-window uses with the
/// new name) at any single position, rewrite unshadowed post-window
/// value uses of the old ident (`follow`), reusing the audited `let`
/// walker verbatim (same shadow/nested-fn/macro/attribute discipline,
/// same shorthand expansion). The window replacement above is the
/// definition (labelled `rename`, parity with the single-`let` arm);
/// only strictly post-window edits are kept, so follows never overlap
/// the window. In-window uses replay verbatim from the plus side —
/// spelled-`new` substitution positions complete in-window by
/// authoring, spelled-`old` ones strand as orphans (honest `partial`).
///
/// Returns `None` when the pattern is not a rename-once binder rename,
/// when the target window does not bind `old` exactly once at exactly
/// one position, or when another window position binds `new`
/// (window-local capture — fail-safe: the window still applies and
/// `orphans` still file via `general_orphans`, never a mis-resolved
/// rewrite).
fn sequence_binder_follows(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    compiled: &rsmpl_pattern::CompiledPattern,
    tgt_locals: &[&syn::Local],
    wend: usize,
) -> Option<Vec<TextEdit>> {
    if !matches!(
        &compiled.kind,
        rsmpl_pattern::PatternKind::General {
            shape: rsmpl_pattern::GeneralShape::Sequence,
            ..
        }
    ) {
        return None;
    }
    let b = rsmpl_pattern::binder::binder_rename(&compiled.kind)?;
    if b.scope != rsmpl_pattern::binder::BinderScope::Let {
        return None;
    }
    // Exactly one window position may bind `old` (structurally walked,
    // so destructuring shadows resolve); all others must bind zero
    // times — the target-side mirror of the pattern rename-once gate.
    // No window position may bind `new`: the rename would capture it
    // (the renamed definition plus any spelled substitution would
    // double-bind).
    let mut hit: Option<usize> = None;
    for (i, local) in tgt_locals.iter().enumerate() {
        let mut offs = crate::use_graph_local::pat_old_offsets(&local.pat, &b.old, original, index);
        offs.sort_unstable();
        offs.dedup();
        if !offs.is_empty() {
            if offs.len() != 1 || hit.is_some() {
                return None;
            }
            if original.get(offs[0]..offs[0] + b.old.len()) != Some(b.old.as_str()) {
                return None;
            }
            hit = Some(i);
        }
        let mut news = crate::use_graph_local::pat_old_offsets(&local.pat, &b.new, original, index);
        news.sort_unstable();
        news.dedup();
        if !news.is_empty() {
            return None;
        }
    }
    let idx = hit?;
    // Scope-wide `new` audit (slice 3 remainder): a pre-existing `new`
    // binder in the window's own scope (same level or outer) collides
    // with the rename either way — the renamed definition shadows it
    // (silently rebinding its later uses) or it double-binds the scope.
    // Decline the arm like the window-local guard: the window still
    // applies verbatim, post-window uses strand as orphans.
    let (ws, _) = crate::matcher_item::span_bytes(original, index, tgt_locals[0].span())?;
    match crate::use_graph_local_orphan::enclosing_new_binder_defs(
        parsed,
        original,
        index,
        &b.new,
        (ws, wend),
    ) {
        Some(v) if v.is_empty() => {}
        _ => return None,
    }
    // The renamed statement's own span is the binder range (the
    // single-`let` arm's discipline: its init RHS names the outer scope
    // and is never completed).
    let (ss, se) = crate::matcher_item::span_bytes(original, index, tgt_locals[idx].span())?;
    let mut edits = crate::use_graph_local::let_binder_follow_edits(
        parsed, original, index, ss, se, &b.old, &b.new,
    );
    edits.retain(|e| e.start >= wend);
    Some(edits)
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

    fn rename_last_patch() -> String {
        format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let a = E0;\n- let old = E1;\n+ let a = E0;\n+ let new = E1;\n"
        )
    }

    #[test]
    fn sequence_final_binder_rename_completes_post_window_use() {
        // Residue slice 1: the final-position plain rename completes
        // post-window uses in-hunk (the definition is `rename`, the use
        // is `follow`).
        let src = "fn f() {\n    let a = g(1);\n    let old = g(2);\n    let c = old;\n}\n";
        let out = apply(&rename_last_patch(), src).unwrap();
        assert!(out.contains("let new = g(2);"), "got: {out}");
        assert!(out.contains("let c = new;"), "got: {out}");
        assert!(!out.contains("old"), "got: {out}");
    }

    #[test]
    fn sequence_final_binder_rename_respects_shadow() {
        // An inner `let old` owns the uses in its block: the outer
        // follow rewrites around it, never through it.
        let src = "fn f() {\n    let a = g(1);\n    let old = g(2);\n    let b = old;\n    {\n        let old = 9;\n        let c = old;\n    }\n    let d = old;\n}\n";
        let out = apply(&rename_last_patch(), src).unwrap();
        assert!(out.contains("let new = g(2);"), "got: {out}");
        assert!(out.contains("let b = new;"), "got: {out}");
        assert!(out.contains("let old = 9;"), "got: {out}");
        assert!(out.contains("let c = old;"), "got: {out}");
        assert!(out.contains("let d = new;"), "got: {out}");
    }

    #[test]
    fn sequence_final_binder_rename_skips_macro_use() {
        // Macro interiors never rewrite (they stay orphans via
        // `general_orphans`); the plain use still completes.
        let src =
            "fn f() {\n    let a = g(1);\n    let old = g(2);\n    println!(\"{}\", old);\n    let d = old;\n}\n";
        let out = apply(&rename_last_patch(), src).unwrap();
        assert!(out.contains("let new = g(2);"), "got: {out}");
        assert!(out.contains("println!(\"{}\", old);"), "got: {out}");
        assert!(out.contains("let d = new;"), "got: {out}");
    }

    #[test]
    fn sequence_non_final_rename_completes_post_window_use() {
        // Residue slice 2: rename-once at any position completes
        // post-window uses (the definition is `rename`, the use is
        // `follow`).
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n"
        );
        let src = "fn f() {\n    let old = g(1);\n    let b = g(2);\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let c = new;"), "got: {out}");
        assert!(!out.contains("old"), "got: {out}");
    }

    #[test]
    fn sequence_spelled_substitution_completes_in_window_and_after() {
        // Residue slice 3: the author spells the intra-window use with
        // the new name; the window applies verbatim (plus keeps
        // re-matching) and post-window uses complete too.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = old;\n+ let new = E0;\n+ let b = new;\n"
        );
        let src = "fn f() {\n    let old = g(1);\n    let b = old;\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let b = new;"), "got: {out}");
        assert!(out.contains("let c = new;"), "got: {out}");
        assert!(!out.contains("old"), "got: {out}");
    }

    #[test]
    fn sequence_destructuring_position_completes_post_window_use() {
        // Residue slice 3: single-leaf destructuring at a sequence
        // position completes post-window uses.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let (x, old) = E0;\n- let b = E1;\n+ let (x, new) = E0;\n+ let b = E1;\n"
        );
        let src = "fn f() {\n    let (x, old) = g();\n    let b = h();\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let (x, new) = g();"), "got: {out}");
        assert!(out.contains("let c = new;"), "got: {out}");
        assert!(!out.contains("old"), "got: {out}");
    }

    #[test]
    fn sequence_destructuring_with_spelled_substitution() {
        // Both new shapes compose: destructuring rename plus a spelled
        // intra-window substitution.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let (x, old) = E0;\n- let b = old;\n+ let (x, new) = E0;\n+ let b = new;\n"
        );
        let src = "fn f() {\n    let (x, old) = g();\n    let b = old;\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let (x, new) = g();"), "got: {out}");
        assert!(out.contains("let b = new;"), "got: {out}");
        assert!(out.contains("let c = new;"), "got: {out}");
        assert!(!out.contains("old"), "got: {out}");
    }

    #[test]
    fn sequence_spelled_old_strands_in_window_only() {
        // The author replays the old name in-window (no substitution
        // spelled): the arm still fires, post-window completes, and the
        // in-window leftover strands for the orphan scan (e2e pins the
        // `partial` verdict; here the text posture is pinned).
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = old;\n+ let new = E0;\n+ let b = old;\n"
        );
        let src = "fn f() {\n    let old = g(1);\n    let b = old;\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let b = old;"), "got: {out}");
        assert!(out.contains("let c = new;"), "got: {out}");
    }

    #[test]
    fn sequence_shadowed_init_replays_verbatim() {
        // A shadowed init matches only through a hole; the hole replays
        // verbatim while post-window uses still complete around it.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n"
        );
        let src =
            "fn f() {\n    let old = g(1);\n    let b = { let old = 9; old };\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let old = 9;"), "got: {out}");
        assert!(out.contains("let c = new;"), "got: {out}");
    }

    #[test]
    fn sequence_destructuring_macro_post_window_use_stays() {
        // Destructuring defs share the plain follow path: the macro
        // interior stays verbatim while the plain post-window use
        // completes.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let (x, old) = E0;\n- let b = E1;\n+ let (x, new) = E0;\n+ let b = E1;\n"
        );
        let src = "fn f() {\n    let (x, old) = g();\n    let b = h();\n    println!(\"{}\", old);\n    let d = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let (x, new) = g();"), "got: {out}");
        assert!(out.contains("println!(\"{}\", old);"), "got: {out}");
        assert!(out.contains("let d = new;"), "got: {out}");
    }

    #[test]
    fn sequence_macro_init_replays_verbatim() {
        // Macro interiors never rewrite: the hole replays verbatim,
        // the macro text stays, the plain post-window use completes.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n"
        );
        let src = "fn f() {\n    let old = g(1);\n    let b = foo!(old);\n    let d = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let b = foo!(old);"), "got: {out}");
        assert!(out.contains("let d = new;"), "got: {out}");
    }

    #[test]
    fn sequence_new_mentioning_window_applies_window_only() {
        // Naming the new ident elsewhere risks capture: decline the
        // arm, window applies verbatim, post-window strands.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = new;\n+ let new = E0;\n+ let b = new;\n"
        );
        let src = "fn f() {\n    let old = g(1);\n    let b = new;\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let c = old;"), "got: {out}");
    }

    #[test]
    fn sequence_window_binding_new_declines_arm() {
        // Another window statement binding `new` would double-bind
        // after the rename: decline the arm (window still applies).
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- let old = E1;\n+ let P0 = E0;\n+ let new = E1;\n"
        );
        let src = "fn f() {\n    let new = g(0);\n    let old = g(1);\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let c = old;"), "got: {out}");
    }

    #[test]
    fn sequence_outer_scope_new_declines_follow() {
        // Slice 3 remainder: a pre-existing `new` binder in the window's
        // own scope declines the follow arm — the window renames but the
        // post-window use strands (the orphan scan owns the `partial`).
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n"
        );
        let src = "fn f() {\n    let new = 0;\n    let old = g(1);\n    let b = h();\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let c = old;"), "got: {out}");
    }

    #[test]
    fn sequence_outer_scope_new_after_window_declines_follow() {
        // A later same-level `let new` double-binds the scope: same
        // decline, post-window strands.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n"
        );
        let src = "fn f() {\n    let old = g(1);\n    let b = h();\n    let c = old;\n    let new = 9;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let c = old;"), "got: {out}");
        assert!(out.contains("let new = 9;"), "got: {out}");
    }

    #[test]
    fn sequence_sibling_scope_new_still_completes() {
        // A `new` binder in a descendant scope never collides: the arm
        // still fires and post-window completes.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let old = E0;\n- let b = E1;\n+ let new = E0;\n+ let b = E1;\n"
        );
        let src = "fn f() {\n    let old = g(1);\n    let b = h();\n    { let new = 1; }\n    let c = old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("let new = g(1);"), "got: {out}");
        assert!(out.contains("let c = new;"), "got: {out}");
    }

    fn wi18_seq_hunk_and_pattern() -> (
        rsmpl_patch::RsmplHunk,
        rsmpl_pattern::CompiledPattern,
        syn::File,
        crate::lines::LineIndex,
        String,
    ) {
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- let P1 = E1;\n+ let P0: u32 = E0;\n+ let P1: u32 = E1;\n"
        );
        let mut parsed_patch = rsmpl_patch::parse_patch(&patch).expect("test patch parses");
        for f in &mut parsed_patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        let hunk = match parsed_patch.files.remove(0).hunks.remove(0) {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&hunk).expect("valid sequence compiles");
        let src = "fn f() {\n    let a = 1;\n    let b = 2;\n}\n".to_string();
        let parsed: syn::File = syn::parse_str(&src).expect("src parses");
        let index = crate::lines::LineIndex::new(&src);
        (hunk, compiled, parsed, index, src)
    }

    fn wi18_seq_repair(err: &rsmpl_patch::StructuredError) -> (String, String) {
        let repair = err
            .repair
            .as_ref()
            .expect("WI-18 slice 3: must carry repair");
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
        (code, write_instead)
    }

    #[test]
    fn wi18_slice3_sequence_dispatch_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_seq_hunk_and_pattern();
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err = super::apply_sequence_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected statement-sequence pattern");
        let (code, w) = wi18_seq_repair(&err);
        assert_eq!(code, "sequence-template");
        assert!(w.contains("2-4 single-line plain `let`"), "got: {w}");
    }

    #[test]
    fn wi18_slice3_sequence_invalid_minus_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_seq_hunk_and_pattern();
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::General { minus_sub, .. } => {
                *minus_sub = "let = ==".to_string();
            }
            _ => panic!("want sequence pattern"),
        }
        let mut edits = Vec::new();
        let err = super::apply_sequence_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("invalid minus must fail");
        assert!(
            err.message
                .starts_with("invalid statement-sequence pattern"),
            "got: {}",
            err.message
        );
        let (code, w) = wi18_seq_repair(&err);
        assert_eq!(code, "sequence-let-syntax");
        assert!(w.contains("`let P0 = E0;`"), "got: {w}");
    }

    #[test]
    fn wi18_slice3_sequence_non_let_member_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_seq_hunk_and_pattern();
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::General { minus_sub, .. } => {
                *minus_sub = "foo();\nbar();".to_string();
            }
            _ => panic!("want sequence pattern"),
        }
        let mut edits = Vec::new();
        let err = super::apply_sequence_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("non-let member must fail");
        assert_eq!(
            err.message,
            "statement-sequence members must be plain `let` statements in v1"
        );
        let (code, w) = wi18_seq_repair(&err);
        assert_eq!(code, "sequence-let-only");
        assert!(w.contains("only plain `let`"), "got: {w}");
    }

    #[test]
    fn wi18_slice3_sequence_k_range_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_seq_hunk_and_pattern();
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::General { minus_sub, .. } => {
                *minus_sub = "let a = 1;".to_string();
            }
            _ => panic!("want sequence pattern"),
        }
        let mut edits = Vec::new();
        let err = super::apply_sequence_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("K=1 must fail");
        assert_eq!(
            err.message,
            "statement-sequence members must be plain `let` statements in v1"
        );
        let (code, w) = wi18_seq_repair(&err);
        assert_eq!(code, "sequence-let-only");
        assert!(w.contains("2-4 lines"), "got: {w}");
    }

    #[test]
    fn wi18_slice3_sequence_plus_count_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = wi18_seq_hunk_and_pattern();
        match &mut compiled.kind {
            rsmpl_pattern::PatternKind::General { plus_raw, .. } => {
                *plus_raw = "let P0: u32 = E0; let P1: u32 = E1;".to_string();
            }
            _ => panic!("want sequence pattern"),
        }
        let mut edits = Vec::new();
        let err = super::apply_sequence_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("one-line two-stmt plus must fail");
        assert!(
            err.message.contains("plus side must hold 2 single-line"),
            "got: {}",
            err.message
        );
        let (code, w) = wi18_seq_repair(&err);
        assert_eq!(code, "sequence-plus-count");
        assert!(w.contains("same count"), "got: {w}");
    }

    #[test]
    fn wi18_slice3_sequence_plus_count_reachable_via_apply() {
        // Authored trigger: one '+' statement split across two diff lines
        // parses as 2 stmts at compile time but counts as 3 lines at match
        // time, so the engine plus-count gate (not the C18 compile gate)
        // fires.
        let patch = format!(
            "{HEAD}@@ -1,1 @@ rule=r\n# sweep\n- let P0 = E0;\n- let P1 = E1;\n+ let P0: u32 =\n+ E0;\n+ let P1: u32 = E1;\n"
        );
        let src = "fn f() {\n    let a = 1;\n    let b = 2;\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(
            err.contains("plus side must hold 2 single-line"),
            "got: {err}"
        );
        assert!(
            !err.contains("\"code\":\"pattern\""),
            "must not be the generic filler: {err}"
        );
    }
}
