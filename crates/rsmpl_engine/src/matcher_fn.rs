//! Function-item matching and rewriting (SPEC 14).
//!
//! v0.1 supports: rename of plain/explicit functions (minus/plus differ only
//! in the function name), item deletion via bare `- I`, literal insertion,
//! and rename inside `impl` context blocks. Format is preserved by replacing
//! only the name token (or deleting/inserting whole line spans).

use syn::spanned::Spanned;

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, FnPattern, PatternKind, RetPat, VisPat};

use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// A function item with its source span lines and attribute lines.
struct FnCandidate<'a> {
    item: &'a syn::ItemFn,
    start_line: u32,
    end_line: u32,
    fn_line: u32,
    attr_start_line: u32,
}

fn collect_fns<'a>(file: &'a syn::File) -> Vec<FnCandidate<'a>> {
    let mut out = Vec::new();
    collect_from_items(&file.items, &mut out);
    out
}

fn collect_from_items<'a>(items: &'a [syn::Item], out: &mut Vec<FnCandidate<'a>>) {
    for it in items {
        match it {
            syn::Item::Fn(f) => out.push(candidate_of(f)),
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_from_items(inner, out);
                }
            }
            _ => {}
        }
    }
}

fn candidate_of(f: &syn::ItemFn) -> FnCandidate<'_> {
    let s = f.span().start();
    let e = f.span().end();
    let fl = f.sig.fn_token.span().start().line as u32;
    // Attribute envelope starts at the item span start (syn includes attrs).
    FnCandidate {
        item: f,
        start_line: s.line as u32,
        end_line: e.line as u32,
        fn_line: fl,
        attr_start_line: s.line as u32,
    }
}

/// Scope filtering (SPEC 17.3). `anchor` uses the `fn` keyword line as the
/// first concrete non-ellipsis token.
fn in_scope(c: &FnCandidate<'_>, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.start_line >= old_start && c.end_line <= end.max(old_start),
        Scope::Anchor => c.fn_line >= old_start && c.fn_line <= end.max(old_start),
        Scope::Overlap => c.start_line <= end.max(old_start) && c.end_line >= old_start,
        Scope::Exact => old_len > 0 && c.start_line == old_start && c.end_line == end,
    }
}

fn check_attrs(
    c: &FnCandidate<'_>,
    policy: AttrsPolicy,
    pattern_attrs: &[String],
    rule: &str,
    file: &str,
) -> Result<(), StructuredError> {
    let has = !c.item.attrs.is_empty();
    match policy {
        AttrsPolicy::Preserve => Ok(()),
        AttrsPolicy::Absent => {
            if has {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "function has attributes but `# attrs=absent` requires none",
                )
                .with_rule(rule)
                .with_file(file));
            }
            Ok(())
        }
        AttrsPolicy::Explicit => {
            let want: Vec<String> = pattern_attrs
                .iter()
                .map(|a| a.split_whitespace().collect::<String>())
                .collect();
            let have: Vec<String> = c
                .item
                .attrs
                .iter()
                .map(|a| quote::quote!(#a).to_string().split_whitespace().collect())
                .collect();
            if want != have {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "attached attributes do not match explicitly written pattern attributes",
                )
                .with_rule(rule)
                .with_file(file)
                .with_suggestion("use `# attrs=preserve` (default) to ignore attributes"));
            }
            Ok(())
        }
        AttrsPolicy::Strip => Ok(()), // eligibility like preserve; stripped at rewrite
    }
}

