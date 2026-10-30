//! Call / method-call expression matching and rewriting (SPEC 13, 17-18).
//!
//! Candidates are collected in preorder (outer before inner) so that
//! `# allow=many` implements outermost-wins single-pass semantics (SPEC
//! 17.6). Rewrites preserve formatting by reusing original source slices for
//! captured metavariables.

use std::cmp::Reverse;
use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{ArgPat, CompiledPattern, ExprPatternKind, PatternKind};

use crate::edits::{fragments_equal, TextEdit};
use crate::lines::LineIndex;

/// A call-like candidate with original byte range.
#[derive(Debug)]
struct Candidate<'a> {
    /// Whole-expression byte range in original text.
    start: usize,
    end: usize,
    start_line: u32,
    end_line: u32,
    callee_segments: Vec<String>,
    arg_texts: Vec<String>,
    /// Source-sliced generic args per callee segment (T145 structural
    /// path); empty vec for segments without turbofish.
    callee_arg_texts: Vec<Vec<String>>,
    /// Source-sliced generic args of the method turbofish (T176 structural
    /// path); `None` when the call site carries no turbofish.
    method_turbofish: Option<Vec<String>>,
    receiver_text: Option<String>,
    method_text: Option<String>,
    is_method: bool,
    _expr: &'a syn::Expr,
}

struct Collector<'a, 'b> {
    original: &'b str,
    index: &'b LineIndex,
    out: Vec<Candidate<'a>>,
}

impl<'a, 'b> Collector<'a, 'b> {
    fn span_range(&self, span: proc_macro2::Span) -> Option<(usize, usize)> {
        let s = span.start();
        let e = span.end();
        let start = self.index.offset(s.line as u32, s.column)?;
        let end = self.index.offset(e.line as u32, e.column)?;
        Some((start, end))
    }

    fn slice(&self, start: usize, end: usize) -> String {
        self.original.get(start..end).unwrap_or("").to_string()
    }
}

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

