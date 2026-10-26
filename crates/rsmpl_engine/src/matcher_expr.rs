//! Call / method-call expression matching and rewriting (SPEC 13, 17-18).
//!
//! Candidates are collected in preorder (outer before inner) so that
//! `# allow=many` implements outermost-wins single-pass semantics (SPEC
//! 17.6). Rewrites preserve formatting by reusing original source slices for
//! captured metavariables.

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
    receiver_text: Option<String>,
    method_text: Option<String>,
    is_method: bool,
    ellipsis_body: String,
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
                        let ellipsis_body = inner_text(&self.slice(start, end));
                        self.out.push(Candidate {
                            start,
                            end,
                            start_line: node.span().start().line as u32,
                            end_line: node.span().end().line as u32,
                            callee_segments: segs,
                            arg_texts: args,
                            receiver_text: None,
                            method_text: None,
                            is_method: false,
                            ellipsis_body,
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
                    let ellipsis_body = inner_text(&self.slice(start, end));
                    self.out.push(Candidate {
                        start,
                        end,
                        start_line: node.span().start().line as u32,
                        end_line: node.span().end().line as u32,
                        callee_segments: vec![m.method.to_string()],
                        arg_texts: args,
                        receiver_text: Some(self.slice_opt(m.receiver.span())),
                        method_text: Some(m.method.to_string()),
                        is_method: true,
                        ellipsis_body,
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

/// Text between the outermost parens of a call slice (for `...` reuse).
fn inner_text(call_slice: &str) -> String {
    if let (Some(l), Some(r)) = (call_slice.find('('), call_slice.rfind(')')) {
        if r > l {
            return call_slice[l + 1..r].to_string();
        }
    }
    String::new()
}

fn in_scope(start_line: u32, end_line: u32, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    let hi = end.max(old_start);
    match scope {
        Scope::Contained => start_line >= old_start && end_line <= hi,
        Scope::Anchor => start_line >= old_start && start_line <= hi,
        Scope::Overlap => start_line <= hi && end_line >= old_start,
        Scope::Exact => old_len > 0 && start_line == old_start && end_line == end,
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
/// Returns `false` when this candidate does not match.
fn match_args(
    pattern: &[ArgPat],
    targets: &[String],
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
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
    // With ellipses: align non-ellipsis pattern args in order (greedy).
    let mut ti = 0;
    let mut pi = 0;
    while pi < pattern.len() {
        if pattern[pi] == ArgPat::Ellipsis {
            // If last pattern element, absorb the rest.
            if pi + 1 == pattern.len() {
                return true;
            }
            // Otherwise skip target args until the next non-ellipsis pattern
            // arg matches (greedy first-fit).
            let next = &pattern[pi + 1];
            let mut found = None;
            for (k, target) in targets.iter().enumerate().skip(ti) {
                let mut probe = bindings.clone();
                if match_single_arg(next, target, reverse_map, &mut probe) {
                    found = Some((k, probe));
                    break;
                }
            }
            match found {
                Some((k, probe)) => {
                    *bindings = probe;
                    ti = k + 1;
                    pi += 2;
                }
                None => return false,
            }
        } else {
            if ti >= targets.len() {
                return false;
            }
            if !match_single_arg(&pattern[pi], &targets[ti], reverse_map, bindings) {
                return false;
            }
            ti += 1;
            pi += 1;
        }
    }
    // Trailing targets must have been absorbed by a trailing ellipsis.
    ti == targets.len()
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
pub fn substitute_plus(
    plus_raw: &str,
    bindings: &HashMap<String, String>,
    ellipsis_text: &str,
    sigiled: bool,
) -> String {
    // Replace standalone `...` first (not part of longer dot runs).
    let mut out = replace_ellipsis_token(plus_raw, ellipsis_text);
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

fn replace_ellipsis_token(text: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 2 < bytes.len() && &text[i..i + 3] == "..." {
            let before_ok = i == 0 || bytes[i - 1] != b'.';
            let after_ok = i + 3 >= bytes.len() || bytes[i + 3] != b'.';
            if before_ok && after_ok {
                out.push_str(replacement);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
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
) -> Result<usize, StructuredError> {
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
            .with_suggestion("add `# fallback=syntax` to allow syntactic matching"));
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
    // but sort defensively by byte offset.
    cands.sort_by_key(|c| (c.start, c.end));

    let reverse_map: HashMap<String, String> = build_reverse_map(compiled);

    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut matched = 0;
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
        let mut bindings: HashMap<String, String> = HashMap::new();
        // Seed symbol bindings (symbols are pre-bound to their path).
        for (n, p) in &compiled.table.symbols {
            bindings.insert(n.clone(), p.clone());
        }
        let ok = match minus {
            ExprPatternKind::Call { callee, args } => {
                !cand.is_method
                    && match_callee(callee, &cand.callee_segments, compiled, &mut bindings)
                    && match_args(args, &cand.arg_texts, &reverse_map, &mut bindings)
            }
            ExprPatternKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                cand.is_method
                    && match_receiver_or_method(receiver, method, cand, &reverse_map, &mut bindings)
                    && match_args(args, &cand.arg_texts, &reverse_map, &mut bindings)
            }
            ExprPatternKind::Macro { .. } => false, // routed to macro matcher
            ExprPatternKind::Raw { .. } => false,
        };
        if !ok {
            continue;
        }
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
        // Build replacement.
        let replacement = if let Some(fast) =
            try_callee_only_rewrite(minus, plus_raw, cand, original, &reverse_map)
        {
            fast
        } else {
            substitute_plus(
                plus_raw,
                &bindings,
                &cand.ellipsis_body,
                compiled.table.sigiled,
            )
        };
        edits.push(TextEdit::new(cand.start, cand.end, replacement));
        accepted.push((cand.start, cand.end));
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break;
        }
    }
    Ok(matched)
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
/// the target's last segment to equal the symbol path's last segment
/// (syntactic fallback approximation, SPEC 19.3).
fn match_callee(
    callee: &str,
    target_segments: &[String],
    compiled: &CompiledPattern,
    bindings: &mut HashMap<String, String>,
) -> bool {
    let reverse_map = build_reverse_map(compiled);
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
    // Method: meta allowed (bind), else exact.
    if let Some(orig) = reverse_map.get(method_pat) {
        let text = cand.method_text.clone().unwrap_or_default();
        match bindings.get(orig) {
            Some(prev) => prev == &text,
            None => {
                bindings.insert(orig.clone(), text);
                true
            }
        }
    } else {
        method_pat == cand.method_text.clone().unwrap_or_default()
    }
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
    out.push_str(plus_callee);
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
    fn no_match_without_optional_fails() {
        let src = "fn a() {}\n";
        assert!(apply(CALL_PATCH, src).is_err());
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
}
