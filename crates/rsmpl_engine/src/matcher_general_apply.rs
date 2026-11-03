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
use crate::matcher_general::{in_scope, match_expr, render, Collector};
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
#[allow(clippy::too_many_arguments)]
fn fold_general_bindings(
    bindings: &mut HashMap<String, String>,
    compiled: &CompiledPattern,
    pat_expr: &syn::Expr,
    plus_raw: &str,
    reverse: &HashMap<String, String>,
    symbols: &HashMap<String, String>,
    fallback_syntax: bool,
    symbol_graph: Option<(&crate::CrateGraph, &str)>,
    seed_canons: &HashMap<String, String>,
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
                &current,
                compiled,
                pat_expr,
                plus_raw,
                reverse,
                symbols,
                fallback_syntax,
                symbol_graph,
                seed_canons,
                depth,
                rule,
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
///
/// NOTE (WI-4 slice 1): the fold applies the outer hunk's symbol posture
/// inside each nested fragment (pre-bound seeds; resolved checks reuse the
/// hunk graph), so a nested whole-path symbol verifies exactly like a
/// top-level one.
#[allow(clippy::too_many_arguments)]
fn rewrite_nested_general(
    text: &str,
    compiled: &CompiledPattern,
    pat_expr: &syn::Expr,
    plus_raw: &str,
    reverse: &HashMap<String, String>,
    symbols: &HashMap<String, String>,
    fallback_syntax: bool,
    symbol_graph: Option<(&crate::CrateGraph, &str)>,
    seed_canons: &HashMap<String, String>,
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
        let mut cx = crate::matcher_symbol::symbol_cx(
            text,
            &index,
            reverse.clone(),
            symbols,
            fallback_syntax,
            symbol_graph,
            seed_canons,
        );
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
            symbols,
            fallback_syntax,
            symbol_graph,
            seed_canons,
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
    symbol_graph: Option<(&crate::CrateGraph, &str)>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (shape, minus_sub, plus_raw, plus_body, minus_attrs, plus_attrs, had_semi) = match &compiled
        .kind
    {
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
        _ => return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "expected generalized stmt/expr/type pattern".to_string(),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "general-dispatch-template",
            "write a `let`-statement, expression, type, or 2-4-`let` sequence pair (see SPEC 18.6)",
        ))),
    };
    let rule = hunk.rule_name();

    // WI-4 slice 1: whole-path `# symbol` posture (SPEC 19.3). Seeds are
    // pre-bound into every candidate context (the plus side emits the
    // concrete seed, exactly the call-hunk posture); bare symbols verify
    // their minus-side seeds here, per-site checks run in
    // `matcher_symbol` through each candidate `MatchCx`.
    let symbols = compiled.table.symbols.clone();
    let fallback_syntax = hunk.fallback_syntax();
    let mut seed_canons: HashMap<String, String> = HashMap::new();
    if !symbols.is_empty() && !fallback_syntax {
        let Some((graph, _file)) = symbol_graph else {
            // Mirror the call-matcher gate (SPEC 19.3): no index, no proof.
            let sym = symbols.keys().next().cloned().unwrap_or_default();
            return Err(StructuredError::new(
                ErrorCode::SymbolConflict,
                format!("symbol `{sym}` could not be resolved (no semantic environment in v0.1)"),
            )
            .with_rule(&rule)
            .with_suggestion("add `# fallback=syntax` to allow syntactic matching")
            .with_repair(StructuredError::add_directive_repair("# fallback=syntax")));
        };
        seed_canons = crate::matcher_symbol::seed_canons_for(&symbols, &minus_sub, graph, &rule)?;
    }
    // Sequences never carry symbols in slice 1 (compile-time C14 owns
    // this; the loud arm below is defense-in-depth so the sequence
    // delegate can never bind a symbol placeholder blindly).
    if matches!(shape, GeneralShape::Sequence) && !symbols.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "this general statement-sequence hunk cannot use `# symbol` — sequences ride concrete paths in slice 1 (see SPEC 19.3)",
        )
        .with_rule(&rule)
        .with_repair(StructuredError::fix_pattern_repair(
            "C14",
            "write the full concrete path instead",
        )));
    }

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

    // C21: general Stmt/Expr templates never own binder renames the
    // binder machinery cannot track. A bound-ident change that invents a
    // binder (rename/add/concretion) — or drops one with no hole replay
    // — would rewrite the header while stranding body/file uses with no
    // orphans filed; fail loud instead of `verified`. Pure permutations,
    // hole replays (`P @ sub` → `P`, `Some(x)` → `Some(P)`), plain
    // renames and Stmt single-leaf destructuring renames through binder
    // identity keep applying. Sequences are exempt except a rename-once
    // `let` rename (plain or single-leaf destructuring) at any single
    // position, which enters binder identity (completion + orphans,
    // incl. authored init-substitution of later in-window uses): the
    // raiser owns other binder-renaming hetero runs there (WI-5d
    // single-block fusion, completed by sibling hunks in the same
    // patch), so a hunk-local veto would break the apply direction of
    // a proven cycle; spelled-`old` uses replay verbatim (orphans),
    // partial swaps and `new`/pat mentions stay declined (exempt
    // window-only).
    if matches!(shape, GeneralShape::Stmt | GeneralShape::Expr)
        && !crate::use_graph_local::is_local_binder_rename(compiled)
    {
        let placeholders: Vec<(String, String)> = match &compiled.kind {
            PatternKind::General { placeholders, .. } => placeholders.clone(),
            _ => Vec::new(),
        };
        if crate::matcher_general_binder::unclaimed_binder_rewrite(
            shape,
            &minus_sub,
            &plus_body,
            &placeholders,
        ) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "general template rewrites bound binder idents without binder identity; v1 general templates never own binder renames (C21)",
            )
            .with_rule(&rule)
            .with_repair(StructuredError::fix_pattern_repair(
                "C21",
                "rewrite as a single-binder hunk (plain `- let old = E;` / `+ let new = E;` and friends, SPEC 14.8) or use an ordinary hunk",
            )));
        }
    }

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
                .with_repair(StructuredError::fix_pattern_repair(
                    "general-let-invalid-template",
                    "write a parseable `let` statement on the minus side (`- let x = E;`; see SPEC 18.6)",
                ))
            })?;
            let syn::Stmt::Local(pat_local) = &pat_stmt else {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "general statement patterns must be `let` statements in v1",
                )
                .with_rule(&rule)
                .with_repair(StructuredError::fix_pattern_repair(
                    "general-let-stmt-template",
                    "write a `let` statement pattern (general statements must be `let` in v1; see SPEC 18.6)",
                )));
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
                let mut cx = crate::matcher_symbol::symbol_cx(
                    original,
                    index,
                    reverse.clone(),
                    &symbols,
                    fallback_syntax,
                    symbol_graph,
                    &seed_canons,
                );
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
                    // WI-2b definition label: a binder-statement rename is
                    // the `rename` (parity with fn/item renames); other
                    // general statements stay unlabeled. Follows below
                    // are `follow`.
                    if let Some((old, new)) =
                        crate::use_graph_local::let_binder_rename(&compiled.kind)
                    {
                        edits.push(
                            TextEdit::new(node.start, node.end, replacement).with_label("rename"),
                        );
                        // Single-`let` outer-`new` capture audit (parity
                        // with the sequence rename-once guard): a
                        // pre-existing `new` binder in the renamed
                        // statement's own scope collides either way, so
                        // decline follow (definition applies verbatim,
                        // post-binder uses strand as orphans).
                        match crate::use_graph_local_orphan::enclosing_new_binder_defs(
                            parsed,
                            original,
                            index,
                            &new,
                            (node.start, node.end),
                        ) {
                            Some(v) if v.is_empty() => {
                                edits.extend(crate::use_graph_local::let_binder_follow_edits(
                                    parsed, original, index, node.start, node.end, &old, &new,
                                ));
                            }
                            _ => {}
                        }
                    } else if let Some(b) = rsmpl_pattern::binder::binder_rename(&compiled.kind) {
                        // WI-3: only `LetClosure` reaches the Stmt arm
                        // besides `Let`. Binder-ident emission plus
                        // closure-body follows (the whole-statement
                        // replacement is redundant: minus/plus are
                        // token-equal outside the binder by construction).
                        let done = match b.scope {
                            rsmpl_pattern::binder::BinderScope::LetClosure => {
                                crate::use_graph_binder_unified::local_binder_span(
                                    tgt, b.scope, &b.old, original, index,
                                )
                                .map(|(bs, be)| {
                                    edits.push(
                                        TextEdit::new(bs, be, b.new.clone()).with_label("rename"),
                                    );
                                    edits.extend(
                                        crate::use_graph_binder_unified::scope_follow_edits(
                                            parsed,
                                            original,
                                            index,
                                            b.scope,
                                            &b.old,
                                            &b.new,
                                            &[(node.start, node.end)],
                                        ),
                                    );
                                })
                                .is_some()
                            }
                            _ => false,
                        };
                        if !done {
                            edits.push(TextEdit::new(node.start, node.end, replacement));
                        }
                    } else {
                        edits.push(TextEdit::new(node.start, node.end, replacement));
                    }
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
                .with_repair(StructuredError::fix_pattern_repair(
                    "general-expr-invalid-template",
                    "write a parseable expression on the minus side (see SPEC 18.6)",
                ))
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
                let mut cx = crate::matcher_symbol::symbol_cx(
                    original,
                    index,
                    reverse.clone(),
                    &symbols,
                    fallback_syntax,
                    symbol_graph,
                    &seed_canons,
                );
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
                        &symbols,
                        fallback_syntax,
                        symbol_graph,
                        &seed_canons,
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
                    // WI-3: single-binder expression renames emit the
                    // binder ident (labelled `rename`) plus scope
                    // follows — never the whole-node replacement (the
                    // rest is token-equal by construction, and keeping
                    // target formatting shrinks the native diff).
                    let binder_done = match rsmpl_pattern::binder::binder_rename(&compiled.kind) {
                        Some(b) => {
                            matches!(
                                b.scope,
                                rsmpl_pattern::binder::BinderScope::Closure
                                    | rsmpl_pattern::binder::BinderScope::MatchArm
                                    | rsmpl_pattern::binder::BinderScope::For
                                    | rsmpl_pattern::binder::BinderScope::IfLet
                                    | rsmpl_pattern::binder::BinderScope::WhileLet
                            ) && crate::use_graph_binder_unified::expr_binder_span(
                                tgt, b.scope, &b.old, original, index,
                            )
                            .map(|(bs, be)| {
                                edits.push(
                                    TextEdit::new(bs, be, b.new.clone()).with_label("rename"),
                                );
                                edits.extend(crate::use_graph_binder_unified::scope_follow_edits(
                                    parsed,
                                    original,
                                    index,
                                    b.scope,
                                    &b.old,
                                    &b.new,
                                    &[(node.start, node.end)],
                                ));
                            })
                            .is_some()
                        }
                        None => false,
                    };
                    if !binder_done {
                        edits.push(TextEdit::new(node.start, end, replacement));
                    }
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
                .with_repair(StructuredError::fix_pattern_repair(
                    "general-type-invalid-template",
                    "write a parseable type on the minus side (`- Vec<T>`; see SPEC 18.6)",
                ))
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
                let mut cx = crate::matcher_symbol::symbol_cx(
                    original,
                    index,
                    reverse.clone(),
                    &symbols,
                    fallback_syntax,
                    symbol_graph,
                    &seed_canons,
                );
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

