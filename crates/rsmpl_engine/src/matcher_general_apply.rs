//! General-hunk driver: `apply_general_hunk` plus the `# repeat` fold (WI-2b split).
//!
//! Split from `matcher_general` (D5): matching stays there
//! (`match_expr`, `MatchCx`, `Collector`); this module owns application —
//! the attribute-envelope helpers, the nested-rewrite fold, and the
//! Stmt/Expr/Type driver. Callers enter via `apply_general_hunk`.

use std::cmp::Reverse;
use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_attr_delta::{
    attr_delta as general_attr_delta, compact_attr, line_indent, norm_pat_list,
    site_attr_texts as general_site_attrs, subsequence_positions as general_subseq,
    AttrDelta as GeneralAttrDelta,
};
use crate::matcher_fn::delete_lines;
use crate::matcher_general::{in_scope, match_expr, render, Collector, MatchCx};
use crate::matcher_general_tails::substitute_general_plus;
use crate::matcher_types::match_type;
use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, StructuredError};
use rsmpl_pattern::{CompiledPattern, GeneralShape, PatternKind};

/// Whitespace-insensitive attribute comparison: strip ALL whitespace so raw
/// pattern text (`#[allow(x)]`) compares equal to `quote!`-rendered targets
/// (`# [ allow ( x ) ]`). Matches the fn/item `check_attrs` normalization.
fn norm_ws(s: &str) -> String {
    s.split_whitespace().collect::<String>()
}

fn target_attrs_rendered(local: &syn::Local) -> Vec<String> {
    local.attrs.iter().map(|a| norm_ws(&render(a))).collect()
}

/// Original byte slice of the attribute envelope: from the first attribute
/// (or the `let` keyword) to the `let` keyword start.
fn attr_prefix<'a>(original: &'a str, index: &LineIndex, local: &syn::Local) -> Option<&'a str> {
    let attr_start = local
        .attrs
        .first()
        .map(|a| a.span().start())
        .unwrap_or_else(|| local.let_token.span().start());
    let let_start = local.let_token.span().start();
    let s = index.offset(attr_start.line as u32, attr_start.column)?;
    let e = index.offset(let_start.line as u32, let_start.column)?;
    original.get(s..e)
}

/// Backstop for `# repeat` nesting (T126: same bound as the call-matcher
/// fold — bindings are strict sub-slices of the matched candidate, so
/// structural descent terminates on its own; this only guards
/// pathological spans).
const MAX_GENERAL_REPEAT_DEPTH: usize = 64;

/// T126 `# repeat`: rewrite every non-symbol binding with the same
/// general-expression rule (bottom-up fold). Symbol seeds are pre-bound
/// paths, not matchable fragments, so they are skipped — like the
/// call-matcher fold in `matcher_expr`.
fn fold_general_bindings(
    bindings: &mut HashMap<String, String>,
    compiled: &CompiledPattern,
    pat_expr: &syn::Expr,
    plus_raw: &str,
    reverse: &HashMap<String, String>,
    depth: usize,
    rule: &str,
) -> Result<(), StructuredError> {
    if depth > MAX_GENERAL_REPEAT_DEPTH {
        return Err(StructuredError::new(
            ErrorCode::MatchConflict,
            "repeat nesting exceeds 64 levels (pathological nesting?)",
        )
        .with_rule(rule)
        .with_suggestion("remove `# repeat` or simplify the nesting")
        .with_repair(StructuredError::fix_pattern_repair(
            "repeat",
            "remove `# repeat` or simplify the nesting",
        )));
    }
    // Snapshot names: rewriting mutates the map.
    let names: Vec<String> = bindings.keys().cloned().collect();
    for name in names {
        if compiled.table.symbols.contains_key(&name) {
            continue;
        }
        if let Some(current) = bindings.get(&name).cloned() {
            let rewritten = rewrite_nested_general(
                &current, compiled, pat_expr, plus_raw, reverse, depth, rule,
            )?;
            bindings.insert(name, rewritten);
        }
    }
    Ok(())
}