/// Reconstruct the `FnPattern` for the minus side (also validates it parses).
///
/// For `ImplContext` hunks the full minus text includes the surrounding
/// `impl` header lines, so only the function block (from the first fn-like
/// line, balanced to its closing brace) is parsed.
fn minus_fn_pattern(
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
) -> Result<FnPattern, StructuredError> {
    let (vis, qual, gen, wh) = slot_metas(compiled);
    // Rebuild full minus text (removals + context) as the pattern crate does.
    let full: String = hunk
        .lines
        .iter()
        .filter(|l| l.is_removal() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let block = match &compiled.kind {
        PatternKind::ImplContext { .. } => extract_fn_block(&full),
        _ => full,
    };
    rsmpl_pattern::parse_fn_pattern(&block, &vis, &qual, &gen, &wh, hunk.returns_none())
}

/// Extract the first function block from text that may be prefixed by
/// context lines (e.g. `impl Service {`). Scans for the first fn-like line
/// and takes lines until braces balance back to zero.
fn extract_fn_block(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut start = None;
    for (idx, line) in lines.iter().enumerate() {
        let t = line.trim();
        let t = t.strip_prefix("#[").unwrap_or(t);
        if t.starts_with("fn ")
            || t.starts_with("pub ")
            || t.starts_with("async ")
            || t.starts_with("unsafe ")
            || t.starts_with("const ")
            || t.starts_with("extern ")
            || t.starts_with("pub(")
            || t.starts_with("fn(")
        {
            start = Some(idx);
            break;
        }
    }
    let Some(s) = start else {
        return text.to_string();
    };
    let mut depth = 0i32;
    let mut seen_open = false;
    let mut end = lines.len();
    for (idx, line) in lines.iter().enumerate().skip(s) {
        for c in line.chars() {
            if c == '{' {
                depth += 1;
                seen_open = true;
            } else if c == '}' {
                depth -= 1;
            }
        }
        if seen_open && depth == 0 {
            end = idx + 1;
            break;
        }
    }
    lines[s..end].join("\n")
}

fn slot_metas(compiled: &CompiledPattern) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    use rsmpl_patch::MetavarKind;
    let mut vis = Vec::new();
    let mut qual = Vec::new();
    let mut gen = Vec::new();
    let mut wh = Vec::new();
    for (kind, name) in &compiled.table.declared {
        match kind {
            MetavarKind::Visibility => vis.push(name.clone()),
            MetavarKind::Qualifier | MetavarKind::QualifierList => qual.push(name.clone()),
            MetavarKind::GenericParams => gen.push(name.clone()),
            MetavarKind::WhereClause => wh.push(name.clone()),
            _ => {}
        }
    }
    (vis, qual, gen, wh)
}

