//! Macro invocation and token-tree matching (SPEC 21).
//!
//! v0.1 supports:
//! - invocation mode: `old_name!(args)` -> `new_name!(args)` with metavariable
//!   argument matching (including repeated-metavar equality);
//! - token-tree mode: only the `NAME!(...)` shape (body preserved verbatim);
//! - `# macro=expanded` is rejected (requires build integration, SPEC 21.5).

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{ArgPat, CompiledPattern, ExprPatternKind, PatternKind};

use crate::edits::{fragments_equal, TextEdit};
use crate::lines::LineIndex;
use crate::matcher_expr::substitute_plus;

struct MacroCandidate {
    start: usize,
    end: usize,
    start_line: u32,
    end_line: u32,
    path_text: String,
    /// Original body slice (verbatim, between the macro delimiters).
    body_text: String,
    body_args: Vec<String>,
}

struct MacroCollector<'b> {
    original: &'b str,
    index: &'b LineIndex,
    out: Vec<MacroCandidate>,
}

impl<'b> MacroCollector<'b> {
    fn push_macro(&mut self, mac: &syn::Macro) {
        let span = mac.span();
        let (Some(start), Some(end)) = (
            self.index
                .offset(span.start().line as u32, span.start().column),
            self.index.offset(span.end().line as u32, span.end().column),
        ) else {
            return;
        };
        let path_text = mac
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        let slice = self.original.get(start..end).unwrap_or("");
        let body_text = raw_macro_body(slice).unwrap_or_else(|| mac.tokens.to_string());
        let body_args = split_macro_args(&body_text);
        self.out.push(MacroCandidate {
            start,
            end,
            start_line: span.start().line as u32,
            end_line: span.end().line as u32,
            path_text,
            body_text,
            body_args,
        });
    }
}

/// Extract the verbatim body between a macro invocation's delimiters from the
/// original source slice (`name!(...)`, `name[...]`, `name{...}`).
fn raw_macro_body(slice: &str) -> Option<String> {
    let bang = slice.find('!')?;
    let rest = slice[bang + 1..].trim_start();
    let mut chars = rest.chars();
    let open = chars.next()?;
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => return None,
    };
    if !rest.trim_end().ends_with(close) {
        return None;
    }
    let inner = &rest[1..rest.trim_end().len() - 1];
    Some(inner.to_string())
}

impl<'ast, 'b> Visit<'ast> for MacroCollector<'b> {
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.push_macro(node);
        visit::visit_macro(self, node);
    }
}

/// Split macro body text into top-level comma-separated token groups.
fn split_macro_args(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in body.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur.trim().to_string());
    }
    out.retain(|s| !s.is_empty());
    out
}

fn in_scope(c: &MacroCandidate, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    let hi = end.max(old_start);
    match scope {
        Scope::Contained => c.start_line >= old_start && c.end_line <= hi,
        Scope::Anchor => c.start_line >= old_start && c.start_line <= hi,
        Scope::Overlap => c.start_line <= hi && c.end_line >= old_start,
        Scope::Exact => old_len > 0 && c.start_line == old_start && c.end_line == end,
    }
}

/// Parse `name!(args)` into (name, args-text). Returns None if not macro-shaped.
fn split_invocation(text: &str) -> Option<(String, String)> {
    let bang = text.find('!')?;
    let name = text[..bang].trim().to_string();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let rest = text[bang + 1..].trim();
    let mut chars = rest.chars();
    let open = chars.next()?;
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => return None,
    };
    if !rest.ends_with(close) {
        return None;
    }
    Some((name, rest[1..rest.len() - 1].to_string()))
}

fn reverse_map_of(compiled: &CompiledPattern) -> HashMap<String, String> {
    // Maps both placeholder spellings (`__RSMPL_M`) and declared spellings
    // (`M`) to declared names, since macro patterns are matched against raw
    // hunk text (never substituted).
    let mut m = HashMap::new();
    if let PatternKind::Expr { placeholders, .. } = &compiled.kind {
        for (name, ph) in placeholders {
            m.insert(ph.clone(), name.clone());
        }
    }
    for name in compiled.table.kinds.keys() {
        m.insert(format!("__RSMPL_{name}"), name.clone());
        m.insert(name.clone(), name.clone());
    }
    m
}