/// Source-sliced generic args per path segment (parallel to
/// `path_segments`); segments without `::<>` contribute an empty vec.
/// Real call targets always use the `::<>` form (a bare `id<T>(...)`
/// never parses as a call), so no colon2 marker is recorded.
fn callee_generic_args(collector: &Collector<'_, '_>, func: &syn::Expr) -> Vec<Vec<String>> {
    let syn::Expr::Path(p) = func else {
        return Vec::new();
    };
    p.path
        .segments
        .iter()
        .map(|seg| match &seg.arguments {
            syn::PathArguments::AngleBracketed(a) => a
                .args
                .iter()
                .map(|arg| collector.slice_opt(arg.span()))
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

/// Source-sliced generic args of a method-call turbofish (T176 structural
/// path, mirror of `callee_generic_args`); `None` when the site carries
/// no turbofish. Real method targets always use the `::<>` form.
fn method_generic_args(
    collector: &Collector<'_, '_>,
    m: &syn::ExprMethodCall,
) -> Option<Vec<String>> {
    m.turbofish.as_ref().map(|t| {
        t.args
            .iter()
            .map(|arg| collector.slice_opt(arg.span()))
            .collect()
    })
}

fn expr_path_segments(func: &syn::Expr) -> Option<Vec<String>> {
    if let syn::Expr::Path(p) = func {
        Some(path_segments(&p.path))
    } else {
        None
    }
}

impl<'ast, 'b> Visit<'ast> for Collector<'ast, 'b> {
    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        match node {
            syn::Expr::Call(call) => {
                if let Some(segs) = expr_path_segments(&call.func) {
                    if let Some((start, end)) = self.span_range(node.span()) {
                        let args: Vec<String> =
                            call.args.iter().map(|a| self.slice_opt(a.span())).collect();
                        self.out.push(Candidate {
                            start,
                            end,
                            start_line: node.span().start().line as u32,
                            end_line: node.span().end().line as u32,
                            callee_segments: segs,
                            arg_texts: args,
                            callee_arg_texts: callee_generic_args(self, &call.func),
                            method_turbofish: None,
                            receiver_text: None,
                            method_text: None,
                            is_method: false,
                            _expr: node,
                        });
                    }
                }
                // Always recurse (preorder already recorded outer first).
                visit::visit_expr(self, node);
            }
            syn::Expr::MethodCall(m) => {
                if let Some((start, end)) = self.span_range(node.span()) {
                    let args: Vec<String> =
                        m.args.iter().map(|a| self.slice_opt(a.span())).collect();
                    self.out.push(Candidate {
                        start,
                        end,
                        start_line: node.span().start().line as u32,
                        end_line: node.span().end().line as u32,
                        callee_segments: vec![m.method.to_string()],
                        arg_texts: args,
                        callee_arg_texts: Vec::new(),
                        method_turbofish: method_generic_args(self, m),
                        receiver_text: Some(self.slice_opt(m.receiver.span())),
                        method_text: Some(m.method.to_string()),
                        is_method: true,
                        _expr: node,
                    });
                }
                visit::visit_expr(self, node);
            }
            _ => visit::visit_expr(self, node),
        }
    }
}

impl<'a, 'b> Collector<'a, 'b> {
    fn slice_opt(&self, span: proc_macro2::Span) -> String {
        self.span_range(span)
            .map(|(s, e)| self.slice(s, e))
            .unwrap_or_default()
    }
}

fn in_scope(start_line: u32, end_line: u32, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    let hi = end.max(old_start);
    match scope {
        Scope::Contained => start_line >= old_start && end_line <= hi,
        Scope::Anchor => start_line >= old_start && start_line <= hi,
        Scope::Overlap => start_line <= hi && end_line >= old_start,
        Scope::Exact => old_len > 0 && start_line == old_start && end_line == end,
        Scope::File => true,
    }
}

/// Match pattern path segments (with `...` wildcards) against target segments.
fn match_path_segments(pattern: &[String], target: &[String]) -> bool {
    if !pattern.contains(&"...".to_string()) {
        return pattern == target;
    }
    // Single `...` anywhere: prefix/suffix matching with zero+ middle.
    // Multiple `...`: left-to-right greedy (documented v0.1 behavior).
    let mut pi = 0;
    let mut ti = 0;
    while pi < pattern.len() && ti < target.len() {
        if pattern[pi] == "..." {
            // Greedy: if rest of pattern matches a suffix, consume.
            let rest = &pattern[pi + 1..];
            if rest.is_empty() {
                return true;
            }
            if rest.contains(&"...".to_string()) {
                // Multiple wildcards: consume one target segment, advance.
                ti += 1;
                pi += 1;
            } else {
                // Last wildcard: target must end with `rest`.
                if target.len() < ti + rest.len() {
                    return false;
                }
                let tail = &target[target.len() - rest.len()..];
                return tail == rest;
            }
        } else if pattern[pi] == target[ti] {
            pi += 1;
            ti += 1;
        } else {
            return false;
        }
    }
    // Trailing wildcards match empty.
    while pi < pattern.len() && pattern[pi] == "..." {
        pi += 1;
    }
    pi == pattern.len() && ti == target.len()
}

/// Match pattern args against target arg texts, filling `bindings`.
/// Every `...` pushes the target args it absorbs (joined with `", "`,
/// possibly empty) onto `remainders`, positionally — N minus-`...`
/// yields N remainders for the plus-side replay.
/// Returns `false` when this candidate does not match.
fn match_args(
    pattern: &[ArgPat],
    targets: &[String],
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
    remainders: &mut Vec<String>,
) -> bool {
    if !pattern.contains(&ArgPat::Ellipsis) {
        if pattern.len() != targets.len() {
            return false;
        }
        for (p, t) in pattern.iter().zip(targets.iter()) {
            if !match_single_arg(p, t, reverse_map, bindings) {
                return false;
            }
        }
        return true;
    }
    // With ellipses: align with backtracking (T159). Splits are tried
    // left-to-right so greedy preference is preserved, but a failed tail
    // retries later splits instead of failing first-fit-only.
    match_args_from(pattern, 0, targets, 0, reverse_map, bindings, remainders)
}

/// Backtracking worker for [`match_args`]: match `pattern[pi..]` against
/// `targets[ti..]`. `remainders` gains one entry per `...` in pattern
/// order on success (pushed before recursing, popped on failure).
fn match_args_from(
    pattern: &[ArgPat],
    pi: usize,
    targets: &[String],
    ti: usize,
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
    remainders: &mut Vec<String>,
) -> bool {
    if pi == pattern.len() {
        // All pattern args consumed: success iff all targets consumed
        // (trailing targets need a trailing `...` to absorb them).
        return ti == targets.len();
    }
    if pattern[pi] == ArgPat::Ellipsis {
        // If last pattern element, absorb the rest.
        if pi + 1 == pattern.len() {
            remainders.push(targets[ti..].join(", "));
            return true;
        }
        for k in ti..=targets.len() {
            let saved = bindings.clone();
            remainders.push(targets[ti..k].join(", "));
            if match_args_from(
                pattern,
                pi + 1,
                targets,
                k,
                reverse_map,
                bindings,
                remainders,
            ) {
                return true;
            }
            remainders.pop();
            *bindings = saved;
        }
        return false;
    }
    if ti >= targets.len() {
        return false;
    }
    let saved = bindings.clone();
    if !match_single_arg(&pattern[pi], &targets[ti], reverse_map, bindings) {
        return false;
    }
    if match_args_from(
        pattern,
        pi + 1,
        targets,
        ti + 1,
        reverse_map,
        bindings,
        remainders,
    ) {
        return true;
    }
    *bindings = saved;
    false
}

fn match_single_arg(
    pattern: &ArgPat,
    target: &str,
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
) -> bool {
    match pattern {
        ArgPat::Ellipsis => true,
        ArgPat::Meta(ph) => {
            // `ph` is the placeholder or the original name; resolve.
            let name = reverse_map.get(ph).cloned().unwrap_or_else(|| ph.clone());
            let name = name.trim_start_matches('$').to_string();
            if let Some(prev) = bindings.get(&name) {
                fragments_equal(prev, target)
            } else {
                bindings.insert(name, target.to_string());
                true
            }
        }
        ArgPat::Concrete(c) => fragments_equal(c, target),
    }
}

/// Substitute bound metavariables and `...` into the plus pattern.
/// Each standalone `...` replays, in order, the target args the matching
/// `- ...` absorbed (T157: the whole arg list is NOT replayed). An empty
/// remainder swallows one adjacent comma so `bar(E, ...)` on `foo(1)`
/// yields `bar(1)`, not `bar(1, )`. Surplus `...` (unreachable after the
/// plus/minus count gate) replay empty.
pub fn substitute_plus(
    plus_raw: &str,
    bindings: &HashMap<String, String>,
    ellipses: &[String],
    sigiled: bool,
) -> String {
    // Replace standalone `...` first (not part of longer dot runs).
    let mut out = replace_ellipsis_tokens(plus_raw, ellipses);
    // Longest names first to avoid prefix collisions.
    let mut names: Vec<&String> = bindings.keys().collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in names {
        let cap = &bindings[name];
        if sigiled {
            out = replace_ident_token(&out, &format!("${name}"), cap);
        } else {
            out = replace_ident_token(&out, name, cap);
            out = replace_ident_token(&out, &format!("${name}"), cap);
        }
    }
    out
}

/// Substitute into a call-shape plus pattern, protecting backtick
/// literals (SPEC 15.4): mask → `substitute_plus` → unmask, mirroring
/// `substitute_general_plus`. Sentinels are ident-shaped and never equal
/// a declared name, so substitution cannot touch them; the restored
/// literal then emits verbatim. All call/method-call plus sites
/// (normal, `# repeat` fold, nested fold) must use this, never raw
/// `substitute_plus` on backticked text.
pub fn substitute_call_plus(
    plus_raw: &str,
    bindings: &HashMap<String, String>,
    ellipses: &[String],
    sigiled: bool,
) -> String {
    let (masked, sentinels) = match rsmpl_pattern::mask_literal_spans(plus_raw) {
        Ok(v) => v,
        // Unbalanced backticks are rejected at parse time; fall back to
        // raw substitution rather than failing the whole rewrite here.
        Err(_) => return substitute_plus(plus_raw, bindings, ellipses, sigiled),
    };
    let out = substitute_plus(&masked, bindings, ellipses, sigiled);
    rsmpl_pattern::unmask_literal_spans(&out, &sentinels)
}

fn replace_ellipsis_tokens(text: &str, replacements: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut n = 0;
    while i < bytes.len() {
        if i + 2 < bytes.len() && &text[i..i + 3] == "..." {
            let before_ok = i == 0 || bytes[i - 1] != b'.';
            let after_ok = i + 3 >= bytes.len() || bytes[i + 3] != b'.';
            if before_ok && after_ok {
                let rep = replacements.get(n).map(String::as_str).unwrap_or("");
                n += 1;
                if rep.is_empty() {
                    // Swallow one adjacent comma (preceding preferred) so
                    // empty replays leave no `, )` / `( ,` artifact.
                    if let Some(pos) = last_comma_before(&out) {
                        out.truncate(pos);
                    } else {
                        i += 3;
                        let mut j = i;
                        while j < bytes.len()
                            && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n')
                        {
                            j += 1;
                        }
                        if j < bytes.len() && bytes[j] == b',' {
                            j += 1;
                            while j < bytes.len()
                                && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n')
                            {
                                j += 1;
                            }
                        }
                        i = j;
                        continue;
                    }
                } else {
                    out.push_str(rep);
                }
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Byte index of the trailing comma (skipping trailing whitespace) in
/// `out`, or `None` when the pre-ellipsis text does not end with one.
fn last_comma_before(out: &str) -> Option<usize> {
    let bytes = out.as_bytes();
    let mut j = bytes.len();
    while j > 0 && (bytes[j - 1] == b' ' || bytes[j - 1] == b'\t' || bytes[j - 1] == b'\n') {
        j -= 1;
    }
    (j > 0 && bytes[j - 1] == b',').then_some(j - 1)
}

/// Count standalone `...` tokens (same definition as the substitution
/// pass: not part of longer dot runs).
fn count_ellipsis_tokens(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        if i + 2 < bytes.len() && &text[i..i + 3] == "..." {
            let before_ok = i == 0 || bytes[i - 1] != b'.';
            let after_ok = i + 3 >= bytes.len() || bytes[i + 3] != b'.';
            if before_ok && after_ok {
                count += 1;
                i += 3;
                continue;
            }
        }
        i += 1;
    }
    count
}

/// True when the text uses `::...::` path ellipsis (SPEC 16.5), as opposed
/// to argument/body ellipsis `foo(...)`. Local copy of the pattern-crate
/// predicate (which is private there).
fn plus_has_path_ellipsis(text: &str) -> bool {
    text.contains("::...") || text.contains("...::")
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn replace_ident_token(text: &str, name: &str, replacement: &str) -> String {
    if name.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if let Some(rest) = text[i..].strip_prefix(name) {
            let before_ok = i == 0 || !is_ident_char(text[..i].chars().last().unwrap_or(' '));
            let after_ok = rest
                .chars()
                .next()
                .is_none_or(|c| !is_ident_char(c) && c != '$');
            // For `$Name` patterns the char after is part of matching.
            if before_ok && after_ok {
                out.push_str(replacement);
                i += name.len();
                continue;
            }
        }
        out.push(text[i..].chars().next().unwrap());
        i += text[i..].chars().next().unwrap().len_utf8();
    }
    out
}

/// Apply an expression/call rewrite hunk. Returns match count.
pub fn apply_expr_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (minus, plus_raw, _ph, _lits, _has_path) = match &compiled.kind {
        PatternKind::Expr {
            minus,
            plus_raw,
            placeholders,
            literals,
            has_path_ellipsis,
        } => (minus, plus_raw, placeholders, literals, has_path_ellipsis),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected expression pattern".to_string(),
            ))
        }
    };
    let rule = hunk.rule_name();

    // T157: a `...` in `+` replays the args the matching `- ...` absorbed,
    // so more plus-`...` than minus-`...` has nothing to replay (the old
    // whole-arg-list substitution silently duplicated instead). Fail fast.
    // Fewer is a deliberate arg-drop. Covers the nested `# repeat` path too
    // (same minus/plus pair).
    let minus_ellipses: usize = match minus {
        ExprPatternKind::Call { args, .. } | ExprPatternKind::MethodCall { args, .. } => {
            args.iter().filter(|a| **a == ArgPat::Ellipsis).count()
        }
        ExprPatternKind::Macro { .. } | ExprPatternKind::Raw { .. } => 0,
    };
    if count_ellipsis_tokens(plus_raw) > minus_ellipses {
        // T166: a `+` path `...` (SPEC 16.5) never replays — only argument
        // `...` replays (SPEC 16.3) — so it trips this count gate even when
        // the minus side HAS a path `...`. Name the path case explicitly.
        if plus_has_path_ellipsis(plus_raw) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "`...` in `+` path has nothing to replay (plus side must be concrete, SPEC 16.5)".to_string(),
            )
            .with_rule(&rule)
            .with_suggestion("write a concrete `+` path (e.g. `+ crate::new(E)`) — path `...` replays nothing, only argument `...` replays")
            .with_repair(StructuredError::fix_pattern_repair(
                "ellipsis",
                "write a concrete `+` path (e.g. `+ crate::new(E)`)",
            )));
        }
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`...` in `+` has no matching `- ...` to replay".to_string(),
        )
        .with_rule(&rule)
        .with_suggestion("drop the `+ ...` or add a `- ...` absorbing the replayed args")
        .with_repair(StructuredError::fix_pattern_repair(
            "ellipsis",
            "drop the `+ ...` or add a `- ...` absorbing the replayed args",
        )));
    }

    // Symbol gate (SPEC 19.3): without a semantic environment, fail unless
    // `# fallback=syntax` is set.
    let fallback_syntax = hunk
        .directives
        .iter()
        .any(|d| matches!(d, rsmpl_patch::Directive::FallbackSyntax(true)));
    for sym in compiled.table.symbols.keys() {
        if !fallback_syntax {
            return Err(StructuredError::new(
                ErrorCode::SymbolConflict,
                format!("symbol `{sym}` could not be resolved (no semantic environment in v0.1)"),
            )
            .with_rule(&rule)
            .with_suggestion("add `# fallback=syntax` to allow syntactic matching")
            .with_repair(StructuredError::add_directive_repair("# fallback=syntax")));
        }
    }

    let mut collector = Collector {
        original,
        index,
        out: Vec::new(),
    };
    collector.visit_file(parsed);
    let mut cands = collector.out;
    // Preorder (outer first) + source order: syn Visit is already preorder,
    // but sort defensively by byte offset, longest-first on a shared start
    // (T167: receiver nesting shares the start; mirrors T143).
    cands.sort_by_key(|c| (c.start, Reverse(c.end)));

    let reverse_map: HashMap<String, String> = build_reverse_map(compiled);

    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut matched = 0;
    let mut all_bindings: Vec<HashMap<String, String>> = Vec::new();
    for cand in &cands {
        // Outermost-wins: skip descendants of accepted matches.
        if accepted
            .iter()
            .any(|&(s, e)| s <= cand.start && cand.end <= e)
        {
            continue;
        }
        if !in_scope(
            cand.start_line,
            cand.end_line,
            hunk.scope(),
            hunk.header.old_start,
            hunk.header.old_len,
        ) {
            continue;
        }
        let (mut bindings, remainders) = match match_candidate(minus, cand, compiled, &reverse_map)
        {
            Some(b) => b,
            None => continue,
        };
        // Partial-overlap guard (SPEC 17.6.5).
        for &(s, e) in &accepted {
            let overlap = cand.start < e && s < cand.end;
            let nested = (s <= cand.start && cand.end <= e) || (cand.start <= s && e <= cand.end);
            if overlap && !nested {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "partially overlapping matches cannot be merged",
                )
                .with_rule(&rule));
            }
        }
        // T20 `# repeat`: fold nested matches by rewriting each bound
        // fragment with the same rule before substitution (bottom-up).
        if hunk.repeats() {
            fold_bindings(
                &mut bindings,
                compiled,
                minus,
                plus_raw,
                &reverse_map,
                0,
                &rule,
            )?;
        }
        // Build replacement. Under `# repeat` the callee-only fast path
        // is skipped: it splices the original slice and would discard
        // folded bindings.
        let replacement = if hunk.repeats() {
            substitute_call_plus(plus_raw, &bindings, &remainders, compiled.table.sigiled)
        } else {
            build_replacement(
                minus,
                plus_raw,
                cand,
                original,
                &bindings,
                &remainders,
                &reverse_map,
                compiled.table.sigiled,
            )
        };
        edits.push(TextEdit::new(cand.start, cand.end, replacement));
        accepted.push((cand.start, cand.end));
        all_bindings.push(bindings.clone());
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break;
        }
    }
    Ok((matched, all_bindings))
}