/// Apply a function-rename hunk. Returns the number of matches rewritten.
pub fn apply_fn_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (minus_name, plus_name) = match &compiled.kind {
        PatternKind::Fn {
            minus_name,
            plus_name,
            ..
        } => (minus_name.clone(), plus_name.clone()),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected function pattern".to_string(),
            ))
        }
    };
    let rule = hunk.rule_name();
    let file = "";
    let minus_pat = minus_fn_pattern(hunk, compiled).map_err(|e| e.with_rule(&rule))?;

    // v0.1: only the name may differ between minus and plus.
    {
        let full_plus: String = hunk
            .lines
            .iter()
            .filter(|l| l.is_addition() || l.is_context())
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let (vis, qual, gen, wh) = slot_metas(compiled);
        let plus_pat = rsmpl_pattern::parse_fn_pattern(&full_plus, &vis, &qual, &gen, &wh, false)
            .map_err(|e| e.with_rule(&rule))?;
        if minus_pat.visibility != plus_pat.visibility
            && !matches!(
                (&minus_pat.visibility, &plus_pat.visibility),
                (VisPat::Meta(_), _) | (_, VisPat::Meta(_))
            )
        {
            return Err(only_rename(&rule));
        }
        // `# returns=none` constrains the minus side only; the plus side
        // keeps `Any` (no `->` written means "preserve absence").
        let ret_compatible = hunk.returns_none()
            || minus_pat.ret == plus_pat.ret
            || matches!(
                (&minus_pat.ret, &plus_pat.ret),
                (RetPat::Meta(_), _) | (_, RetPat::Meta(_))
            );
        if minus_pat.qualifiers != plus_pat.qualifiers
            || minus_pat.generics != plus_pat.generics
            || minus_pat.where_clause != plus_pat.where_clause
            || !ret_compatible
        {
            return Err(only_rename(&rule));
        }
    }

    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    for c in collect_fns(parsed) {
        if !in_scope(&c, hunk.scope(), old_start, old_len) {
            continue;
        }
        // Name must equal the minus name (or bind if minus is `$Meta`).
        let name_ok = if minus_name.starts_with('$') {
            true
        } else {
            c.item.sig.ident == minus_name.as_str()
        };
        if !name_ok {
            continue;
        }
        if !minus_pat.matches_item_fn(c.item) {
            continue;
        }
        check_attrs(
            &c,
            hunk.attrs_policy(),
            &minus_pat.attrs_written,
            &rule,
            file,
        )?;
        if hunk.returns_none() && !matches!(c.item.sig.output, syn::ReturnType::Default) {
            continue;
        }
        // Rewrite: replace the name ident bytes. Plus name may be `$Meta`
        // (abstracted) meaning "keep original" — only concrete renames
        // change text.
        if !plus_name.starts_with('$') && c.item.sig.ident != plus_name {
            let ns = c.item.sig.ident.span();
            let (sl, sc, el, ec) = (
                ns.start().line as u32,
                ns.start().column,
                ns.end().line as u32,
                ns.end().column,
            );
            let start = index.offset(sl, sc).ok_or_else(|| range_err(&rule, sl))?;
            let end = index.offset(el, ec).ok_or_else(|| range_err(&rule, el))?;
            // Safety: the sliced text must be the old name.
            if c.item.sig.ident != original[start..end] {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "function name span mismatch (file changed during matching?)",
                )
                .with_rule(&rule));
            }
            edits.push(TextEdit::new(start, end, plus_name.clone()));
        }
        // `# attrs=strip`: delete attribute lines above the fn item.
        if hunk.attrs_policy() == AttrsPolicy::Strip && !c.item.attrs.is_empty() {
            let start = index
                .line_start(c.attr_start_line)
                .ok_or_else(|| range_err(&rule, c.attr_start_line))?;
            let end = index
                .line_start(c.fn_line)
                .ok_or_else(|| range_err(&rule, c.fn_line))?;
            if end > start {
                edits.push(TextEdit::new(start, end, String::new()));
            }
        }
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break; // count validation reports the error
        }
    }
    Ok(matched)
}

fn only_rename(rule: &str) -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        "v0.1 function hunks may differ only in the function name (same visibility, qualifiers, generics, return type, where clause)",
    )
    .with_rule(rule)
}

/// Find item targets (function or other items) for an item-capture hunk.
///
/// v0.1 matches any item (fn, struct, mod, use, ...) by line-span
/// containment; only functions are structurally verified elsewhere.
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
            ))
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
            ))
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