/// Apply a macro hunk. Returns match count.
pub fn apply_macro_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let rule = hunk.rule_name();
    if hunk.macro_mode() == rsmpl_patch::MacroMode::Expanded {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# macro=expanded` requires build integration (macro expansion span mapping), unavailable in v0.1",
        )
        .with_rule(&rule));
    }
    let (minus_raw, plus_raw) = match &compiled.kind {
        PatternKind::Expr {
            minus: ExprPatternKind::Macro { .. },
            plus_raw,
            ..
        } => {
            let full_minus: String = hunk
                .lines
                .iter()
                .filter(|l| l.is_removal() || l.is_context())
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            (full_minus.trim().to_string(), plus_raw.clone())
        }
        PatternKind::Macro {
            minus_raw,
            plus_raw,
        } => (minus_raw.clone(), plus_raw.clone()),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected macro pattern".to_string(),
            ))
        }
    };

    let token_tree = hunk.macro_mode() == rsmpl_patch::MacroMode::TokenTree;
    let (minus_name_pat, minus_args_pat) = split_invocation(&minus_raw).ok_or_else(|| {
        StructuredError::new(
            ErrorCode::PatternConflict,
            format!("v0.1 macro patterns must have `NAME!(...)` shape, got `{minus_raw}`"),
        )
        .with_rule(&rule)
    })?;
    // The plus side may be macro-shaped (`new_name!(...)`) or a plain
    // expression (macro -> function migration like `assert_eq!(E, E)` to
    // `assert_equal_once(E)`).
    let plus_invocation = split_invocation(plus_raw.trim());
    let (plus_name_pat, plus_args_pat) = match &plus_invocation {
        Some((n, a)) => (n.clone(), a.clone()),
        None => (String::new(), String::new()),
    };

    // Token-tree mode: body must be a bare `...` (verbatim preservation).
    if token_tree && minus_args_pat.trim() != "..." {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "v0.1 `# macro=token_tree` supports only `NAME!(...)` with verbatim body",
        )
        .with_rule(&rule));
    }

    let reverse_map = reverse_map_of(compiled);
    // Classify minus args with the shared arg classifier. The pattern text at
    // this point still holds `__RSMPL_` placeholders (from the pattern
    // crate substitution); map them back to declared names.
    let minus_args = rsmpl_pattern_args(&minus_args_pat, &reverse_map);

    let mut collector = MacroCollector {
        original,
        index,
        out: Vec::new(),
    };
    collector.visit_file(parsed);
    let mut cands = collector.out;
    cands.sort_by_key(|c| (c.start, c.end));

    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut matched = 0;
    for cand in &cands {
        if accepted
            .iter()
            .any(|&(s, e)| s <= cand.start && cand.end <= e)
        {
            continue;
        }
        if !in_scope(
            cand,
            hunk.scope(),
            hunk.header.old_start,
            hunk.header.old_len,
        ) {
            continue;
        }
        let mut bindings: HashMap<String, String> = HashMap::new();
        if !match_macro_name(
            &minus_name_pat,
            &cand.path_text,
            &reverse_map,
            &compiled.table.symbols,
            &mut bindings,
        ) {
            continue;
        }
        if token_tree {
            // Verbatim body: nothing to match inside.
        } else if is_lone_body_meta(&minus_args) {
            // `M!(TT)`: the single metavariable captures the whole body.
            let name = lone_meta_name(&minus_args, &reverse_map).unwrap();
            match bindings.get(&name) {
                Some(prev) if prev != &cand.body_text => continue,
                Some(_) => {}
                None => {
                    bindings.insert(name, cand.body_text.clone());
                }
            }
        } else if !match_macro_args(&minus_args, &cand.body_args, &reverse_map, &mut bindings) {
            continue;
        }
        // Build replacement.
        let replacement = if token_tree
            || (plus_args_pat.trim() == minus_args_pat.trim()
                && plus_name_pat == minus_name_pat
                && reverse_map.is_empty())
        {
            // Pure path rename with identical body: splice path only.
            let slice = original.get(cand.start..cand.end).unwrap_or("");
            let bang_off = slice.find('!').unwrap_or(0);
            format!(
                "{}{}",
                resolve_name(&plus_name_pat, &bindings),
                &slice[bang_off..]
            )
        } else if plus_args_pat.trim() == "..." || minus_args_pat.trim() == "..." {
            let slice = original.get(cand.start..cand.end).unwrap_or("");
            let bang_off = slice.find('!').unwrap_or(0);
            format!(
                "{}{}",
                resolve_name(&plus_name_pat, &bindings),
                &slice[bang_off..]
            )
        } else if plus_invocation.is_some() {
            let new_args = substitute_plus(
                &plus_args_pat,
                &bindings,
                &cand.body_text,
                compiled.table.sigiled,
            );
            format!("{}!({})", resolve_name(&plus_name_pat, &bindings), new_args)
        } else {
            // Plus side is not macro-shaped (e.g. macro -> function call):
            // substitute into the whole plus text.
            substitute_plus(
                plus_raw.trim(),
                &bindings,
                &cand.body_text,
                compiled.table.sigiled,
            )
        };
        // Partial-overlap guard.
        for &(s, e) in &accepted {
            if cand.start < e && s < cand.end && !(s <= cand.start && cand.end <= e) {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "partially overlapping matches cannot be merged",
                )
                .with_rule(&rule));
            }
        }
        edits.push(TextEdit::new(cand.start, cand.end, replacement));
        accepted.push((cand.start, cand.end));
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break;
        }
    }
    Ok(matched)
}