/// Backstop for `# repeat` nesting. Bindings are strict sub-slices of the
/// matched candidate, so structural descent terminates on its own; this
/// only guards pathological spans.
const MAX_REPEAT_DEPTH: usize = 64;

/// Match one call/method-call candidate, returning its bindings
/// (symbol seeds included) plus the per-`...` absorbed remainders for
/// plus-side replay. `None` means this candidate does not match.
fn match_candidate(
    minus: &ExprPatternKind,
    cand: &Candidate<'_>,
    compiled: &CompiledPattern,
    reverse_map: &HashMap<String, String>,
) -> Option<(HashMap<String, String>, Vec<String>)> {
    let mut bindings: HashMap<String, String> = HashMap::new();
    // Seed symbol bindings (symbols are pre-bound to their path).
    for (n, p) in &compiled.table.symbols {
        bindings.insert(n.clone(), p.clone());
    }
    let mut remainders: Vec<String> = Vec::new();
    let ok = match minus {
        ExprPatternKind::Call { callee, args } => {
            !cand.is_method
                && match_callee(callee, cand, compiled, &mut bindings)
                && match_args(
                    args,
                    &cand.arg_texts,
                    reverse_map,
                    &mut bindings,
                    &mut remainders,
                )
        }
        ExprPatternKind::MethodCall {
            receiver,
            method,
            args,
        } => {
            cand.is_method
                && match_receiver_or_method(receiver, method, cand, reverse_map, &mut bindings)
                && match_args(
                    args,
                    &cand.arg_texts,
                    reverse_map,
                    &mut bindings,
                    &mut remainders,
                )
        }
        ExprPatternKind::Macro { .. } => false, // routed to macro matcher
        ExprPatternKind::Raw { .. } => false,
    };
    ok.then_some((bindings, remainders))
}