/// Rename a function inside an `impl` context block (SPEC 29.9).
pub fn apply_impl_context_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let impl_header = match &compiled.kind {
        PatternKind::ImplContext { impl_header } => impl_header.clone(),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected impl-context pattern".to_string(),
            ))
        }
    };
    let rule = hunk.rule_name();
    // Find the impl block whose header matches (whitespace-normalized).
    let want = impl_header.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut matched = 0;
    for it in &parsed.items {
        let syn::Item::Impl(imp) = it else { continue };
        let header_tokens: Vec<&str> = want.split_whitespace().collect();
        // Verify the impl's self type + optional trait match the pattern by
        // comparing normalized renderings.
        let pat_self = header_tokens.join(" ");
        let got_self = quote::quote!(#imp.self_ty)
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let got_trait = imp
            .trait_
            .as_ref()
            .map(|(_, p, _)| {
                quote::quote!(#p)
                    .to_string()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let pat_has_trait = pat_self.contains(&got_trait) && !got_trait.is_empty();
        let pat_mentions_self = header_tokens
            .iter()
            .any(|t| got_self.split_whitespace().any(|g| g == *t) || pat_self.contains(&got_self));
        if !pat_has_trait && !pat_mentions_self {
            // Fallback: accept impl blocks overlapping the range and let the
            // inner fn match decide (range-gated).
            let s = it.span().start().line as u32;
            let e = it.span().end().line as u32;
            let end = hunk.header.old_start + hunk.header.old_len.saturating_sub(1);
            if !(s <= end.max(hunk.header.old_start) && e >= hunk.header.old_start) {
                continue;
            }
        }
        // Match methods inside.
        let minus_pat = minus_fn_pattern(hunk, compiled).map_err(|e| e.with_rule(&rule))?;
        for item in &imp.items {
            let syn::ImplItem::Fn(m) = item else { continue };
            // Wrap as ItemFn-like check: name + modifiers via a synthetic
            // Name check against minus name.
            let (minus_name, plus_name) = {
                // Re-derive from patterns (ImplContext stores header only).
                let full_minus: String = hunk
                    .lines
                    .iter()
                    .filter(|l| l.is_removal() || l.is_context())
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let full_plus: String = hunk
                    .lines
                    .iter()
                    .filter(|l| l.is_addition() || l.is_context())
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                (extract_fn_name(&full_minus), extract_fn_name(&full_plus))
            };
            if m.sig.ident != minus_name {
                continue;
            }
            // Scope: method span within old range.
            let ms = m.span().start().line as u32;
            let me = m.span().end().line as u32;
            let end = hunk.header.old_start + hunk.header.old_len.saturating_sub(1);
            let inside = match hunk.scope() {
                Scope::Contained => {
                    ms >= hunk.header.old_start && me <= end.max(hunk.header.old_start)
                }
                _ => ms >= hunk.header.old_start && ms <= end.max(hunk.header.old_start),
            };
            if !inside {
                continue;
            }
            // Qualifier strictness via minus pattern on a synthetic ItemFn.
            let synth: syn::ItemFn =
                syn::parse_str(&format!("{} {{}}", full_sig_without_body(&m.sig))).map_err(
                    |_| {
                        StructuredError::new(
                            ErrorCode::PatternConflict,
                            "cannot analyze method signature",
                        )
                        .with_rule(&rule)
                    },
                )?;
            if !minus_pat.matches_item_fn(&synth) {
                continue;
            }
            let ns = m.sig.ident.span();
            let start = index
                .offset(ns.start().line as u32, ns.start().column)
                .ok_or_else(|| range_err(&rule, ns.start().line as u32))?;
            let endb = index
                .offset(ns.end().line as u32, ns.end().column)
                .ok_or_else(|| range_err(&rule, ns.end().line as u32))?;
            if m.sig.ident != original[start..endb] {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "method name span mismatch",
                )
                .with_rule(&rule));
            }
            if m.sig.ident != plus_name {
                edits.push(TextEdit::new(start, endb, plus_name.clone()));
            }
            matched += 1;
        }
    }
    Ok(matched)
}

/// Extract the function name from a (possibly multi-line) fn pattern block.
fn extract_fn_name(block: &str) -> String {
    for line in block.lines() {
        let t = line.trim();
        // Strip attributes.
        if t.starts_with("#[") || t.starts_with("///") || t.starts_with("//!") {
            continue;
        }
        // Find `fn <name>`.
        let mut toks = t
            .split(|c: char| c.is_whitespace() || c == '(' || c == '<')
            .filter(|s| !s.is_empty());
        let mut prev = String::new();
        for tok in &mut toks {
            if prev == "fn" {
                return tok.trim_matches('$').to_string();
            }
            if tok == "fn" {
                prev = "fn".to_string();
                continue;
            }
            // qualifiers/vis: keep scanning
            if ["pub", "async", "unsafe", "const", "extern"].contains(&tok) {
                continue;
            }
            if tok.starts_with("pub(") {
                continue;
            }
            prev = tok.to_string();
        }
        if prev == "fn" {
            continue;
        }
    }
    String::new()
}

/// Render a signature without its body for synthetic parsing.
fn full_sig_without_body(sig: &syn::Signature) -> String {
    let mut s = quote::quote!(#sig).to_string();
    // `quote` renders `fn foo (x : i32)` etc.; parse needs no trailing body.
    // Drop anything from the first `{` (shouldn't be present for Signature).
    if let Some(pos) = s.find('{') {
        s.truncate(pos);
    }
    // Ensure it starts with `fn` (methods have receiver; keep as-is for
    // qualifier checks by wrapping in a trait? No: parse as ItemFn requires
    // no `self`. Replace `self` params textually for the check.)
    s = s.replace("self", "_self_placeholder : u8");
    if !s.trim_start().starts_with("fn ") {
        s = format!("fn {s}");
    }
    s
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
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

    const RENAME: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";

    #[test]
    fn renames_plain_fn_preserving_body_and_attrs() {
        let src = "/// Adds one.\n#[inline]\nfn foo(x: i32) -> i32 {\n    x + 1\n}\n";
        let out = apply(RENAME, src).unwrap();
        assert!(out.contains("fn bar(x: i32) -> i32 {"), "got: {out}");
        assert!(out.contains("/// Adds one."));
        assert!(out.contains("#[inline]"));
        assert!(!out.contains("fn foo"));
    }

    #[test]
    fn rejects_pub_fn_for_plain_pattern() {
        let src = "pub fn foo() {}\n";
        let patch = RENAME.replace("@@ -1,5 +1,5 @@", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn rejects_generic_fn_for_plain_pattern() {
        let src = "fn foo<T>(x: T) {}\n";
        let patch = RENAME.replace("@@ -1,5 +1,5 @@", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn returns_none_constrains() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# returns=none\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        assert!(apply(patch, "fn foo() {}\n").is_ok());
        assert!(apply(patch, "fn foo() -> i32 {\n    1\n}\n").is_err());
    }

    #[test]
    fn attrs_absent_rejects_attributed() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# attrs=absent\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        assert!(apply(patch, "fn foo() {}\n").is_ok());
        let patch2 = patch.replace("@@ -1,1 +1,1 @@", "@@ -1,2 +1,2 @@");
        assert!(apply(&patch2, "#[inline]\nfn foo() {}\n").is_err());
    }

    #[test]
    fn optional_no_match_ok() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# optional\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let out = apply(patch, "fn a() {}\n").unwrap();
        assert_eq!(out, "fn a() {}\n");
    }

    #[test]
    fn deletes_item() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,0 @@ rule=cut\n# lang=rust\n# item I\n- I\n";
        let src = "fn a() {}\nfn dead() {}\nfn b() {}\n";
        // Range covers only `fn dead` (line 2); single-item delete.
        let patch2 = patch.replace("@@ -1,3 +1,0 @@", "@@ -2,1 +2,0 @@");
        let out = apply(&patch2, src).unwrap();
        assert!(!out.contains("fn dead"), "got: {out}");
        assert!(out.contains("fn a"));
        assert!(out.contains("fn b"));
    }

    #[test]
    fn inserts_use_item() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,1 @@ rule=ins\n# lang=rust\n+ use crate::new_module;\n";
        let src = "fn a() {}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("use crate::new_module;"), "got: {out}");
    }

    #[test]
    fn renames_inside_impl() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=x\n# lang=rust\n  impl Service {\n      ...\n-     fn old_name(...) {\n+     fn new_name(...) {\n           ...\n       }\n  }\n";
        let src = "impl Service {\n    fn old_name(&self) {}\n    fn other(&self) {}\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("fn new_name(&self)"), "got: {out}");
        assert!(out.contains("fn other(&self)"));
    }

    #[test]
    fn mixed_ordinary_and_semantic_disjoint_ok() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,8 +1,8 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n\n@@ -8,3 +8,3 @@\n fn calls() {\n-    old_call();\n+    new_call();\n }\n";
        let src = "fn foo() {}\nfn other() {}\nfn more() {}\nfn extra() {}\nfn tail() {}\nfn t2() {}\nfn t3() {}\nfn calls() {\n    old_call();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("fn bar()"), "got: {out}");
        assert!(out.contains("new_call();"), "got: {out}");
    }

    #[test]
    fn mixed_overlap_rejected() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n\n@@ -1,1 +1,1 @@\n-fn foo() {}\n+fn foo() { 1; }\n";
        let src = "fn foo() {}\n";
        assert!(apply(patch, src).is_err());
    }
}