/// Re-apply the same general-expression rule inside one binding fragment:
/// collect sub-expressions over the fragment, rewrite outermost-first with
/// recursive descent into sub-bindings, and splice the result. The produced
/// output is never re-scanned, so self-reproducing rules apply exactly one
/// level and terminate by construction. Unparseable fragments are kept
/// verbatim (conservative, like the call-matcher fold).
fn rewrite_nested_general(
    text: &str,
    compiled: &CompiledPattern,
    pat_expr: &syn::Expr,
    plus_raw: &str,
    reverse: &HashMap<String, String>,
    depth: usize,
    rule: &str,
) -> Result<String, StructuredError> {
    let expr: syn::Expr = match syn::parse_str(text) {
        Ok(e) => e,
        Err(_) => return Ok(text.to_string()),
    };
    let index = LineIndex::new(text);
    let mut collector = Collector {
        index: &index,
        stmts: Vec::new(),
        exprs: Vec::new(),
        types: Vec::new(),
    };
    collector.visit_expr(&expr);
    let mut cands = collector.exprs;
    // Longest-first on shared start (T143): left-nested `a + 1 + 1`
    // parses as `(a + 1) + 1` with inner/outer sharing a start byte;
    // shortest-first accepted the inner and the outer tripped the
    // partial-overlap guard. Longest-first restores outermost-wins.
    cands.sort_by_key(|(_, n)| (n.start, Reverse(n.end)));
    // Nested fragments are sub-expressions: strip one trailing `;` so a
    // `;`-terminated file-level pattern never injects semicolons
    // mid-expression. File-level `had_semi` handling is untouched.
    let trimmed = plus_raw.trim_end();
    let nested_plus = trimmed.strip_suffix(';').unwrap_or(trimmed);
    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut edits: Vec<TextEdit> = Vec::new();
    for (tgt, node) in &cands {
        // Outermost-wins, like the file-level pass (SPEC 17.6).
        if accepted
            .iter()
            .any(|&(s, e)| s <= node.start && node.end <= e)
        {
            continue;
        }
        let mut cx = MatchCx::with_reverse(text, &index, reverse.clone());
        if !match_expr(pat_expr, tgt, &mut cx) {
            continue;
        }
        for &(s, e) in &accepted {
            if node.start < e && s < node.end && !(s <= node.start && node.end <= e) {
                return Err(crate::validate::partial_overlap_error(rule));
            }
        }
        fold_general_bindings(
            &mut cx.bindings,
            compiled,
            pat_expr,
            plus_raw,
            reverse,
            depth + 1,
            rule,
        )?;
        let replacement = substitute_general_plus(
            nested_plus,
            &cx.bindings,
            &cx.block_tails,
            compiled.table.sigiled,
        )?;
        edits.push(TextEdit::new(node.start, node.end, replacement));
        accepted.push((node.start, node.end));
    }
    if edits.is_empty() {
        return Ok(text.to_string());
    }
    crate::edits::apply_edits(text, edits)
}