/// Build the replacement text for a matched candidate: the callee-only
/// fast path when just the callee changed, else full substitution.
#[allow(clippy::too_many_arguments)]
fn build_replacement(
    minus: &ExprPatternKind,
    plus_raw: &str,
    cand: &Candidate<'_>,
    source: &str,
    bindings: &HashMap<String, String>,
    remainders: &[String],
    reverse_map: &HashMap<String, String>,
    sigiled: bool,
) -> String {
    if let Some(fast) = try_callee_only_rewrite(minus, plus_raw, cand, source, reverse_map) {
        fast
    } else {
        substitute_call_plus(plus_raw, bindings, remainders, sigiled)
    }
}

/// T20 `# repeat`: rewrite every non-symbol binding with the same rule
/// (bottom-up fold). Symbol seeds are pre-bound paths, not matchable
/// fragments, so they are skipped.
fn fold_bindings(
    bindings: &mut HashMap<String, String>,
    compiled: &CompiledPattern,
    minus: &ExprPatternKind,
    plus_raw: &str,
    reverse_map: &HashMap<String, String>,
    depth: usize,
    rule: &str,
) -> Result<(), StructuredError> {
    if depth > MAX_REPEAT_DEPTH {
        return Err(StructuredError::new(
            ErrorCode::MatchConflict,
            "repeat nesting exceeds 64 levels (pathological nesting?)",
        )
        .with_rule(rule)
        .with_suggestion("remove `# repeat` or simplify the nesting"));
    }
    // Snapshot names: rewriting mutates the map.
    let names: Vec<String> = bindings.keys().cloned().collect();
    for name in names {
        if compiled.table.symbols.contains_key(&name) {
            continue;
        }
        if let Some(current) = bindings.get(&name).cloned() {
            let rewritten = rewrite_nested(
                &current,
                compiled,
                minus,
                plus_raw,
                reverse_map,
                depth,
                rule,
            )?;
            bindings.insert(name, rewritten);
        }
    }
    Ok(())
}

/// Re-apply the same call rewrite inside one binding fragment: collect
/// candidates over the fragment, rewrite outermost-first with recursive
/// descent into sub-bindings, and splice the result. The produced output
/// is never re-scanned, so self-reproducing rules
/// (`foo(E)` -> `foo(foo(E))`) terminate after one level by construction.
/// Unparseable fragments are kept verbatim (conservative).
fn rewrite_nested(
    text: &str,
    compiled: &CompiledPattern,
    minus: &ExprPatternKind,
    plus_raw: &str,
    reverse_map: &HashMap<String, String>,
    depth: usize,
    rule: &str,
) -> Result<String, StructuredError> {
    let expr: syn::Expr = match syn::parse_str(text) {
        Ok(e) => e,
        Err(_) => return Ok(text.to_string()),
    };
    let index = LineIndex::new(text);
    let mut collector = Collector {
        original: text,
        index: &index,
        out: Vec::new(),
    };
    collector.visit_expr(&expr);
    let mut cands = collector.out;
    // Longest-first on a shared start (T167, same as the file-level pass).
    cands.sort_by_key(|c| (c.start, Reverse(c.end)));
    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut edits: Vec<TextEdit> = Vec::new();
    for cand in &cands {
        // Outermost-wins, like the file-level pass (SPEC 17.6).
        if accepted
            .iter()
            .any(|&(s, e)| s <= cand.start && cand.end <= e)
        {
            continue;
        }
        let (mut bindings, remainders) = match match_candidate(minus, cand, compiled, reverse_map) {
            Some(b) => b,
            None => continue,
        };
        for &(s, e) in &accepted {
            let overlap = cand.start < e && s < cand.end;
            let nested = (s <= cand.start && cand.end <= e) || (cand.start <= s && e <= cand.end);
            if overlap && !nested {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "partially overlapping matches cannot be merged",
                )
                .with_rule(rule));
            }
        }
        fold_bindings(
            &mut bindings,
            compiled,
            minus,
            plus_raw,
            reverse_map,
            depth + 1,
            rule,
        )?;
        // Nested folds always substitute: the callee-only fast path
        // would splice the unfolded slice and discard folded bindings.
        let replacement =
            substitute_call_plus(plus_raw, &bindings, &remainders, compiled.table.sigiled);
        edits.push(TextEdit::new(cand.start, cand.end, replacement));
        accepted.push((cand.start, cand.end));
    }
    if edits.is_empty() {
        return Ok(text.to_string());
    }
    crate::edits::apply_edits(text, edits)
}

fn build_reverse_map(compiled: &CompiledPattern) -> HashMap<String, String> {
    // placeholder -> original name, for Meta args stored as placeholders.
    let mut m = HashMap::new();
    if let PatternKind::Expr { placeholders, .. } = &compiled.kind {
        for (name, ph) in placeholders {
            m.insert(ph.clone(), name.clone());
        }
    }
    m
}

/// Match the callee path pattern against target segments. A declared
/// metavariable binds the whole path text; a symbol name additionally requires
/// full-path text equality with the seeded symbol path (exact syntactic
/// fallback, SPEC 19.3): symbols are pre-bound to their declared path, so
/// only identically-spelled callees match — the last-segment gate below is
/// subsumed by that equality check.
fn match_callee(
    callee: &str,
    cand: &Candidate<'_>,
    compiled: &CompiledPattern,
    bindings: &mut HashMap<String, String>,
) -> bool {
    let reverse_map = build_reverse_map(compiled);
    // T145: a pattern callee carrying `<...>` takes the structural path
    // (the ident-only text comparison below could never match it — the
    // target segments drop generic args, so `id::<T>` always failed).
    // Callees without `<` keep the exact historical behavior.
    if callee.contains('<') {
        return match_callee_turbofish(
            callee,
            cand,
            &reverse_map,
            &compiled.table.symbols,
            bindings,
        );
    }
    let target_segments = &cand.callee_segments;
    // T177: a bare pattern (no `::<...>`) never matches a turbofish site
    // (None-vs-Some fails, general-matcher parity) — spell `::<T>` explicitly.
    if cand.callee_arg_texts.iter().any(|v| !v.is_empty()) {
        return false;
    }
    // Placeholder-generated names look like `__RSMPL_E`; map back via the
    // declared-name table. Anything still shaped like a placeholder with no
    // entry is treated as concrete (shouldn't happen).
    if let Some(orig) = reverse_map.get(callee) {
        if let Some(sym_path) = compiled.table.symbols.get(orig) {
            let want_last = sym_path.rsplit("::").next().unwrap_or(sym_path);
            let got_last = target_segments.last().map(|s| s.as_str()).unwrap_or("");
            if got_last != want_last {
                return false;
            }
            // Bind the full path text; repeated occurrences must agree.
            let text = target_segments.join("::");
            if let Some(prev) = bindings.get(orig) {
                return prev == &text;
            }
            bindings.insert(orig.clone(), text);
            return true;
        }
        // Plain metavariable as whole callee: binds any path.
        let text = target_segments.join("::");
        if let Some(prev) = bindings.get(orig) {
            return prev == &text;
        }
        bindings.insert(orig.clone(), text);
        return true;
    }
    // Concrete (possibly with `...` wildcards).
    let pat_segments: Vec<String> = callee.split("::").map(|s| s.to_string()).collect();
    match_path_segments(&pat_segments, target_segments)
}