/// A lone body metavariable (`M!(TT)`) captures the entire macro body.
fn is_lone_body_meta(args: &[ArgPat]) -> bool {
    matches!(args, [ArgPat::Meta(_)])
}

fn lone_meta_name(args: &[ArgPat], reverse_map: &HashMap<String, String>) -> Option<String> {
    match args {
        [ArgPat::Meta(ph)] => Some(
            reverse_map
                .get(ph)
                .cloned()
                .unwrap_or_else(|| ph.clone())
                .trim_start_matches('$')
                .to_string(),
        ),
        _ => None,
    }
}

fn match_macro_name(
    pattern: &str,
    target: &str,
    reverse_map: &HashMap<String, String>,
    symbols: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
) -> bool {
    // The minus pattern here is raw hunk text, so a declared macro-name
    // metavariable appears under its declared spelling (`M`), not the
    // `__RSMPL_` placeholder used in substituted classifier output.
    let declared = reverse_map.get(pattern).cloned().or_else(|| {
        let bare = pattern.trim_start_matches('$');
        reverse_map
            .values()
            .any(|n| n == bare)
            .then(|| bare.to_string())
    });
    if let Some(orig) = declared {
        if let Some(sym_path) = symbols.get(&orig) {
            let want_last = sym_path.rsplit("::").next().unwrap_or(sym_path);
            let got_last = target.rsplit("::").next().unwrap_or(target);
            if got_last != want_last {
                return false;
            }
        }
        let name = orig.trim_start_matches('$').to_string();
        match bindings.get(&name) {
            Some(prev) => prev == target,
            None => {
                bindings.insert(name, target.to_string());
                true
            }
        }
    } else if let Some(sym_path) = symbols.get(pattern) {
        // Raw (unsubstituted) symbol name in the pattern.
        let want_last = sym_path.rsplit("::").next().unwrap_or(sym_path);
        let got_last = target.rsplit("::").next().unwrap_or(target);
        got_last == want_last
    } else {
        pattern == target
    }
}