/// Apply a generalized Stmt/Expr/Type hunk. Returns the match count.
pub fn apply_general_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (shape, minus_sub, plus_raw, plus_body, minus_attrs, plus_attrs, had_semi) =
        match &compiled.kind {
            PatternKind::General {
                shape,
                minus_sub,
                plus_raw,
                plus_body,
                minus_attrs,
                plus_attrs,
                had_semi,
                ..
            } => (
                *shape,
                minus_sub.clone(),
                plus_raw.clone(),
                plus_body.clone(),
                minus_attrs.clone(),
                plus_attrs.clone(),
                *had_semi,
            ),
            _ => {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "expected generalized stmt/expr/type pattern".to_string(),
                ))
            }
        };
    let rule = hunk.rule_name();

    // Attribute shape validation up front (clear errors, no guessing).
    if shape != GeneralShape::Stmt && (!minus_attrs.is_empty() || !plus_attrs.is_empty()) {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "leading attributes are only supported on `let` statement patterns in v1",
        )
        .with_rule(&rule)
        .with_repair(StructuredError::fix_pattern_repair(
            "C10",
            "write a `let` statement pattern, or drop the leading attributes",
        )));
    }
    // WI-10b: under `# attrs=preserve` written lines are the delta
    // (add/remove/same-length replace, shared laws with fn/item);
    // whole-stmt deletion (empty plus) keeps the gates — no statement
    // remains to attach a delta to. Differing non-empty lengths stay
    // ambiguous.
    let deleting_shape = plus_raw.trim().is_empty();
    let preserve = hunk.attrs_policy() == AttrsPolicy::Preserve;
    let minus_norm = norm_pat_list(&minus_attrs);
    let plus_norm = norm_pat_list(&plus_attrs);
    let delta = general_attr_delta(&minus_attrs, &plus_attrs);
    let ambiguous_len = preserve
        && minus_norm != plus_norm
        && !minus_attrs.is_empty()
        && !plus_attrs.is_empty()
        && minus_attrs.len() != plus_attrs.len();
    let delta_active =
        preserve && !deleting_shape && !ambiguous_len && delta != GeneralAttrDelta::Mirror;
    if !delta_active && !plus_attrs.is_empty() && minus_attrs.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them",
        )
        .with_rule(&rule)
        .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`")
        .with_repair(StructuredError::fix_pattern_repair(
            "C11",
            "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
        )));
    }
    // `# attrs=strip` skips the mirror gate: mirrored minus attrs are
    // eligibility-ignored (like the fn/item `check_attrs` Strip arm) and
    // the Stmt replacement path drops the envelope. The plus-writes-attrs
    // gate above stays: adding attributes while stripping is ambiguous.
    if !delta_active
        && hunk.attrs_policy() != AttrsPolicy::Strip
        && !minus_attrs.is_empty()
        && norm_ws(&plus_attrs.join("\n")) != norm_ws(&minus_attrs.join("\n"))
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
        )
        .with_rule(&rule)
        .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`")
        .with_repair(StructuredError::fix_pattern_repair(
            "C12",
            "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
        )));
    }

    let reverse: HashMap<String, String> = match &compiled.kind {
        PatternKind::General { placeholders, .. } => placeholders
            .iter()
            .map(|(n, ph)| (ph.clone(), n.clone()))
            .collect(),
        _ => HashMap::new(),
    };

    // Parse the substituted minus pattern and collect candidates.
    let mut collector = Collector {
        index,
        stmts: Vec::new(),
        exprs: Vec::new(),
        types: Vec::new(),
    };
    collector.visit_file(parsed);
    // T143: exprs sort longest-first on shared start so left-nested
    // `(a + 1) + 1`-shaped spans see the outer first (outermost-wins);
    // stmts/types have no shared-start nesting, keep source order.
    collector
        .exprs
        .sort_by_key(|(_, n)| (n.start, Reverse(n.end)));
    collector.types.sort_by_key(|(_, n)| (n.start, n.end));
    collector.stmts.sort_by_key(|(_, n)| (n.start, n.end));

    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut matched = 0;
    let mut all_bindings: Vec<HashMap<String, String>> = Vec::new();

    match shape {
        // WI-5c: sequences match K consecutive `Block` statements with
        // shared bindings — a different candidate domain (windows, not
        // single statements), so they delegate to the sequence module.
        GeneralShape::Sequence => {
            return crate::matcher_sequence::apply_sequence_hunk(
                parsed, original, index, hunk, compiled, edits,
            );
        }
        GeneralShape::Stmt => {
            let pat_stmt = syn::parse_str::<syn::Stmt>(&minus_sub).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("invalid `let` pattern: {e}"),
                )
                .with_rule(&rule)
            })?;
            let syn::Stmt::Local(pat_local) = &pat_stmt else {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "general statement patterns must be `let` statements in v1",
                )
                .with_rule(&rule));
            };
            let deleting = plus_raw.trim().is_empty();
            for (tgt, node) in &collector.stmts {
                if accepted
                    .iter()
                    .any(|&(s, e)| s <= node.start && node.end <= e)
                {
                    continue;
                }
                if !in_scope(
                    node.start_line,
                    node.end_line,
                    hunk.scope(),
                    hunk.header.old_start,
                    hunk.header.old_len,
                ) {
                    continue;
                }
                let mut cx = MatchCx::with_reverse(original, index, reverse.clone());
                if !crate::matcher_general_ctrl::match_local(pat_local, tgt, &mut cx) {
                    continue;
                }
                // Attribute policy (mirrors fn matching: fail, never guess).
                // WI-10b: delta hunks use subset semantics (add skips
                // already-plus sites; remove/replace require the minus
                // lines present, preserving the rest).
                let prefix = attr_prefix(original, index, tgt).unwrap_or("");
                match hunk.attrs_policy() {
                    AttrsPolicy::Preserve => {
                        if delta_active {
                            let have = general_site_attrs(&tgt.attrs);
                            match delta {
                                GeneralAttrDelta::Add => {
                                    if general_subseq(&have, &plus_norm).is_some() {
                                        continue;
                                    }
                                }
                                GeneralAttrDelta::Remove | GeneralAttrDelta::Replace => {
                                    if general_subseq(&have, &minus_norm).is_none() {
                                        continue;
                                    }
                                }
                                GeneralAttrDelta::Mirror => {}
                            }
                        } else if !minus_attrs.is_empty()
                            && norm_ws(&minus_attrs.join("\n"))
                                != norm_ws(&target_attrs_rendered(tgt).join("\n"))
                        {
                            return Err(StructuredError::new(
                                ErrorCode::MatchConflict,
                                "attached attributes do not match explicitly written pattern attributes",
                            )
                            .with_rule(&rule)
                            .with_suggestion("use `# attrs=preserve` (default) to ignore attributes")
                            .with_repair(StructuredError::add_directive_repair("# attrs=preserve")));
                        }
                    }
                    AttrsPolicy::Absent | AttrsPolicy::Explicit => {
                        if !minus_attrs.is_empty() {
                            if norm_ws(&minus_attrs.join("\n"))
                                != norm_ws(&target_attrs_rendered(tgt).join("\n"))
                            {
                                return Err(StructuredError::new(
                                    ErrorCode::MatchConflict,
                                    "attached attributes do not match explicitly written pattern attributes",
                                )
                                .with_rule(&rule)
                                .with_suggestion("use `# attrs=preserve` (default) to ignore attributes")
                                .with_repair(StructuredError::add_directive_repair("# attrs=preserve")));
                            }
                        } else if !tgt.attrs.is_empty() {
                            return Err(StructuredError::new(
                                ErrorCode::MatchConflict,
                                format!(
                                    "statement has attributes but `# attrs={}` requires none",
                                    match hunk.attrs_policy() {
                                        AttrsPolicy::Absent => "absent",
                                        _ => "explicit",
                                    }
                                ),
                            )
                            .with_rule(&rule)
                            .with_repair(StructuredError::fix_pattern_repair(
                                "attrs",
                                "remove the `# attrs=...` directive to ignore attributes (the default), or write the attributes into the pattern",
                            )));
                        }
                    }
                    AttrsPolicy::Strip => {}
                }
                if deleting {
                    edits.push(delete_lines(
                        original,
                        index,
                        node.start_line,
                        node.end_line,
                        &rule,
                    )?);
                } else if delta_active {
                    // WI-10b: Preserve attr delta — the body edit shrinks
                    // to the `let` keyword (attrs excluded) while attr
                    // edits land above it, disjoint like fn/item. The
                    // plus body is attr-stripped (`plus_body`); attrs ride
                    // the separate edits, never the spliced text.
                    let let_lc = tgt.let_token.span().start();
                    let let_line = let_lc.line as u32;
                    let Some(let_start) = index.offset(let_line, let_lc.column) else {
                        continue;
                    };
                    let body = substitute_general_plus(
                        &plus_body,
                        &cx.bindings,
                        &cx.block_tails,
                        compiled.table.sigiled,
                    )?;
                    edits.push(TextEdit::new(let_start, node.end, body));
                    let have = general_site_attrs(&tgt.attrs);
                    match delta {
                        GeneralAttrDelta::Add => {
                            let indent = line_indent(original, index, let_line);
                            let mut text = String::new();
                            for p in &plus_attrs {
                                text.push_str(&indent);
                                text.push_str(&compact_attr(p.trim()));
                                text.push('\n');
                            }
                            if let Some(at) = index.line_start(let_line) {
                                edits.push(TextEdit::new(at, at, text));
                            }
                        }
                        GeneralAttrDelta::Remove => {
                            if let Some(idxs) = general_subseq(&have, &minus_norm) {
                                for i in idxs {
                                    let attr = &tgt.attrs[i];
                                    let s = attr.span().start().line as u32;
                                    let e = attr.span().end().line as u32;
                                    let a = index.line_start(s);
                                    let b = index.line_start(e + 1).unwrap_or(original.len());
                                    if let (Some(a), b) = (a, b) {
                                        if b > a {
                                            edits.push(TextEdit::new(a, b, String::new()));
                                        }
                                    }
                                }
                            }
                        }
                        GeneralAttrDelta::Replace => {
                            if let Some(idxs) = general_subseq(&have, &minus_norm) {
                                for (k, i) in idxs.iter().enumerate() {
                                    let Some(plus_line) = plus_attrs.get(k) else {
                                        continue;
                                    };
                                    let attr = &tgt.attrs[*i];
                                    let s = attr.span().start().line as u32;
                                    let e = attr.span().end().line as u32;
                                    let (Some(a), b) = (
                                        index.line_start(s),
                                        index.line_start(e + 1).unwrap_or(original.len()),
                                    ) else {
                                        continue;
                                    };
                                    let Some(cur_start) = index.line_start(s) else {
                                        continue;
                                    };
                                    let cur_line = &original[cur_start..];
                                    let cur_line = cur_line.split('\n').next().unwrap_or("");
                                    let indent: String = cur_line
                                        .chars()
                                        .take_while(|ch| *ch == ' ' || *ch == '\t')
                                        .collect();
                                    let mut text = String::new();
                                    text.push_str(&indent);
                                    text.push_str(&compact_attr(plus_line.trim()));
                                    text.push('\n');
                                    if b > a {
                                        edits.push(TextEdit::new(a, b, text));
                                    }
                                }
                            }
                        }
                        GeneralAttrDelta::Mirror => {}
                    }
                } else {
                    // Mirrored attributes (T153): the plus attr lines equal the
                    // target envelope modulo whitespace, so reuse the original
                    // envelope slice (attrs + `let` indent, exactly like the
                    // preserve path) and substitute only the attr-stripped
                    // plus body. Splicing raw plus text would replace the
                    // target's `let` indent with the pattern's one-space
                    // diff-prefix artifact. Scoped to Preserve/Explicit (the
                    // policies that keep attributes); Strip keeps its path.
                    let mirrored = !minus_attrs.is_empty()
                        && matches!(
                            hunk.attrs_policy(),
                            AttrsPolicy::Preserve | AttrsPolicy::Explicit
                        );
                    let mut replacement = if mirrored {
                        let mut with_prefix = prefix.to_string();
                        with_prefix.push_str(&substitute_general_plus(
                            &plus_body,
                            &cx.bindings,
                            &cx.block_tails,
                            compiled.table.sigiled,
                        )?);
                        with_prefix
                    } else {
                        substitute_general_plus(
                            &plus_raw,
                            &cx.bindings,
                            &cx.block_tails,
                            compiled.table.sigiled,
                        )?
                    };
                    if hunk.attrs_policy() == AttrsPolicy::Preserve && minus_attrs.is_empty() {
                        let mut with_prefix = prefix.to_string();
                        with_prefix.push_str(&replacement);
                        replacement = with_prefix;
                    }
                    // Strip: replacement already drops the envelope.
                    edits.push(TextEdit::new(node.start, node.end, replacement));
                }
                for &(s, e) in &accepted {
                    if node.start < e && s < node.end && !(s <= node.start && node.end <= e) {
                        return Err(crate::validate::partial_overlap_error(&rule));
                    }
                }
                accepted.push((node.start, node.end));
                all_bindings.push(cx.bindings.clone());
                matched += 1;
                if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                    break;
                }
            }
        }
        GeneralShape::Expr => {
            let pat_expr = syn::parse_str::<syn::Expr>(&minus_sub).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("invalid expression pattern: {e}"),
                )
                .with_rule(&rule)
            })?;
            let deleting = plus_raw.trim().is_empty();
            for (tgt, node) in &collector.exprs {
                if accepted
                    .iter()
                    .any(|&(s, e)| s <= node.start && node.end <= e)
                {
                    continue;
                }
                if !in_scope(
                    node.start_line,
                    node.end_line,
                    hunk.scope(),
                    hunk.header.old_start,
                    hunk.header.old_len,
                ) {
                    continue;
                }
                let mut cx = MatchCx::with_reverse(original, index, reverse.clone());
                if !match_expr(&pat_expr, tgt, &mut cx) {
                    continue;
                }
                // T126 `# repeat`: fold nested matches by rewriting each
                // bound fragment with the same rule before substitution
                // (bottom-up, like the call-matcher fold).
                if hunk.repeats() {
                    fold_general_bindings(
                        &mut cx.bindings,
                        compiled,
                        &pat_expr,
                        &plus_raw,
                        &reverse,
                        0,
                        &rule,
                    )?;
                }
                if deleting {
                    // Only `...;` statement shapes reach here (compile-gated).
                    edits.push(delete_lines(
                        original,
                        index,
                        node.start_line,
                        node.end_line,
                        &rule,
                    )?);
                } else {
                    let mut replacement = substitute_general_plus(
                        &plus_raw,
                        &cx.bindings,
                        &cx.block_tails,
                        compiled.table.sigiled,
                    )?;
                    let mut end = node.end;
                    if had_semi {
                        // The candidate span excludes the statement's `;`:
                        // consume the original semicolon (mod whitespace) so
                        // the replacement ends with exactly one `;`
                        // (dogfood: `E + 1;` rewrites gave `;;`).
                        let rest = &original[node.end..];
                        let ws = rest.len() - rest.trim_start().len();
                        if rest.get(ws..).is_some_and(|t| t.starts_with(';')) {
                            end = node.end + ws + 1;
                        }
                        if !replacement.trim_end().ends_with(';') {
                            replacement.push(';');
                        }
                    }
                    edits.push(TextEdit::new(node.start, end, replacement));
                }
                for &(s, e) in &accepted {
                    if node.start < e && s < node.end && !(s <= node.start && node.end <= e) {
                        return Err(crate::validate::partial_overlap_error(&rule));
                    }
                }
                accepted.push((node.start, node.end));
                all_bindings.push(cx.bindings.clone());
                matched += 1;
                if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                    break;
                }
            }
        }
        GeneralShape::Type => {
            let pat_ty = syn::parse_str::<syn::Type>(&minus_sub).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("invalid type pattern: {e}"),
                )
                .with_rule(&rule)
            })?;
            for (tgt, node) in &collector.types {
                if accepted
                    .iter()
                    .any(|&(s, e)| s <= node.start && node.end <= e)
                {
                    continue;
                }
                if !in_scope(
                    node.start_line,
                    node.end_line,
                    hunk.scope(),
                    hunk.header.old_start,
                    hunk.header.old_len,
                ) {
                    continue;
                }
                let mut cx = MatchCx::with_reverse(original, index, reverse.clone());
                if !match_type(&pat_ty, tgt, &mut cx) {
                    continue;
                }
                let replacement = substitute_general_plus(
                    &plus_raw,
                    &cx.bindings,
                    &cx.block_tails,
                    compiled.table.sigiled,
                )?;
                edits.push(TextEdit::new(node.start, node.end, replacement));
                for &(s, e) in &accepted {
                    if node.start < e && s < node.end && !(s <= node.start && node.end <= e) {
                        return Err(crate::validate::partial_overlap_error(&rule));
                    }
                }
                accepted.push((node.start, node.end));
                all_bindings.push(cx.bindings.clone());
                matched += 1;
                if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                    break;
                }
            }
        }
    }
    Ok((matched, all_bindings))
}