/// T145: structural function-call turbofish comparison (mirror of T144's
/// `MethodCall` arm). The pattern callee parses as a `syn::Path`
/// (placeholders are ordinary idents post-substitution); target segments
/// pair with their source-sliced generic args from collection.
fn match_callee_turbofish(
    pat_callee: &str,
    cand: &Candidate<'_>,
    reverse_map: &HashMap<String, String>,
    symbols: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
) -> bool {
    let pat_path: syn::Path = match syn::parse_str(pat_callee) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pat_path.leading_colon.is_some() {
        return false;
    }
    let mut segs: Vec<syn::PathSegment> = pat_path.segments.into_iter().collect();
    // T199: a sole symbol ident with turbofish (`Old::<u8>` with
    // `# symbol Old=crate::old_api`) expands to its seeded path
    // segments; seeded idents compare by full-path equality (SPEC 19.3,
    // same rule as the bare-symbol arm) while the turbofish args compare
    // structurally below. Symbols are pre-bound, so no binding is added.
    // A placeholder-mapped ident keeps placeholder behavior (the bare
    // arm checks the reverse map first, too).
    if segs.len() == 1 {
        let ident = segs[0].ident.to_string();
        // A symbol name is also placeholder-substituted, so resolve
        // through the reverse map first (mirrors the bare-symbol arm).
        // A plain placeholder has no symbols entry and keeps its
        // behavior.
        if let Some(orig) = reverse_map.get(&ident) {
            if let Some(seed) = symbols.get(orig) {
                let seed_path: syn::Path = match syn::parse_str(seed) {
                    Ok(p) => p,
                    Err(_) => return false,
                };
                if seed_path.leading_colon.is_some() {
                    return false;
                }
                let mut seed_segs: Vec<_> = seed_path.segments.into_iter().collect();
                if let Some(last) = seed_segs.last_mut() {
                    last.arguments = segs[0].arguments.clone();
                }
                segs = seed_segs;
            }
        }
    }
    if segs.len() != cand.callee_segments.len() {
        return false;
    }
    for ((pat_seg, tgt_ident), tgt_args) in segs
        .iter()
        .zip(cand.callee_segments.iter())
        .zip(cand.callee_arg_texts.iter())
    {
        if pat_seg.ident == "..." {
            return false;
        }
        if let Some(orig) = reverse_map.get(&pat_seg.ident.to_string()) {
            let prev_ok = match bindings.get(orig) {
                Some(prev) => prev == tgt_ident,
                None => {
                    bindings.insert(orig.clone(), tgt_ident.clone());
                    true
                }
            };
            if !prev_ok {
                return false;
            }
            // T144 bare-placeholder parity: no-arg pattern skips the
            // args check; `P::<T>` compares structurally.
            if matches!(pat_seg.arguments, syn::PathArguments::None) {
                continue;
            }
        } else {
            if pat_seg.ident != *tgt_ident {
                return false;
            }
            if matches!(pat_seg.arguments, syn::PathArguments::None) && tgt_args.is_empty() {
                continue;
            }
        }
        let syn::PathArguments::AngleBracketed(pat_bracketed) = &pat_seg.arguments else {
            return false;
        };
        // A bare `id<T>` pattern never matches a real `id::<u8>` call
        // (it never did — a non-turbofish call site cannot exist).
        if pat_bracketed.colon2_token.is_none() {
            return false;
        }
        if pat_bracketed.args.len() != tgt_args.len() {
            return false;
        }
        for (pat_arg, tgt_text) in pat_bracketed.args.iter().zip(tgt_args.iter()) {
            if !match_callee_generic_arg(pat_arg, tgt_text, reverse_map, bindings) {
                return false;
            }
        }
    }
    true
}

