//! Zero-match diagnostic previews (T67).
//!
//! Split from `plan.rs` (WI-10): nearest-header candidate excerpts for
//! `no match found` diagnostics. Diagnostic-only: callers fire these on the
//! hard zero-match path, so successful and `# optional` hunks pay nothing.

use rsmpl_pattern::{GeneralShape, PatternKind};

use crate::lines::FileState;
/// T67 candidate preview: nearest same-kind item header (or nearest
/// non-empty source line as fallback) for zero-match diagnostics.
///
/// Diagnostic-only: returns a preformatted, pre-capped excerpt such as
/// `fn foo at line 5: 'fn foo() {}'`. Single line, at most 200 chars of
/// source plus an ellipsis marker. `None` when the file has nothing to
/// show (empty file). Callers fire this only on the hard zero-match path,
/// so successful and `# optional` hunks pay nothing.
pub(crate) fn candidate_preview(
    state: &FileState,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<String> {
    let old_start = rh.header.old_start;
    let kind_specific = matches!(
        &compiled.kind,
        PatternKind::Fn { .. }
            | PatternKind::ImplContext { .. }
            | PatternKind::ItemCapture { .. }
            | PatternKind::ItemRename { .. }
            | PatternKind::FieldRename { .. }
            | PatternKind::VariantRename { .. }
            | PatternKind::TypeAliasEvolution { .. }
            | PatternKind::ConstEvolution { .. }
            | PatternKind::UseRename { .. }
    );
    if kind_specific {
        if let Some(parsed) = state.parsed.as_ref() {
            let mut headers = Vec::new();
            collect_item_headers(&parsed.items, &mut headers);
            // T141: prefer the same-kind header (a typo'd `struct Lcoal`
            // should preview the real `struct Local`, not the enclosing
            // `fn outer`). Fall back to nearest-any when no same-kind
            // header exists, preserving the old behavior.
            // Prefix + optional wanted name for the same-kind tier.
            // `Fn` and `ItemRename` carry the concrete minus-side name, so
            // T142 can rank by similarity; `ImplContext`/`UseRename`
            // headers have no single comparable name and keep the original
            // distance ranking.
            let want: Option<(String, Option<String>)> = match &compiled.kind {
                PatternKind::Fn { minus_name, .. } => {
                    Some(("fn ".to_string(), Some(minus_name.clone())))
                }
                PatternKind::ItemRename {
                    kind, minus_name, ..
                } => Some((format!("{} ", kind.as_str()), Some(minus_name.clone()))),
                PatternKind::FieldRename { struct_name, .. } => {
                    Some(("struct ".to_string(), Some(struct_name.clone())))
                }
                PatternKind::TypeAliasEvolution { name, .. } => {
                    Some(("type ".to_string(), Some(name.clone())))
                }
                PatternKind::ConstEvolution {
                    is_static, name, ..
                } => Some((
                    if *is_static {
                        "static ".to_string()
                    } else {
                        "const ".to_string()
                    },
                    Some(name.clone()),
                )),
                PatternKind::VariantRename { enum_name, .. } => {
                    Some(("enum ".to_string(), Some(enum_name.clone())))
                }
                PatternKind::ImplContext { .. } => Some(("fn ".to_string(), None)),
                PatternKind::UseRename { .. } => Some(("use ".to_string(), None)),
                _ => None,
            };
            let nearest = match want {
                Some((prefix, wanted)) => {
                    let same_kind: Vec<(u32, String)> = headers
                        .iter()
                        .filter(|(_, label)| label.starts_with(&prefix))
                        .cloned()
                        .collect();
                    // T142: rank same-kind headers by name similarity first
                    // (a typo'd `struct Betta` should preview the real
                    // `struct Beta`, not the nearer `struct Alpha`), line
                    // distance breaks ties. No-name shapes keep distance.
                    match wanted {
                        Some(name) => nearest_similar(&same_kind, &prefix, &name, old_start)
                            .or_else(|| nearest_header(&headers, old_start)),
                        None => nearest_header(&same_kind, old_start)
                            .or_else(|| nearest_header(&headers, old_start)),
                    }
                }
                None => nearest_header(&headers, old_start),
            };
            if let Some((line, label)) = nearest {
                if let Some(excerpt) = source_line(state, line) {
                    return Some(cap_preview(format!("{label} at line {line}: '{excerpt}'")));
                }
            }
        }
    }
    // WI-2b head-token tier: the dummy `@@ -1,1 @@` range of `# sweep`
    // hunks makes the plain nearest-line fallback point at line 1; the
    // pattern’s own head token points at its vocabulary instead.
    if let Some(head) = head_token(&compiled.kind) {
        if let Some(preview) = nearest_line_with(state, &head, old_start) {
            return Some(cap_preview(preview.1));
        }
    }
    nearest_source(state, old_start).map(cap_preview)
}

/// WI-2b head-token tier: the concrete token that names what the pattern is
/// looking for (`let` for `let` statements, the macro/callee/method name
/// for expressions and calls). Zero-match previews for `# sweep` hunks cite
/// a dummy `@@ -1,1 @@` range, so the plain nearest-line fallback points
/// at line 1; this tier points at the pattern's own vocabulary instead.
/// `None` when the pattern names nothing concrete (all-hole templates keep
/// the old fallback).
fn head_token(kind: &PatternKind) -> Option<String> {
    match kind {
        PatternKind::General {
            shape, minus_sub, ..
        } => match shape {
            // The bound name (`b` in `let b = …`) is as generic as `let`; the
            // longest concrete ident (usually the callee/macro name) locates
            // the pattern’s line. All-hole templates keep the old fallback.
            GeneralShape::Stmt => {
                longest_concrete_ident(minus_sub).or_else(|| Some("let".to_string()))
            }
            GeneralShape::Expr | GeneralShape::Sequence | GeneralShape::Type => {
                longest_concrete_ident(minus_sub)
            }
        },
        PatternKind::Expr { minus, .. } => match minus {
            rsmpl_pattern::ExprPatternKind::Call { callee, .. } => {
                Some(callee.rsplit("::").next().unwrap_or(callee).to_string())
            }
            rsmpl_pattern::ExprPatternKind::MethodCall { method, .. } => Some(method.clone()),
            rsmpl_pattern::ExprPatternKind::Macro { name, .. } => Some(name.clone()),
            rsmpl_pattern::ExprPatternKind::Raw { text } => longest_concrete_ident(text),
        },
        PatternKind::Macro { minus_raw, .. } => longest_concrete_ident(minus_raw),
        PatternKind::TypeAliasEvolution { minus_sub, .. }
        | PatternKind::ConstEvolution { minus_sub, .. } => longest_concrete_ident(minus_sub),
        _ => None,
    }
}

/// Longest ident-like token that is not a substituted placeholder
/// (`__RSMPL_*` markers name holes, never source text). Length wins over
/// position: the callee/macro name locates the pattern’s line better than
/// the bound name or `let`. First wins ties in source order.
fn longest_concrete_ident(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut best: Option<String> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i + 1;
            while j < bytes.len() {
                let d = bytes[j] as char;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            let tok = &text[i..j];
            if !tok.starts_with("__RSMPL_")
                && best.as_ref().is_none_or(|b: &String| tok.len() > b.len())
            {
                best = Some(tok.to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    best
}

/// Nearest non-empty source line containing `token` (case-sensitive
/// substring), by distance from `old_start`. `None` when no line names it.
fn nearest_line_with(state: &FileState, token: &str, old_start: u32) -> Option<(u32, String)> {
    let n = state.index.line_count() as u32;
    let mut best: Option<(u32, u32, String)> = None; // (distance, line, excerpt)
    for line in 1..=n {
        let start = state.index.line_start(line)?;
        let end = state
            .index
            .line_start(line + 1)
            .unwrap_or_else(|| state.index.eof());
        let raw = state.original.get(start..end.min(state.original.len()))?;
        if !raw.contains(token) {
            continue;
        }
        let Some(excerpt) = source_line(state, line) else {
            continue;
        };
        let d = line.abs_diff(old_start);
        if best.as_ref().is_none_or(|(bd, _, _)| d < *bd) {
            best = Some((d, line, excerpt));
        }
    }
    best.map(|(_, line, excerpt)| (line, format!("source at line {line}: '{excerpt}'")))
}
/// `(line, label)` item headers for the T67 kind-aware tier.
/// T141: walks the full `Visit` tree so block-level items (fn bodies,
/// closures, method bodies — the same scope the T23 matcher reaches) are
/// previewed, not just file/inline-mod items. Macro bodies stay excluded
/// only insofar as `syn` nests them as tokens, matching matcher behavior.
fn collect_item_headers(items: &[syn::Item], out: &mut Vec<(u32, String)>) {
    use syn::spanned::Spanned;
    use syn::visit::{self, Visit};
    struct Headers {
        out: Vec<(u32, String)>,
    }
    impl<'ast> Visit<'ast> for Headers {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            match it {
                syn::Item::Fn(f) => {
                    let line = f.sig.fn_token.span().start().line as u32;
                    self.out.push((line, format!("fn {}", f.sig.ident)));
                }
                syn::Item::Struct(s) => {
                    self.out.push((
                        s.ident.span().start().line as u32,
                        format!("struct {}", s.ident),
                    ));
                }
                syn::Item::Enum(e) => {
                    self.out.push((
                        e.ident.span().start().line as u32,
                        format!("enum {}", e.ident),
                    ));
                }
                syn::Item::Trait(t) => {
                    self.out.push((
                        t.ident.span().start().line as u32,
                        format!("trait {}", t.ident),
                    ));
                }
                syn::Item::Union(u) => {
                    self.out.push((
                        u.ident.span().start().line as u32,
                        format!("union {}", u.ident),
                    ));
                }
                syn::Item::Type(t) => {
                    self.out.push((
                        t.ident.span().start().line as u32,
                        format!("type {}", t.ident),
                    ));
                }
                syn::Item::Const(c) => {
                    self.out.push((
                        c.ident.span().start().line as u32,
                        format!("const {}", c.ident),
                    ));
                }
                syn::Item::Static(s) => {
                    self.out.push((
                        s.ident.span().start().line as u32,
                        format!("static {}", s.ident),
                    ));
                }
                syn::Item::Mod(m) => {
                    self.out.push((
                        m.ident.span().start().line as u32,
                        format!("mod {}", m.ident),
                    ));
                }
                syn::Item::Use(u) => {
                    let tree = &u.tree;
                    let rendered = quote::quote!(#tree).to_string();
                    self.out.push((
                        u.use_token.span.start().line as u32,
                        format!("use {rendered}"),
                    ));
                }
                syn::Item::Impl(imp) => {
                    // T19b: name both sides so the preview distinguishes impl
                    // headers (`impl Printable for Old` vs `impl Old`).
                    let line = imp.self_ty.span().start().line as u32;
                    let self_ty = &imp.self_ty;
                    let rendered_self = quote::quote!(#self_ty).to_string();
                    let label = match &imp.trait_ {
                        Some((_, path, _)) => {
                            format!("impl {} for {rendered_self}", quote::quote!(#path))
                        }
                        None => format!("impl {rendered_self}"),
                    };
                    self.out.push((line, label));
                    for it in &imp.items {
                        if let syn::ImplItem::Fn(m) = it {
                            let line = m.sig.fn_token.span().start().line as u32;
                            self.out.push((line, format!("fn {}", m.sig.ident)));
                        }
                    }
                }
                _ => {}
            }
            visit::visit_item(self, it);
        }
    }
    let mut h = Headers { out: Vec::new() };
    for it in items {
        h.visit_item(it);
    }
    out.extend(h.out);
}

fn nearest_header(headers: &[(u32, String)], old_start: u32) -> Option<(u32, String)> {
    headers
        .iter()
        .min_by_key(|(line, _)| line.abs_diff(old_start))
        .map(|(line, label)| (*line, label.clone()))
}

/// T142: most similar same-kind header to `wanted`, line distance breaking
/// ties. Candidate names are labels minus the `kind ` prefix. Diagnostic
/// only — caps and fallback tiers are unchanged.
fn nearest_similar(
    headers: &[(u32, String)],
    prefix: &str,
    wanted: &str,
    old_start: u32,
) -> Option<(u32, String)> {
    headers
        .iter()
        .min_by_key(|(line, label)| {
            let name = label.strip_prefix(prefix).unwrap_or(label);
            (edit_distance(wanted, name), line.abs_diff(old_start))
        })
        .map(|(line, label)| (*line, label.clone()))
}

/// Char-level Levenshtein distance (std only, no new dependency).
/// Preview-sized inputs (idents), so the quadratic table is trivial.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, &cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// First source line at 1-based `line`, trimmed, capped at 200 chars.
fn source_line(state: &FileState, line: u32) -> Option<String> {
    let start = state.index.line_start(line)?;
    let end = state
        .index
        .line_start(line + 1)
        .unwrap_or_else(|| state.index.eof());
    let raw = state.original.get(start..end.min(state.original.len()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Single-line excerpt: a header line never legitimately spans lines
    // here (`source_line` slices one line); cut at the first newline anyway.
    let first = trimmed.lines().next().unwrap_or("");
    Some(truncate_excerpt(first))
}

fn truncate_excerpt(s: &str) -> String {
    const MAX: usize = 200;
    if s.chars().count() > MAX {
        let mut out: String = s.chars().take(MAX).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

/// Hard bound on the whole preview string: pathological idents (a 300-char
/// fn name) would otherwise blow past the 200-char excerpt cap via the
/// `kind name` label. Single line, token-bounded failure output.
fn cap_preview(s: String) -> String {
    const MAX: usize = 320;
    if s.chars().count() > MAX {
        let mut out: String = s.chars().take(MAX).collect();
        out.push('…');
        out
    } else {
        s
    }
}

/// Fallback tier: nearest non-empty source line to `old_start`.
fn nearest_source(state: &FileState, old_start: u32) -> Option<String> {
    let n = state.index.line_count() as u32;
    let mut best: Option<(u32, u32)> = None; // (distance, line)
    for line in 1..=n {
        let start = state.index.line_start(line)?;
        let end = state
            .index
            .line_start(line + 1)
            .unwrap_or_else(|| state.index.eof());
        let raw = state.original.get(start..end.min(state.original.len()))?;
        if raw.trim().is_empty() {
            continue;
        }
        let d = line.abs_diff(old_start);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, line));
        }
    }
    let (_, line) = best?;
    source_line(state, line).map(|excerpt| format!("source at line {line}: '{excerpt}'"))
}