#[cfg(test)]
mod wi18_slice12_tests {
    // WI-18 slice 12: the five general-apply sites carry site-specific
    // repairs. The dispatch else-arm is defence-in-depth (dispatch routes
    // by kind: forced with a wrong-kind `Raw` overwrite, slice-7/10/11
    // idiom). The `syn` parse arms are defence-in-depth too (compile-time
    // `try_general` validates first, so unparsable text never compiles to
    // `General`): the pins compile a valid `General` shape naturally, then
    // mutate `minus_sub` white-box to exercise the real arm.
    use super::GeneralShape;
    use crate::edits::TextEdit;

    const LET_SHAPE: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# sweep\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n";
    const SRC: &str = "fn f() {\n    let x = 1;\n}\n";

    fn setup(
        patch_text: &str,
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
        let compiled = rsmpl_pattern::compile(&hunk).expect("general patch compiles");
        let src = SRC.to_string();
        let parsed: syn::File = syn::parse_str(&src).expect("src parses");
        let index = crate::lines::LineIndex::new(&src);
        (hunk, compiled, parsed, index, src)
    }

    fn repair_of(err: &rsmpl_patch::StructuredError) -> (String, String) {
        let repair = err
            .repair
            .as_ref()
            .expect("WI-18 slice 12: must carry repair");
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

    #[test]
    fn wi18_slice12_general_dispatch_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = setup(LET_SHAPE);
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_general_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits, None)
                .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected generalized stmt/expr/type pattern");
        let (code, w) = repair_of(&err);
        assert_eq!(code, "general-dispatch-template");
        assert!(w.contains("SPEC 18.6"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_general_invalid_let_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = setup(LET_SHAPE);
        if let rsmpl_pattern::PatternKind::General { minus_sub, .. } = &mut compiled.kind {
            *minus_sub = "let x = ;".to_string();
        } else {
            panic!("LET_SHAPE must compile to General");
        }
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_general_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits, None)
                .expect_err("invalid let must fail");
        assert!(
            err.message.starts_with("invalid `let` pattern"),
            "got: {}",
            err.message
        );
        let (code, w) = repair_of(&err);
        assert_eq!(code, "general-let-invalid-template");
        assert!(w.contains("`- let x = E;`"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_general_non_let_stmt_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = setup(LET_SHAPE);
        if let rsmpl_pattern::PatternKind::General {
            minus_sub,
            plus_raw,
            plus_body,
            ..
        } = &mut compiled.kind
        {
            // An expression statement: parses as `syn::Stmt` but not as
            // `Stmt::Local`. Plus sides carry no binders either so the C21
            // binder gate (which runs first) stays silent.
            *minus_sub = "foo(1);".to_string();
            *plus_raw = "foo(2);".to_string();
            *plus_body = "foo(2);".to_string();
        } else {
            panic!("LET_SHAPE must compile to General");
        }
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_general_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits, None)
                .expect_err("non-let stmt must fail");
        assert_eq!(
            err.message,
            "general statement patterns must be `let` statements in v1"
        );
        let (code, w) = repair_of(&err);
        assert_eq!(code, "general-let-stmt-template");
        assert!(w.contains("`let`"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_general_invalid_expr_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = setup(LET_SHAPE);
        if let rsmpl_pattern::PatternKind::General {
            shape, minus_sub, ..
        } = &mut compiled.kind
        {
            *shape = GeneralShape::Expr;
            *minus_sub = "foo(;".to_string();
        } else {
            panic!("LET_SHAPE must compile to General");
        }
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_general_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits, None)
                .expect_err("invalid expr must fail");
        assert!(
            err.message.starts_with("invalid expression pattern"),
            "got: {}",
            err.message
        );
        let (code, w) = repair_of(&err);
        assert_eq!(code, "general-expr-invalid-template");
        assert!(w.contains("SPEC 18.6"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_general_invalid_type_repair_is_specific() {
        let (hunk, mut compiled, parsed, index, src) = setup(LET_SHAPE);
        if let rsmpl_pattern::PatternKind::General {
            shape, minus_sub, ..
        } = &mut compiled.kind
        {
            *shape = GeneralShape::Type;
            *minus_sub = "Vec<;".to_string();
        } else {
            panic!("LET_SHAPE must compile to General");
        }
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_general_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits, None)
                .expect_err("invalid type must fail");
        assert!(
            err.message.starts_with("invalid type pattern"),
            "got: {}",
            err.message
        );
        let (code, w) = repair_of(&err);
        assert_eq!(code, "general-type-invalid-template");
        assert!(w.contains("`- Vec<T>`"), "got: {w}");
    }
}

#[cfg(test)]
mod wi4_slice1_tests {
    // WI-4 slice 1: whole-path `# symbol` in General Stmt/Expr/Type hunks.
    // Fires exactly (fallback) or by crate-graph proof (resolved, short
    // spellings via imports included, vendored copies excluded);
    // unresolvable seeds fail loud.
    use std::collections::HashMap;

    use super::apply_general_hunk;
    use crate::edits::TextEdit;

    fn compiled_of(patch_text: &str) -> (rsmpl_patch::RsmplHunk, rsmpl_pattern::CompiledPattern) {
        let mut parsed_patch = rsmpl_patch::parse_patch(patch_text).expect("test patch parses");
        let hunk = match parsed_patch.files.remove(0).hunks.remove(0) {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&hunk).expect("test patch compiles");
        (hunk, compiled)
    }

    fn run(
        patch_text: &str,
        src: &str,
        graph: Option<(&crate::CrateGraph, &str)>,
    ) -> Result<(usize, String), rsmpl_patch::StructuredError> {
        let (hunk, compiled) = compiled_of(patch_text);
        let parsed: syn::File = syn::parse_str(src).expect("src parses");
        let index = crate::lines::LineIndex::new(src);
        let mut edits = Vec::<TextEdit>::new();
        let (n, _) = apply_general_hunk(&parsed, src, &index, &hunk, &compiled, &mut edits, graph)?;
        let out = crate::edits::apply_edits(src, edits).expect("edits apply");
        Ok((n, out))
    }

    const LET_FALLBACK: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- let x = Old(E);\n+ let x = New(E);\n";
    const LET_RESOLVED: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- let x = Old(E);\n+ let x = New(E);\n";

    #[test]
    fn wi4_let_init_fallback_fires_exact() {
        let src = "pub fn old_api(x: i32) -> i32 { x }\nfn f() {\n    let x = crate::old_api(1);\n    let _ = x;\n}\n";
        let (n, out) = run(LET_FALLBACK, src, None).expect("fallback fires");
        assert_eq!(n, 1);
        assert!(out.contains("let x = crate::new_api(1);"), "got:\n{out}");
    }

    #[test]
    fn wi4_let_init_fallback_short_spelling_misses() {
        // Exact full-path text under fallback (SPEC 19.3): a bare
        // `old_api` site with an import does not match.
        let src = "use crate::old_api;\nfn f() {\n    let x = old_api(1);\n    let _ = x;\n}\n";
        let res = run(LET_FALLBACK, src, None);
        assert!(res.is_ok(), "compiles and runs, matching nothing");
        assert_eq!(res.unwrap().0, 0);
    }

    #[test]
    fn wi4_let_init_resolved_fires_via_import() {
        let files: HashMap<String, String> = [
            (
                "src/lib.rs".to_string(),
                "pub mod a;\npub fn old_api(x: i32) -> i32 { x }\npub fn new_api(x: i32) -> i32 { x }\n".to_string(),
            ),
            (
                "src/a.rs".to_string(),
                "use crate::old_api;\nfn f() {\n    let x = old_api(1);\n    let _ = x;\n}\n".to_string(),
            ),
        ]
        .into_iter()
        .collect();
        let graph = crate::CrateGraph::build(&files);
        let src = files["src/a.rs"].clone();
        let (n, out) = run(LET_RESOLVED, &src, Some((&graph, "src/a.rs"))).expect("resolved fires");
        assert_eq!(n, 1);
        assert!(out.contains("let x = crate::new_api(1);"), "got:\n{out}");
    }

    #[test]
    fn wi4_let_init_resolved_vendored_copy_skipped() {
        let files: HashMap<String, String> = [
            (
                "src/lib.rs".to_string(),
                "pub mod vendored;\npub fn old_api(x: i32) -> i32 { x }\n".to_string(),
            ),
            (
                "src/vendored.rs".to_string(),
                "pub fn old_api(x: i32) -> i32 { x }\nfn g() {\n    let x = old_api(1);\n    let _ = x;\n}\n".to_string(),
            ),
        ]
        .into_iter()
        .collect();
        let graph = crate::CrateGraph::build(&files);
        let src = files["src/vendored.rs"].clone();
        let (n, _) = run(LET_RESOLVED, &src, Some((&graph, "src/vendored.rs")))
            .expect("runs, matching nothing");
        assert_eq!(n, 0, "vendored copy must not match");
    }

    #[test]
    fn wi4_unresolvable_seed_fails_loud() {
        let files: HashMap<String, String> =
            [("src/lib.rs".to_string(), "fn f() {}\n".to_string())]
                .into_iter()
                .collect();
        let graph = crate::CrateGraph::build(&files);
        let src = "fn f() {\n    let x = crate::missing(1);\n    let _ = x;\n}\n";
        let err = run(LET_RESOLVED, src, Some((&graph, "src/lib.rs"))).expect_err("must fail");
        assert_eq!(err.code, rsmpl_patch::ErrorCode::SymbolConflict);
        assert!(
            err.message.contains("resolves to no definition"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn wi4_general_type_fires() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# root=type\n# fallback=syntax\n# symbol Old=crate::Old\n# symbol New=crate::New\n- Old\n+ New\n";
        let src = "pub struct Old;\npub struct New;\nfn f(x: crate::Old) -> crate::Old { x }\n";
        let (n, out) = run(patch, src, None).expect("type fires");
        assert_eq!(n, 2, "annotation + return type");
        assert!(
            out.contains("fn f(x: crate::New) -> crate::New"),
            "got:\n{out}"
        );
    }

    #[test]
    fn wi4_nested_generic_type_fires() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# root=type\n# fallback=syntax\n# symbol Old=crate::Old\n# symbol New=crate::New\n- Vec<Old>\n+ Vec<New>\n";
        let src =
            "pub struct Old;\npub struct New;\nfn f(x: Vec<crate::Old>) -> Vec<crate::Old> { x }\n";
        let (n, out) = run(patch, src, None).expect("nested type fires");
        assert_eq!(n, 2);
        assert!(
            out.contains("fn f(x: Vec<crate::New>) -> Vec<crate::New>"),
            "got:\n{out}"
        );
    }

    #[test]
    fn wi4_general_expr_callee_fires() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- E + Old(1)\n+ E + New(1)\n";
        let src = "fn f(a: i32) -> i32 {\n    let y = a + crate::old_api(1);\n    y\n}\n";
        let (n, out) = run(patch, src, None).expect("expr fires");
        assert_eq!(n, 1);
        assert!(out.contains("a + crate::new_api(1)"), "got:\n{out}");
    }
}

#[cfg(test)]
mod wi4_slice1_plus_only_tests {
    // Plus-only symbols name the rewrite target (never verified): the
    // minus side matches concretely and the plus emits the seed.
    use super::apply_general_hunk;
    use crate::edits::TextEdit;

    #[test]
    fn wi4_plus_only_symbol_emits_seed() {
        let patch_text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# symbol New=crate::new_api\n# expr E\n- let x = E;\n+ let x = New(E);\n";
        let mut parsed_patch = rsmpl_patch::parse_patch(patch_text).expect("parses");
        let hunk = match parsed_patch.files.remove(0).hunks.remove(0) {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl"),
        };
        let compiled = rsmpl_pattern::compile(&hunk).expect("plus-only compiles");
        let src = "fn f() {\n    let x = foo(1);\n    let _ = x;\n}\n";
        let parsed: syn::File = syn::parse_str(src).expect("src parses");
        let index = crate::lines::LineIndex::new(src);
        let mut edits = Vec::<TextEdit>::new();
        // Bare symbol needs a graph present (same law as call hunks),
        // but the plus-only seed is never resolved — an empty graph
        // suffices and the minus side matches concretely.
        let files: std::collections::HashMap<String, String> =
            [("src/lib.rs".to_string(), "fn f() {}\n".to_string())]
                .into_iter()
                .collect();
        let graph = crate::CrateGraph::build(&files);
        let (n, _) = apply_general_hunk(
            &parsed,
            src,
            &index,
            &hunk,
            &compiled,
            &mut edits,
            Some((&graph, "src/lib.rs")),
        )
        .expect("plus-only fires");
        assert_eq!(n, 1);
        let out = crate::edits::apply_edits(src, edits).expect("edits apply");
        assert!(
            out.contains("let x = crate::new_api(foo(1));"),
            "got:\n{out}"
        );
    }
}

#[cfg(test)]
mod wi4_slice2_tests {
    // WI-4 slice 2a: prefix (`Old::suffix`), stem turbofish
    // (`Old::<T>`), and nested-annotation (`Vec<Old>`) symbols in
    // General Stmt/Expr/Type hunks. Fallback is exact head+tail text;
    // resolved composes `seed_canon + "::" + suffix` through the crate
    // graph (impl methods indexed, vendored twins skipped).
    use std::collections::HashMap;

    use super::apply_general_hunk;
    use crate::edits::TextEdit;

    fn compiled_of(patch_text: &str) -> (rsmpl_patch::RsmplHunk, rsmpl_pattern::CompiledPattern) {
        let mut parsed_patch = rsmpl_patch::parse_patch(patch_text).expect("test patch parses");
        let hunk = match parsed_patch.files.remove(0).hunks.remove(0) {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&hunk).expect("test patch compiles");
        (hunk, compiled)
    }

    fn run(
        patch_text: &str,
        src: &str,
        graph: Option<(&crate::CrateGraph, &str)>,
    ) -> Result<(usize, String), rsmpl_patch::StructuredError> {
        let (hunk, compiled) = compiled_of(patch_text);
        let parsed: syn::File = syn::parse_str(src).expect("src parses");
        let index = crate::lines::LineIndex::new(src);
        let mut edits = Vec::<TextEdit>::new();
        let (n, _) = apply_general_hunk(&parsed, src, &index, &hunk, &compiled, &mut edits, graph)?;
        let out = crate::edits::apply_edits(src, edits).expect("edits apply");
        Ok((n, out))
    }

    fn graph_of(files: &[(&str, &str)]) -> (HashMap<String, String>, crate::CrateGraph) {
        let map: HashMap<String, String> = files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect();
        let graph = crate::CrateGraph::build(&map);
        (map, graph)
    }

    const MOD_SRC: &str = "pub mod oldm { pub fn foo() -> i32 { 1 } }\npub mod newm { pub fn foo() -> i32 { 1 } }\nuse crate::oldm;\nfn f() {\n    let x = oldm::foo();\n}\n";
    const PREFIX: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# symbol Old=crate::oldm\n# symbol New=crate::newm\n- let x = Old::foo();\n+ let x = New::foo();\n";

    #[test]
    fn wi4_prefix_resolved_short_spelling_fires() {
        let (_m, graph) = graph_of(&[("src/lib.rs", MOD_SRC)]);
        let (n, out) = run(PREFIX, MOD_SRC, Some((&graph, "src/lib.rs"))).expect("fires");
        assert_eq!(n, 1);
        assert!(out.contains("let x = crate::newm::foo();"), "got:\n{out}");
    }

    #[test]
    fn wi4_prefix_resolved_full_spelling_fires() {
        let src = "pub mod oldm { pub fn foo() -> i32 { 1 } }\npub mod newm { pub fn foo() -> i32 { 1 } }\nfn f() {\n    let x = crate::oldm::foo();\n}\n";
        let (_m, graph) = graph_of(&[("src/lib.rs", src)]);
        let (n, out) = run(PREFIX, src, Some((&graph, "src/lib.rs"))).expect("fires");
        assert_eq!(n, 1);
        assert!(out.contains("let x = crate::newm::foo();"), "got:\n{out}");
    }

    #[test]
    fn wi4_prefix_fallback_full_spelling_fires() {
        let src = "pub mod oldm { pub fn foo() -> i32 { 1 } }\npub mod newm { pub fn foo() -> i32 { 1 } }\nfn f() {\n    let x = crate::oldm::foo();\n}\n";
        let patch = PREFIX.replace("# symbol Old=", "# fallback=syntax\n# symbol Old=");
        let (n, out) = run(&patch, src, None).expect("fires");
        assert_eq!(n, 1);
        assert!(out.contains("let x = crate::newm::foo();"), "got:\n{out}");
    }

    #[test]
    fn wi4_prefix_fallback_short_spelling_misses() {
        // Exact head+tail text under fallback (SPEC 19.3): a short
        // spelling never equals the expanded seed path.
        let patch = PREFIX.replace("# symbol Old=", "# fallback=syntax\n# symbol Old=");
        let (n, _) = run(&patch, MOD_SRC, None).expect("runs, matching nothing");
        assert_eq!(n, 0);
    }

    #[test]
    fn wi4_prefix_wrong_suffix_misses() {
        let src = "pub mod oldm { pub fn foo() -> i32 { 1 } pub fn bar() -> i32 { 2 } }\npub mod newm { pub fn foo() -> i32 { 1 } }\nuse crate::oldm;\nfn f() {\n    let x = oldm::bar();\n}\n";
        let (_m, graph) = graph_of(&[("src/lib.rs", src)]);
        let (n, _) = run(PREFIX, src, Some((&graph, "src/lib.rs"))).expect("runs");
        assert_eq!(n, 0, "suffix `bar` is not `foo`");
    }

    #[test]
    fn wi4_prefix_vendored_twin_skipped() {
        // Same text, different definition: resolution proves the head
        // is not the seed, so the site is skipped, never rewritten.
        let src = "pub mod oldm { pub fn foo() -> i32 { 1 } }\npub mod newm { pub fn foo() -> i32 { 1 } }\npub mod vendored { pub fn foo() -> i32 { 2 } }\nuse crate::vendored;\nfn f() {\n    let x = vendored::foo();\n}\n";
        let (_m, graph) = graph_of(&[("src/lib.rs", src)]);
        let (n, _) = run(PREFIX, src, Some((&graph, "src/lib.rs"))).expect("runs");
        assert_eq!(n, 0, "vendored copy must not match");
    }

    #[test]
    fn wi4_prefix_method_resolved_via_impl_index() {
        // Associated-function prefix resolves through the impl-method
        // index (`crate::Old::make`).
        let src = "pub struct Old;\npub struct New;\nimpl Old { pub fn make(v: u8) -> u8 { v } }\nimpl New { pub fn make(v: u8) -> u8 { v } }\nuse crate::Old;\nfn f() {\n    let x = Old::make(1);\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# symbol Old=crate::Old\n# symbol New=crate::New\n# expr E\n- let x = Old::make(E);\n+ let x = New::make(E);\n";
        let (_m, graph) = graph_of(&[("src/lib.rs", src)]);
        let (n, out) = run(patch, src, Some((&graph, "src/lib.rs"))).expect("fires");
        assert_eq!(n, 1);
        assert!(out.contains("let x = crate::New::make(1);"), "got:\n{out}");
    }

    #[test]
    fn wi4_prefix_turbofish_hole_binds() {
        // Suffix turbofish with a hole: `T` binds, stem verifies.
        let src = "pub struct Old;\npub struct New;\nimpl Old { pub fn make<T>(v: T) -> T { v } }\nimpl New { pub fn make<T>(v: T) -> T { v } }\nuse crate::Old;\nfn f() {\n    let x = Old::make::<u8>(1);\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# symbol Old=crate::Old\n# symbol New=crate::New\n# expr E\n# type T\n- let x = Old::make::<T>(E);\n+ let x = New::make::<T>(E);\n";
        let (_m, graph) = graph_of(&[("src/lib.rs", src)]);
        let (n, out) = run(patch, src, Some((&graph, "src/lib.rs"))).expect("fires");
        assert_eq!(n, 1);
        assert!(
            out.contains("let x = crate::New::make::<u8>(1);"),
            "got:\n{out}"
        );
    }

    #[test]
    fn wi4_prefix_turbofish_parity_holds() {
        // An arg-free stem never matches a turbofish site and vice
        // versa (same parity as whole-path symbols).
        let src = "pub struct Old;\npub struct New;\nimpl Old { pub fn make<T>(v: T) -> T { v } }\nimpl New { pub fn make<T>(v: T) -> T { v } }\nuse crate::Old;\nfn f() {\n    let x = Old::make::<u8>(1);\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# symbol Old=crate::Old\n# symbol New=crate::New\n# expr E\n- let x = Old::make(E);\n+ let x = New::make(E);\n";
        let (_m, graph) = graph_of(&[("src/lib.rs", src)]);
        let (n, _) = run(patch, src, Some((&graph, "src/lib.rs"))).expect("runs");
        assert_eq!(n, 0, "turbofish site needs a turbofish pattern");
    }

    #[test]
    fn wi4_whole_path_stem_turbofish_fires() {
        // `Old::<T>` whole-path stem: full spelling under fallback.
        let src = "pub fn old_id<T>(v: T) -> T { v }\npub fn new_id<T>(v: T) -> T { v }\nfn f() {\n    let x = crate::old_id::<u8>(1);\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_id\n# symbol New=crate::new_id\n# expr E\n# type E2\n- let x = Old::<E2>(E);\n+ let x = New::<E2>(E);\n";
        let (n, out) = run(patch, src, None).expect("fires");
        assert_eq!(n, 1);
        assert!(
            out.contains("let x = crate::new_id::<u8>(1);"),
            "got:\n{out}"
        );
    }

    #[test]
    fn wi4_annot_nested_symbol_fires() {
        // `let x: Vec<Old>` — the C8 gate admits symbol-only
        // annotation types; the engine verifies structurally.
        let src = "pub struct Old;\npub struct New;\nfn f() {\n    let x: Vec<Old> = vec![];\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# symbol Old=crate::Old\n# symbol New=crate::New\n# expr E\n- let x: Vec<Old> = E;\n+ let x: Vec<New> = E;\n";
        let (_m, graph) = graph_of(&[("src/lib.rs", src)]);
        let (n, out) = run(patch, src, Some((&graph, "src/lib.rs"))).expect("fires");
        assert_eq!(n, 1);
        assert!(
            out.contains("let x: Vec<crate::New> = vec![];"),
            "got:\n{out}"
        );
    }

    #[test]
    fn wi4_struct_literal_path_pins_resolved() {
        // Whole-path struct-literal names already fired in slice 1 —
        // this pins the posture for the prefix family.
        let src = "pub struct Old { pub f: i32 }\npub struct New { pub f: i32 }\nfn f() {\n    let x = Old { f: 1 };\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n# symbol Old=crate::Old\n# symbol New=crate::New\n# expr E\n- let x = Old { f: E };\n+ let x = New { f: E };\n";
        let (_m, graph) = graph_of(&[("src/lib.rs", src)]);
        let (n, out) = run(patch, src, Some((&graph, "src/lib.rs"))).expect("fires");
        assert_eq!(n, 1);
        assert!(out.contains("let x = crate::New { f: 1 };"), "got:\n{out}");
    }

    #[test]
    fn wi4_prefix_unresolvable_seed_fails_loud() {
        let src = "fn f() {\n    let x = oldm::foo();\n}\n";
        let files: HashMap<String, String> =
            [("src/lib.rs".to_string(), "fn f() {}\n".to_string())]
                .into_iter()
                .collect();
        let graph = crate::CrateGraph::build(&files);
        let err = run(PREFIX, src, Some((&graph, "src/lib.rs"))).expect_err("must fail");
        assert_eq!(err.code, rsmpl_patch::ErrorCode::SymbolConflict);
    }
}