/// One generic argument of a pattern turbofish against target source
/// text: single-placeholder types bind; lifetimes wildcard; everything
/// else (concrete types, consts) compares token-normalized.
fn match_callee_generic_arg(
    pat_arg: &syn::GenericArgument,
    tgt_text: &str,
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
) -> bool {
    if let Some(orig) = single_placeholder_arg_type(pat_arg, reverse_map) {
        return match bindings.get(&orig) {
            Some(prev) => fragments_equal(prev, tgt_text),
            None => {
                bindings.insert(orig, tgt_text.to_string());
                true
            }
        };
    }
    if let syn::GenericArgument::Lifetime(_) = pat_arg {
        if syn::parse_str::<syn::Lifetime>(tgt_text).is_ok() {
            return true;
        }
    }
    fragments_equal(&quote::quote!(#pat_arg).to_string(), tgt_text)
}

/// Declared name if `arg` is a single-placeholder type (`T` for `# type T`).
fn single_placeholder_arg_type(
    arg: &syn::GenericArgument,
    reverse_map: &HashMap<String, String>,
) -> Option<String> {
    let syn::GenericArgument::Type(syn::Type::Path(tp)) = arg else {
        return None;
    };
    if tp.qself.is_some()
        || tp.path.segments.len() != 1
        || !matches!(tp.path.segments[0].arguments, syn::PathArguments::None)
    {
        return None;
    }
    reverse_map
        .get(&tp.path.segments[0].ident.to_string())
        .cloned()
}

fn match_receiver_or_method(
    receiver_pat: &str,
    method_pat: &str,
    cand: &Candidate<'_>,
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
) -> bool {
    // Receiver: placeholder/meta binds any receiver; concrete must be equal.
    let recv_ok = if let Some(orig) = reverse_map.get(receiver_pat) {
        let text = cand.receiver_text.clone().unwrap_or_default();
        match bindings.get(orig) {
            Some(prev) => fragments_equal(prev, &text),
            None => {
                bindings.insert(orig.clone(), text);
                true
            }
        }
    } else {
        fragments_equal(
            receiver_pat,
            &cand.receiver_text.clone().unwrap_or_default(),
        )
    };
    if !recv_ok {
        return false;
    }
    // Method: turbofish presence must agree (T177, general-matcher parity):
    // a bare pattern never matches a turbofish site and vice versa. `<`-
    // carrying patterns compare structurally (T176, mirror of T145).
    if let Some(orig) = reverse_map.get(method_pat) {
        if cand.method_turbofish.is_some() {
            return false;
        }
        let text = cand.method_text.clone().unwrap_or_default();
        match bindings.get(orig) {
            Some(prev) => prev == &text,
            None => {
                bindings.insert(orig.clone(), text);
                true
            }
        }
    } else if method_pat.contains('<') {
        match_method_turbofish(method_pat, cand, reverse_map, bindings)
    } else {
        if cand.method_turbofish.is_some() {
            return false;
        }
        method_pat == cand.method_text.clone().unwrap_or_default()
    }
}
/// T176: structural method-call turbofish comparison (mirror of T145's
/// `match_callee_turbofish`). The pattern method parses as a `syn::Path`
/// with one segment (placeholders are ordinary idents post-substitution);
/// the ident binds-or-equals (bare-`P` parity) and each generic arg
/// compares via `match_callee_generic_arg` (so `collect::<T>` binds `T`).
/// A bare `id<T>` pattern (no `::`) never matches, mirroring T145.
fn match_method_turbofish(
    method_pat: &str,
    cand: &Candidate<'_>,
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
) -> bool {
    let pat_path: syn::Path = match syn::parse_str(method_pat) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if pat_path.leading_colon.is_some() || pat_path.segments.len() != 1 {
        return false;
    }
    let seg = &pat_path.segments[0];
    if seg.ident == "..." {
        return false;
    }
    let tgt_ident = cand.method_text.clone().unwrap_or_default();
    if let Some(orig) = reverse_map.get(&seg.ident.to_string()) {
        match bindings.get(orig) {
            Some(prev) => {
                if prev != &tgt_ident {
                    return false;
                }
            }
            None => {
                bindings.insert(orig.clone(), tgt_ident);
            }
        }
        // Bare-placeholder method skips the args check (T144/T145 parity).
        if matches!(seg.arguments, syn::PathArguments::None) {
            return true;
        }
    } else {
        if seg.ident != tgt_ident {
            return false;
        }
        if matches!(seg.arguments, syn::PathArguments::None) {
            // T177: unreachable via the router (bare patterns never route
            // here), but a bare segment must never match a turbofish site.
            return false;
        }
    }
    let syn::PathArguments::AngleBracketed(pat_bracketed) = &seg.arguments else {
        return false;
    };
    if pat_bracketed.colon2_token.is_none() {
        return false;
    }
    let Some(tgt_args) = cand.method_turbofish.as_ref() else {
        return false;
    };
    if pat_bracketed.args.len() != tgt_args.len() {
        return false;
    }
    for (pat_arg, tgt_text) in pat_bracketed.args.iter().zip(tgt_args.iter()) {
        if !match_callee_generic_arg(pat_arg, tgt_text, reverse_map, bindings) {
            return false;
        }
    }
    true
}

/// Fast path: only the callee changed (`foo(E)` -> `bar(E)` with identical
/// arg metas in order). Preserves original formatting byte-for-byte except
/// the callee token.
fn try_callee_only_rewrite(
    minus: &ExprPatternKind,
    plus_raw: &str,
    cand: &Candidate<'_>,
    original: &str,
    reverse_map: &HashMap<String, String>,
) -> Option<String> {
    let (minus_callee, minus_args) = match minus {
        ExprPatternKind::Call { callee, args } => (callee.as_str(), args),
        _ => return None,
    };
    // Parse plus as `callee(args)` with matching arity.
    let plus = plus_raw.trim();
    let paren = plus.find('(')?;
    let plus_callee = plus[..paren].trim();
    let plus_inner = plus[paren + 1..].rsplit_once(')')?.0;
    let plus_args: Vec<String> = plus_inner
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if plus_args.len() != minus_args.len() || plus_args.len() != cand.arg_texts.len() {
        return None;
    }
    // Each plus arg must be exactly the corresponding bound metavar.
    for (pa, ma) in plus_args.iter().zip(minus_args.iter()) {
        match ma {
            ArgPat::Meta(ph) => {
                let name = reverse_map.get(ph).map(|s| s.as_str()).unwrap_or(ph);
                let bare = name.trim_start_matches('$');
                let pa_bare = pa.trim_start_matches('$');
                if pa_bare != bare {
                    return None;
                }
            }
            ArgPat::Ellipsis => {
                if pa != "..." {
                    return None;
                }
            }
            ArgPat::Concrete(c) => {
                if pa != c {
                    return None;
                }
            }
        }
    }
    // Reconstruct: original slice with callee range replaced.
    if minus_callee.contains("__RSMPL") || plus_callee.contains("__RSMPL") {
        return None;
    }
    // WI-7b: the raw plus callee may ride backticks (a concrete uppercase
    // constructor, SPEC 15.4); splice the de-quoted literal, never the
    // quotes — otherwise the rewritten file carries backticks and fails
    // the reparse. Unquotable text falls back to full substitution.
    let plus_callee = match rsmpl_pattern::strip_literal_quotes(plus_callee) {
        Ok((s, _)) => s,
        Err(_) => return None,
    };
    let pat_segs: Vec<&str> = minus_callee.split("::").collect();
    if pat_segs.contains(&"...") {
        return None; // path ellipsis: fall back to full substitution
    }
    let slice = original.get(cand.start..cand.end)?;
    // Callee is a path prefix of the slice; locate `(` to bound it.
    let paren_off = slice.find('(')?;
    let callee_slice = slice[..paren_off].trim().to_string();
    if callee_slice != *minus_callee
        && callee_slice.split("::").last() != minus_callee.split("::").last()
    {
        return None;
    }
    let mut out = String::new();
    out.push_str(&plus_callee);
    out.push_str(&slice[paren_off..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use rsmpl_patch::parse_patch;
    use std::collections::HashMap;

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        let patch = parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        // Patch targets must point at src/lib.rs; rewrite target paths.
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files["src/lib.rs"].clone())
    }

    const CALL_PATCH: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@ rule=rename_calls\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";

    #[test]
    fn renames_calls_many() {
        let src = "fn a() {\n    foo(1);\n    foo(2,);\n    other(3);\n}\n";
        let out = apply(CALL_PATCH, src).unwrap();
        assert!(out.contains("bar(1);"));
        assert!(out.contains("bar(2,);"));
        assert!(out.contains("other(3);"));
    }

    #[test]
    fn repeated_metavar_equality() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- assert_eq!(E, E)\n+ assert_equal_once(E)\n";
        // assert_eq is a macro; routed to macro matcher (tested there).
        let _ = patch;
    }

    #[test]
    fn method_to_function() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=m2f\n# lang=rust\n# allow=many\n# expr Receiver\n# expr Arg\n- Receiver.old_method(Arg)\n+ new_function(Receiver, Arg)\n";
        let src = "fn a() {\n    client.old_method(x);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("new_function(client, x);"));
    }

    #[test]
    fn method_receiver_nesting_outer_wins() {
        // T167: receiver nesting shares the candidate start offset, so
        // longest-first order accepts the outer match (mirrors T143).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=m\n# lang=rust\n# allow=many\n# expr R\n- R.to_string()\n+ R.to_owned()\n";
        let src = "fn a() {\n    let s = x.to_string().to_string();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("x.to_string().to_owned()"), "got: {out}");
    }

    #[test]
    fn method_receiver_nesting_repeat_folds() {
        // T167: `# repeat` folds receiver nesting bottom-up.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=m\n# lang=rust\n# repeat\n# allow=many\n# expr R\n- R.to_string()\n+ R.to_owned()\n";
        let src = "fn a() {\n    let s = x.to_string().to_string();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("x.to_owned().to_owned()"), "got: {out}");
    }

    #[test]
    fn no_match_without_optional_fails() {
        let src = "fn a() {}\n";
        assert!(apply(CALL_PATCH, src).is_err());
    }

    #[test]
    fn turbofish_placeholder_binds_type() {
        // T145: `id::<T>(E)` rewrites `id::<u8>(1u8)` binding `T` to the
        // target source text (the dogfood evidence case).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=x\n# lang=rust\n# allow=many\n# type T\n# expr E\n- id::<T>(E)\n+ id2::<T>(E)\n";
        let src = "fn id<T>(x: T) -> T { x }\nfn a() {\n    let a = id::<u8>(1u8);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("id2::<u8>(1u8);"), "got: {out}");
    }

    #[test]
    fn turbofish_concrete_matches_and_mismatch_fails() {
        // T145: concrete turbofish args compare token-normalized.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo::<u8>(E)\n+ bar::<u8>(E)\n";
        let src = "fn a() {\n    let a = foo::<u8>(1u8);\n    let b = foo::<u16>(1u16);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar::<u8>(1u8);"), "got: {out}");
        assert!(out.contains("foo::<u16>(1u16);"), "must not rewrite: {out}");
    }

    #[test]
    fn turbofish_const_arg_compares_rendered() {
        // T145 REVIEW: const generic args compare token-normalized, not
        // structurally — concrete `3` matches. WI-1c diet: undeclared `N`
        // in turbofish position now infers `# type N` and binds, so it
        // matches `3`; a backticked literal `` `N` `` keeps the old
        // fail-safe.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo::<3>(E)\n+ bar::<3>(E)\n";
        let src = "fn a() {\n    let a = foo::<3>(x);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar::<3>(x);"), "got: {out}");
        let miss = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo::<N>(E)\n+ bar::<N>(E)\n";
        let out = apply(miss, src).unwrap();
        assert!(out.contains("bar::<3>(x);"), "got: {out}");
        let lit = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo::<`N`>(E)\n+ bar::<`N`>(E)\n";
        assert!(apply(lit, src).is_err());
    }

    #[test]
    fn turbofish_bare_angle_brackets_fail_safe() {
        // T145: a bare `id<T>` pattern (no `::`) never matches a real
        // `id::<u8>` call — it never did; now documented fail-safe.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# type T\n# expr E\n- id<T>(E)\n+ id2<T>(E)\n";
        let src = "fn a() {\n    let a = id::<u8>(1u8);\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn turbofish_placeholder_segment_binds() {
        // T145: `P::<T>` binds the segment ident and the type arg.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n# expr P\n# type T\n# expr E\n- P::<T>(E)\n+ wrap2::<T>(E)\n";
        let src = "fn a() {\n    let a = wrap::<u8>(1u8);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("wrap2::<u8>(1u8);"), "got: {out}");
    }
    #[test]
    fn method_turbofish_placeholder_binds() {
        // T176: `R.collect::<T>()` matches through the call router (not
        // just the general matcher) and binds `T` structurally.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n# type T\n- R.collect::<T>()\n+ R.collect::<Vec<T>>()\n";
        let src = "fn a() {\n    let x = v.into_iter().collect::<Vec<i32>>();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("collect::<Vec<Vec<i32>>>()"), "got: {out}");
    }

    #[test]
    fn method_turbofish_placeholder_method_concrete_binds() {
        // T178: `R.M::<u8>()` binds the method ident like callee
        // `P::<T>` (T176 residual, works by construction — P10b probe).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n# expr M\n- R.M::<u8>()\n+ R.N::<u8>()\n";
        let src = "fn a() {\n    let x = v.collect::<u8>();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("v.N::<u8>()"), "got: {out}");
    }

    #[test]
    fn method_turbofish_placeholder_method_type_binds() {
        // T178: `R.M::<T>()` binds the method ident and `T`
        // structurally (P11 probe).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n# expr M\n# type T\n- R.M::<T>()\n+ R.N::<T>()\n";
        let src = "fn a() {\n    let x = v.collect::<u8>();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("v.N::<u8>()"), "got: {out}");
    }

    #[test]
    fn method_turbofish_placeholder_method_bare_rejects() {
        // T178: bare `R.M()` never matches a turbofish site (T177
        // parity on the placeholder-method arm — P9 probe).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n# expr M\n- R.M()\n+ R.N()\n";
        let src = "fn a() {\n    let x = v.collect::<Vec<i32>>();\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn method_turbofish_concrete_mismatch_fails() {
        // T176: concrete turbofish args compare structurally; a mismatch
        // fails safe instead of rewriting.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n- R.collect::<u8>()\n+ R.collect::<u16>()\n";
        let src = "fn a() {\n    let x = v.into_iter().collect::<Vec<i32>>();\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn method_bare_pattern_rejects_turbofish_site() {
        // T177: a bare `R.collect()` never matches a turbofish site
        // (None-vs-Some fails, general-matcher parity) — spell `::<T>`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n- R.collect()\n+ R.collected()\n";
        let src = "fn a() {\n    let x = v.into_iter().collect::<Vec<i32>>();\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn method_bare_pattern_still_matches_clean_site() {
        // T177 control: the same bare pattern still rewrites a
        // turbofish-free site byte-exact.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n- R.collect()\n+ R.collected()\n";
        let src = "fn a() {\n    let x = v.into_iter().collect();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("collected()"), "got: {out}");
    }

    #[test]
    fn call_bare_pattern_rejects_turbofish_site() {
        // T177: a bare `foo(E)` never matches `foo::<u8>(..)` —
        // spell `foo::<T>(E)` with `# type T` (T145).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src = "fn a() {\n    let a = foo::<u8>(1u8);\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn call_placeholder_callee_rejects_turbofish_site() {
        // T177: a bare whole-callee `P(E)` never matches a turbofish
        // site either — spell `P::<T>(E)` (T144/T145).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr P\n# expr E\n- P(E)\n+ bar(E)\n";
        let src = "fn a() {\n    let a = foo::<u8>(1u8);\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn symbol_callee_with_turbofish_binds() {
        // T199: `Old::<u8>(E)` with `# symbol Old=crate::old_api`
        // matches the turbofish site (sole-symbol expansion to the seeded
        // path; concrete-path parity is the P5b probe).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old::<u8>(E)\n+ New::<u8>(E)\n";
        let src = "fn a() {\n    let a = crate::old_api::<u8>(1u8);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("crate::new_api::<u8>(1u8);"), "got: {out}");
    }

    #[test]
    fn symbol_callee_with_placeholder_turbofish_binds() {
        // T199: `Old::<T>(E)` binds `T` structurally through the seeded
        // path expansion (P5e placeholder probe).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n# type T\n- Old::<T>(E)\n+ New::<T>(E)\n";
        let src = "fn a() {\n    let a = crate::old_api::<u8>(1u8);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("crate::new_api::<u8>(1u8);"), "got: {out}");
    }

    #[test]
    fn symbol_callee_with_turbofish_mismatch_fails() {
        // T199 negative: the seeded path must equal the site path, and a
        // wrong turbofish arg still fails safe.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old::<u16>(E)\n+ New::<u16>(E)\n";
        let src = "fn a() {\n    let a = crate::old_api::<u8>(1u8);\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn outermost_wins_single_pass() {
        // SPEC 17.6: `Box::new(Box::new(x))` rewrites the outer call only.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=unbox\n# lang=rust\n# allow=many\n# expr E\n- Box::new(E)\n+ E\n";
        let src = "fn a() {\n    let v = Box::new(Box::new(x));\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("Box::new(x)"), "got: {out}");
        assert!(!out.contains("let v = x;"), "must not recurse: {out}");
    }

    #[test]
    fn repeat_folds_nested_unbox() {
        // T20: `# repeat` folds the nested match into the outer edit.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=unbox\n# repeat\n# expr E\n- Box::new(E)\n+ E\n";
        let src = "fn a() {\n    let v = Box::new(Box::new(x));\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("let v = x;"), "got: {out}");
    }

    #[test]
    fn repeat_many_folds_each_site() {
        // T20: `allow=many` sites each fold their own nesting.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=unbox\n# repeat\n# allow=many\n# expr E\n- Box::new(E)\n+ E\n";
        let src = "fn a() {\n    let x = Box::new(Box::new(1));\n    let y = Box::new(2);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("let x = 1;"), "got: {out}");
        assert!(out.contains("let y = 2;"), "got: {out}");
    }

    #[test]
    fn repeat_same_rule_nests_in_args() {
        // T20: the same rule re-applies inside bound arguments.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=wrap\n# repeat\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(foo(x));\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(bar(x));"), "got: {out}");
    }

    #[test]
    fn repeat_self_reproducing_terminates() {
        // T20: the produced output is never re-scanned, so
        // `foo(E)` -> `foo(foo(E))` applies exactly one level.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=dup\n# repeat\n# expr E\n- foo(E)\n+ foo(foo(E))\n";
        let src = "fn a() {\n    foo(x);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("foo(foo(x));"), "got: {out}");
        assert!(!out.contains("foo(foo(foo("), "must terminate: {out}");
    }

    #[test]
    fn repeat_keeps_non_matching_binding_verbatim() {
        // T20: a binding that matches nothing (here `1 + 2`) is kept as-is.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=wrap\n# repeat\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1 + 2);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(1 + 2);"), "got: {out}");
    }

    #[test]
    fn repeat_counts_outermost_sites_only() {
        // T20: nested folds ride the outer edit; the report counts edits.
        let patch = parse_patch("diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=unbox\n# repeat\n# expr E\n- Box::new(E)\n+ E\n")
        .unwrap();
        let mut files = HashMap::new();
        files.insert(
            "src/lib.rs".to_string(),
            "fn a() {\n    let v = Box::new(Box::new(x));\n}\n".to_string(),
        );
        let report = crate::apply_patch_to_tree(&mut files, &patch).unwrap();
        assert_eq!(report.rules.len(), 1);
        assert_eq!(
            report.rules[0].matches, 1,
            "nested folds are not separate matches"
        );
        assert!(files["src/lib.rs"].contains("let v = x;"));
    }

    #[test]
    fn scope_exact_requires_full_span() {
        let mk = |scope: &str| {
            format!("diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# scope={scope}\n# expr E\n- foo(E)\n+ bar(E)\n")
        };
        let src = "fn a() {\n    foo(1);\n}\n";
        assert!(apply(&mk("contained"), src).is_ok());
        assert!(apply(&mk("anchor"), src).is_ok());
        assert!(apply(&mk("overlap"), src).is_ok());
        assert!(apply(&mk("exact"), src).is_ok());
        // Exact with a wider range must fail.
        let wide = mk("exact").replace("@@ -2,1 +2,1 @@", "@@ -1,3 +1,3 @@");
        assert!(apply(&wide, src).is_err());
    }

    #[test]
    fn ellipsis_replays_absorbed_remainder_not_whole_list() {
        // T157: `foo(E, ...)` -> `bar(E, ...)` replays only the absorbed
        // remainder (`2, 3`), not the whole arg list (was `bar(1, 1, 2, 3)`).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E, ...)\n+ bar(E, ...)\n";
        let src = "fn a() {\n    foo(1, 2, 3);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(1, 2, 3);"), "got: {out}");
    }

    #[test]
    fn ellipsis_empty_remainder_swallows_comma() {
        // T157: `bar(E, ...)` on `foo(1)` yields `bar(1)`, not `bar(1, )`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E, ...)\n+ bar(E, ...)\n";
        let src = "fn a() {\n    foo(1);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(1);"), "got: {out}");
        assert!(!out.contains("bar(1, )"), "comma artifact: {out}");
        // Leading mirror: `bar(..., E)` on `foo(1)` yields `bar(1)`.
        let lead = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(..., E)\n+ bar(..., E)\n";
        let out = apply(lead, src).unwrap();
        assert!(out.contains("bar(1);"), "got: {out}");
    }

    #[test]
    fn ellipsis_arg_drop_omits_plus_ellipsis() {
        // T157: dropping `...` on the plus side is a deliberate arg-drop.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E, ...)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1, 2);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(1);"), "got: {out}");
    }

    #[test]
    fn ellipsis_method_call_replays_remainder() {
        // T157: the fix rides the shared `match_args` path, so method
        // calls replay too.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n# expr E\n- R.foo(E, ...)\n+ R.bar(E, ...)\n";
        let src = "fn a() {\n    r.foo(1, 2);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("r.bar(1, 2);"), "got: {out}");
    }

    #[test]
    fn ellipsis_repeat_folds_with_replay() {
        // T157: `# repeat` + `...` folds instead of duplicating
        // (was `bar(bar(1, 1), foo(1))`).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# repeat\n# expr E\n- foo(E, ...)\n+ bar(E, ...)\n";
        let src = "fn a() {\n    foo(foo(1));\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(bar(1));"), "got: {out}");
    }

    #[test]
    fn ellipsis_pure_shape_regresses_clean() {
        // T157: lone `...` still replays the whole body (== remainder).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n- foo(...)\n+ bar(...)\n";
        let src = "fn a() {\n    foo(1, 2);\n    foo();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(1, 2);"), "got: {out}");
        assert!(out.contains("bar();"), "got: {out}");
    }

    #[test]
    fn ellipsis_plus_without_minus_is_pattern_conflict() {
        // T157: `+ ...` with no `- ...` has nothing to replay — fail fast
        // instead of duplicating the arg list (was `bar(1, 2, 1, 2)`).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n# expr F\n- foo(E, F)\n+ bar(E, F, ...)\n";
        let src = "fn a() {\n    foo(1, 2);\n}\n";
        let err = apply(patch, src).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("no matching `- ...`"), "got: {err}");
    }

    #[test]
    fn ellipsis_leading_backtracks_to_last_arg() {
        // T159: `- foo(..., E)` on `foo(1, 2, 3)` binds the last arg
        // (first-fit alone would bind `1` and fail the tail check).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(..., E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1, 2, 3);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(3);"), "got: {out}");
    }

    #[test]
    fn ellipsis_middle_binds_both_ends() {
        // T159: `- foo(A, ..., B)` on `foo(1, 2, 3)` binds `1` and `3`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr A\n# expr B\n- foo(A, ..., B)\n+ bar(A, B)\n";
        let src = "fn a() {\n    foo(1, 2, 3);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(1, 3);"), "got: {out}");
    }

    #[test]
    fn ellipsis_middle_replays_remainder() {
        // T159: the absorbed middle replays positionally (T157 replay
        // rides the backtracking alignment).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr A\n# expr B\n- foo(A, ..., B)\n+ bar(A, ..., B)\n";
        let src = "fn a() {\n    foo(1, 2, 3);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(1, 2, 3);"), "got: {out}");
    }

    #[test]
    fn ellipsis_backtrack_keeps_repeated_metavar_equal() {
        // T159: `E` must stay structurally equal across the backtrack —
        // `- foo(E, ..., E)` on `foo(1, 2, 1)` binds `E` to `1`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E, ..., E)\n+ bar(E, ...)\n";
        let src = "fn a() {\n    foo(1, 2, 1);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("bar(1, 2);"), "got: {out}");
        // Unequal ends fail safe rather than binding inconsistently.
        let src = "fn a() {\n    foo(1, 2, 3);\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn ellipsis_leading_method_call_backtracks() {
        // T159: the method-call arm shares `match_args`, so leading
        // `...` backtracks there too.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n# expr E\n- R.foo(..., E)\n+ R.bar(E)\n";
        let src = "fn a() {\n    r.foo(1, 2, 3);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("r.bar(3);"), "got: {out}");
    }

    #[test]
    fn path_ellipsis_matches_zero_or_more_segments() {
        // T164 (SPEC 16.5): a minus-side callee `...` matches zero or
        // more path segments; the plus side stays concrete.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::new(E)\n";
        let src = "fn a() {\n    crate::old(1);\n    crate::a::old(2);\n    crate::a::b::old(3);\n    crate::a::other(4);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("crate::new(1);"), "got: {out}");
        assert!(out.contains("crate::new(2);"), "got: {out}");
        assert!(out.contains("crate::new(3);"), "got: {out}");
        assert!(
            out.contains("crate::a::other(4);"),
            "must not rewrite: {out}"
        );
    }

    #[test]
    fn path_ellipsis_last_segment_mismatch_fails() {
        // T164: the segments around `...` still match exactly.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- crate::...::old(E)\n+ crate::new(E)\n";
        let src = "fn a() {\n    crate::a::newer(1);\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn inner_array_ellipsis_rejected_not_literal() {
        // T442: `- run(D, &["--patch", M, ...])` is not an arg-level
        // `...` shape — the inner `...` must fail fast as PatternConflict
        // (via the Raw bypass + general-pattern `...` rejection), never
        // match with a literal unbound `M` in the output.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr D\n# expr M\n- run(D, &[\"--patch\", M, ...])\n+ run_stdin(D, M, &[...])\n";
        let src = "fn m() {\n    run(dir, &[\"--patch\", rename_patch()]);\n}\n";
        let err = apply(patch, src).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("..."), "got: {err}");
    }
}