fn match_macro_args(
    pattern: &[ArgPat],
    targets: &[String],
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
) -> bool {
    // Reuse expression arg matching semantics (ellipsis, equality).
    if !pattern.contains(&ArgPat::Ellipsis) {
        if pattern.len() != targets.len() {
            return false;
        }
        for (p, t) in pattern.iter().zip(targets.iter()) {
            if !match_one(p, t, reverse_map, bindings) {
                return false;
            }
        }
        return true;
    }
    let mut ti = 0;
    let mut pi = 0;
    while pi < pattern.len() {
        if pattern[pi] == ArgPat::Ellipsis {
            if pi + 1 == pattern.len() {
                return true;
            }
            let next = &pattern[pi + 1];
            let mut found = None;
            for (k, target) in targets.iter().enumerate().skip(ti) {
                let mut probe = bindings.clone();
                if match_one(next, target, reverse_map, &mut probe) {
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
            if !match_one(&pattern[pi], &targets[ti], reverse_map, bindings) {
                return false;
            }
            ti += 1;
            pi += 1;
        }
    }
    ti == targets.len()
}

fn match_one(
    pattern: &ArgPat,
    target: &str,
    reverse_map: &HashMap<String, String>,
    bindings: &mut HashMap<String, String>,
) -> bool {
    match pattern {
        ArgPat::Ellipsis => true,
        ArgPat::Meta(ph) => {
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

fn resolve_name(pattern: &str, bindings: &HashMap<String, String>) -> String {
    let bare = pattern.trim_start_matches('$');
    if let Some(v) = bindings.get(bare) {
        return v.clone();
    }
    if let Some(v) = bindings.get(pattern) {
        return v.clone();
    }
    pattern.trim_start_matches('$').to_string()
}

/// Classify a comma-separated macro arg pattern, mapping `__RSMPL_`
/// placeholders back to declared metavariable names.
fn rsmpl_pattern_args(pattern_args: &str, reverse_map: &HashMap<String, String>) -> Vec<ArgPat> {
    let mut out = Vec::new();
    for part in split_top_level(pattern_args) {
        let t = part.trim().to_string();
        if t.is_empty() {
            continue;
        }
        if t == "..." || t.starts_with("__RSMPL_ELLIPSIS") {
            out.push(ArgPat::Ellipsis);
            continue;
        }
        if let Some(name) = reverse_map.get(&t) {
            out.push(ArgPat::Meta(name.clone()));
            continue;
        }
        if let Some(rest) = t.strip_prefix("__RSMPL_") {
            // Undeclared placeholder (shouldn't happen): keep concrete.
            let _ = rest;
            out.push(ArgPat::Concrete(t));
            continue;
        }
        let bare = t.trim_start_matches('$');
        if reverse_map.values().any(|n| n == bare) {
            out.push(ArgPat::Meta(bare.to_string()));
        } else {
            out.push(ArgPat::Concrete(t));
        }
    }
    out
}

fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur);
    }
    out
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
    fn renames_macro_invocation() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# allow=many\n# macro M\n# token_tree TT\n- M!(TT)\n+ new_macro!(TT)\n";
        let src = "fn a() {\n    old!(1, 2);\n}\n";
        // M/TT declared as metavars: M binds `old`, TT binds body.
        let out = apply(patch, src).unwrap();
        assert!(out.contains("new_macro!(1, 2);"), "got: {out}");
    }

    #[test]
    fn repeated_macro_args_checked() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- assert_eq!(E, E)\n+ assert_equal_once(E)\n";
        let same = "fn a() {\n    assert_eq!(foo(), foo());\n}\n";
        let out = apply(patch, same).unwrap();
        assert!(out.contains("assert_equal_once(foo())"), "got: {out}");
        let diff = "fn a() {\n    assert_eq!(foo(), bar());\n}\n";
        assert!(apply(patch, diff).is_err());
    }

    #[test]
    fn expanded_mode_rejected() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# macro=expanded\n- m!(...)\n+ n!(...)\n";
        assert!(apply(patch, "fn a() { m!(x); }\n").is_err());
    }
}
