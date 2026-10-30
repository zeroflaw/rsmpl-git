//! Zero-match diagnostic previews (T67).
//!
//! Split from `plan.rs` (WI-10): nearest-header candidate excerpts for
//! `no match found` diagnostics. Diagnostic-only: callers fire these on the
//! hard zero-match path, so successful and `# optional` hunks pay nothing.

use rsmpl_pattern::PatternKind;

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
    nearest_source(state, old_start).map(cap_preview)
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
