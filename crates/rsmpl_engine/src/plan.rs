//! Two-pass application planner (SPEC 23.1).
//!
//! All RSmPL old-side ranges refer to **original-source coordinates**
//! (SPEC 10.2): matching never sees the output of earlier hunks.
//!
//! - Pass 0: parse every targeted file, build line indexes, validate ranges.
//! - Pass 1: evaluate `# bind=` cut hunks (item captures) and record the
//!   captured source text. Forward references work because uses are resolved
//!   only in pass 2 (SPEC 23.2-23.3).
//! - Pass 2: evaluate every other hunk in document order into a single
//!   per-file edit list (original coordinates), resolving `# use=` pastes.
//! - Finally: overlap-check, materialize, reparse-validate, report.

use std::collections::HashMap;

use rsmpl_patch::{Directive, ErrorCode, Hunk, Patch, Scope, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::capture::CaptureStore;
use crate::edits::{apply_edits, TextEdit};
use crate::lines::LineIndex;
use crate::validate::{ensure_match_count, ZeroMatchHint};
use crate::{matcher_expr, matcher_fn, matcher_general, matcher_item, matcher_macro, matcher_use};
use crate::{ordinary_hunk_edits, RuleReport};

/// Parsed original state for one file.
///
/// `parsed` is `None` for ordinary-only files (T137): a file touched by
/// no RSmPL hunk is plain text — no Rust parse of the original or the
/// rewritten file. Any file with >=1 RSmPL hunk keeps `Some` and the
/// current strict Rust behavior.
struct FileState {
    original: String,
    index: LineIndex,
    parsed: Option<syn::File>,
}

/// Apply `patch` to `files` (path -> contents) in place.
pub fn apply_with_captures(
    files: &mut HashMap<String, String>,
    patch: &Patch,
) -> Result<crate::ApplyReport, StructuredError> {
    let mut report = crate::ApplyReport::default();

    // ---- Pass 0: parse all targeted files. ----
    // T137: ordinary-only files skip the Rust parse entirely (original +
    // rewritten). The unit is the file: any RSmPL hunk in any section with
    // the same target path means Rust.
    let mut states: HashMap<String, FileState> = HashMap::new();
    for section in &patch.files {
        let path = section.target_path.clone();
        if states.contains_key(&path) {
            continue;
        }
        let original = files.get(&path).cloned().ok_or_else(|| {
            StructuredError::new(
                ErrorCode::ParseConflict,
                format!("target file not found in tree: {path}"),
            )
        })?;
        let index = LineIndex::new(&original);
        crate::validate::ensure_file_range(&index, section, &path)?;
        let has_semantic = patch
            .files
            .iter()
            .filter(|s| s.target_path == path)
            .any(|s| s.hunks.iter().any(|h| matches!(h, Hunk::Rsmpl(_))));
        let parsed: Option<syn::File> = if has_semantic {
            Some(syn::parse_str(&original).map_err(|e| {
                StructuredError::new(
                    ErrorCode::ParseConflict,
                    format!("target file does not parse as Rust: {e}"),
                )
                .with_file(&path)
            })?)
        } else {
            None
        };
        states.insert(
            path,
            FileState {
                original,
                index,
                parsed,
            },
        );
    }

    let mut captures = CaptureStore::default();
    // Per-file edit lists, built across pass 1 (cuts) and pass 2 (rest).
    let mut file_edits: HashMap<String, Vec<TextEdit>> = HashMap::new();
    // T249: same-anchor capture-paste edit indices per file, in document
    // order. Used after pass 2 to join multi-pastes with one blank line.
    let mut paste_marks: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for path in states.keys() {
        file_edits.insert(path.clone(), Vec::new());
        paste_marks.insert(path.clone(), Vec::new());
    }
    // Rule reports staged with their document position (T45): pass 1
    // evaluates cuts first, but the report must read in patch document
    // order (section order, then hunk order), so entries are sorted
    // after pass 2 before being committed to `report.rules`.
    let mut rule_order: Vec<((usize, usize), RuleReport)> = Vec::new();

    // ---- Pass 1: `# bind=` cut hunks (v0.1: item captures only). ----
    for (sec_idx, section) in patch.files.iter().enumerate() {
        let path = &section.target_path;
        let Some(state) = states.get(path) else {
            continue;
        };
        for (hunk_idx, hunk) in section.hunks.iter().enumerate() {
            let Hunk::Rsmpl(rh) = hunk else { continue };
            let binds = rh.binds();
            if binds.is_empty() {
                continue;
            }
            if binds.len() > 1 {
                return Err(StructuredError::new(
                    ErrorCode::CaptureConflict,
                    "only one `# bind=<Name>` per hunk is supported in v0.1",
                )
                .with_rule(rh.rule_name())
                .with_file(path));
            }
            let compiled = rsmpl_pattern::compile(rh)
                .map_err(|e| e.with_rule(rh.rule_name()).with_file(path))?;
            gate_directives(rh, &compiled, path)?;
            if !matches!(&compiled.kind, PatternKind::ItemCapture { .. }) {
                return Err(StructuredError::new(
                    ErrorCode::CaptureConflict,
                    "v0.1 `# bind=` is only supported on item-capture cut hunks (`- I`)",
                )
                .with_rule(rh.rule_name())
                .with_file(path));
            }
            let parsed = state.parsed.as_ref().expect(
                "T137: RSmPL cut hunk on a file with no parsed AST (ordinary-only files carry no semantic hunks)",
            );
            let targets = matcher_fn::find_item_targets(
                parsed,
                rh.scope(),
                rh.header.old_start,
                rh.header.old_len,
            );
            let range_desc = format!(
                "{}..{}",
                rh.header.old_start,
                rh.header.old_start + rh.header.old_len
            );
            if targets.is_empty() && rh.is_optional() {
                continue; // optional cut with no match: no capture, no cut
            }
            // T62: same whole-file probe for cut hunks (span collection,
            // no edits involved).
            let wide = if targets.is_empty() && rh.scope() != Scope::File {
                let wide_targets = matcher_fn::find_item_targets(
                    parsed,
                    Scope::File,
                    rh.header.old_start,
                    rh.header.old_len,
                );
                Some(wide_targets.len())
            } else {
                None
            };
            // T67: nearest-item excerpt so the retry often needs no read.
            let preview = if targets.is_empty() {
                candidate_preview(state, rh, &compiled)
            } else {
                None
            };
            ensure_match_count(
                targets.len(),
                rsmpl_patch::Allow::One,
                false,
                &rh.rule_name(),
                path,
                &range_desc,
                ZeroMatchHint { wide, preview },
            )?;
            let (sl, el) = targets[0];
            // Capture the exact item span (no blank-line absorption); the
            // delete edit may absorb an adjacent blank separator line.
            let cap_start = state
                .index
                .line_start(sl)
                .ok_or_else(|| range_err(&rh.rule_name(), path, sl))?;
            let cap_end = state
                .index
                .line_start(el + 1)
                .unwrap_or_else(|| state.index.eof());
            let cut =
                matcher_fn::delete_lines(&state.original, &state.index, sl, el, &rh.rule_name())
                    .map_err(|e| e.with_file(path))?;
            let captured = state
                .original
                .get(cap_start..cap_end)
                .unwrap_or("")
                .to_string();
            captures.define(binds[0], captured, &rh.rule_name(), path)?;
            file_edits.get_mut(path).expect("file state").push(cut);
            rule_order.push((
                (sec_idx, hunk_idx),
                RuleReport {
                    rule: rh.rule_name(),
                    file: path.clone(),
                    matches: 1,
                    spans: vec![(sl, el)],
                },
            ));
        }
    }

    // ---- Pass 2: every other hunk in document order. ----
    for (sec_idx, section) in patch.files.iter().enumerate() {
        let path = &section.target_path;
        let Some(state) = states.get(path) else {
            continue;
        };
        let edits = file_edits.get_mut(path).expect("file state");
        for (hunk_idx, hunk) in section.hunks.iter().enumerate() {
            match hunk {
                Hunk::Ordinary(oh) => {
                    let mut e = ordinary_hunk_edits(&state.index, &state.original, oh)
                        .map_err(|s| s.with_file(path))?;
                    edits.append(&mut e);
                }
                Hunk::Rsmpl(rh) => {
                    if !rh.binds().is_empty() {
                        continue; // handled in pass 1
                    }
                    let compiled = rsmpl_pattern::compile(rh)
                        .map_err(|e| e.with_rule(rh.rule_name()).with_file(path))?;
                    gate_directives(rh, &compiled, path)?;
                    if !compiled.unbound_plus.is_empty() {
                        return Err(unbound_plus(rh, &compiled.unbound_plus, path));
                    }
                    let rule = rh.rule_name();
                    let range_desc = format!(
                        "{}..{}",
                        rh.header.old_start,
                        rh.header.old_start + rh.header.old_len
                    );
                    let edits_before = edits.len();
                    let found = apply_rsmpl_hunk(state, path, rh, &compiled, &captures, edits)
                        .map_err(|e| e.with_file(path))?;
                    // T249: record capture-paste edits for blank-join. A
                    // paste hunk pushes exactly one insertion at
                    // `edits_before`, so its index is stable.
                    if matches!(
                        &compiled.kind,
                        PatternKind::Insertion {
                            use_name: Some(_),
                            ..
                        }
                    ) && found == 1
                        && edits.len() == edits_before + 1
                    {
                        let pos = edits[edits_before].start;
                        debug_assert_eq!(edits[edits_before].end, pos);
                        if let Some(marks) = paste_marks.get_mut(path) {
                            marks.push((pos, edits_before));
                        }
                    }
                    let spans = edit_spans(&state.index, &edits[edits_before..]);
                    // A non-insertion hunk carrying `# use=` has no v0.1 meaning.
                    if !rh.uses().is_empty()
                        && !matches!(
                            &compiled.kind,
                            PatternKind::Insertion {
                                use_name: Some(_),
                                ..
                            }
                        )
                    {
                        return Err(StructuredError::new(
                            ErrorCode::CaptureConflict,
                            "v0.1 `# use=` is only supported on pure insertion hunks (`+ Name`)",
                        )
                        .with_rule(rule.clone())
                        .with_file(path));
                    }
                    // T62: on a hard zero-match, probe the same pattern
                    // whole-file (scope overridden, count-only, edits
                    // discarded) so an unambiguous failure can suggest the
                    // zero-discovery `# scope=file` retry. Insertions use
                    // anchors rather than pattern search and are excluded
                    // by the probe itself.
                    let wide = if found == 0 && !rh.is_optional() && rh.scope() != Scope::File {
                        probe_whole_file(state, path, rh, &compiled, &captures)
                    } else {
                        None
                    };
                    // T67: diagnostic-only excerpt; happy/optional paths pay nothing.
                    let preview = if found == 0 && !rh.is_optional() {
                        candidate_preview(state, rh, &compiled)
                    } else {
                        None
                    };
                    ensure_match_count(
                        found,
                        rh.allow(),
                        rh.is_optional(),
                        &rule,
                        path,
                        &range_desc,
                        ZeroMatchHint { wide, preview },
                    )?;
                    rule_order.push((
                        (sec_idx, hunk_idx),
                        RuleReport {
                            rule,
                            file: path.clone(),
                            matches: found,
                            spans,
                        },
                    ));
                }
            }
        }
    }

    // T249: join same-anchor multi-pastes with one blank line. Each paste
    // pushes `captured.trim_end() + "\n"` (no separators by construction,
    // since the cut absorbs the blank — SPEC 10.5). A lone paste is left
    // alone (surrounding context unchanged); for N>1 pastes at the same
    // byte position, every paste but the last in document order gains one
    // extra "\n" so the document-order materialization (SPEC 23.5) yields
    // exactly one blank line between items. Ordinary literal insertions
    // are untouched — the agent controls their blanks explicitly.
    for (path, marks) in &paste_marks {
        if marks.len() < 2 {
            continue;
        }
        let mut by_pos: HashMap<usize, Vec<usize>> = HashMap::new();
        for (pos, idx) in marks {
            by_pos.entry(*pos).or_default().push(*idx);
        }
        if let Some(edits) = file_edits.get_mut(path) {
            for indices in by_pos.values() {
                if indices.len() < 2 {
                    continue;
                }
                let mut ordered = indices.clone();
                ordered.sort_unstable();
                // All but the last in document order get the separator.
                for idx in &ordered[..ordered.len() - 1] {
                    let e = &mut edits[*idx];
                    debug_assert!(e.is_insertion());
                    if !e.replacement.ends_with("\n\n") {
                        e.replacement.push('\n');
                    }
                }
            }
        }
    }

    // Commit rule reports in patch document order (T36/T45 contract:
    // section order, then hunk order within a section).
    rule_order.sort_by_key(|(pos, _)| *pos);
    report.rules.extend(rule_order.into_iter().map(|(_, r)| r));

    // ---- Pass 3: overlap check, materialize, reparse-validate. ----
    // T137: ordinary-only files (`parsed.is_none()`) skip the Rust
    // reparse — they are plain text, not Rust.
    for (path, state) in &states {
        let edits = file_edits.remove(path).unwrap_or_default();
        if edits.is_empty() {
            continue;
        }
        check_overlaps(&edits, path)?;
        let updated = apply_edits(&state.original, edits).map_err(|e| e.with_file(path))?;
        if state.parsed.is_some() {
            syn::parse_str::<syn::File>(&updated).map_err(|e| {
                StructuredError::new(
                    ErrorCode::ParseConflict,
                    format!("rewritten file does not parse as Rust: {e}"),
                )
                .with_file(path)
            })?;
        }
        files.insert(path.clone(), updated);
    }

    Ok(report)
}

/// Dispatch one pass-2 RSmPL hunk to its matcher (SPEC 23.1 step 7).
///
/// Extracted from the pass-2 loop so the T62 whole-file probe reuses the
/// exact same dispatch (no predicate duplication, no drift). File context
/// (`with_file`) is applied by the caller, not here, so real and probe
/// runs share error shapes.
fn apply_rsmpl_hunk(
    state: &FileState,
    path: &str,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    captures: &CaptureStore,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let rule = rh.rule_name();
    let parsed = state.parsed.as_ref().expect(
        "T137: RSmPL hunk on a file with no parsed AST (ordinary-only files carry no semantic hunks)",
    );
    match &compiled.kind {
        PatternKind::Fn { .. } => matcher_fn::apply_fn_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Expr {
            minus: rsmpl_pattern::ExprPatternKind::Macro { .. },
            ..
        } => matcher_macro::apply_macro_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Expr { .. } => matcher_expr::apply_expr_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ItemRename { .. } => matcher_item::apply_item_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::UseRename { .. } => matcher_use::apply_use_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::General { .. } => matcher_general::apply_general_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ItemCapture { .. } => matcher_fn::apply_item_delete(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Insertion { use_name, .. } => match use_name {
            Some(name) => {
                paste_capture(
                    captures,
                    name,
                    &state.index,
                    rh.header.old_start,
                    &rule,
                    path,
                    edits,
                )?;
                Ok(1)
            }
            None => {
                matcher_fn::apply_insertion(&state.original, &state.index, rh, compiled, edits)
            }
        },
        PatternKind::ImplContext { .. } => matcher_fn::apply_impl_context_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Macro { .. } => matcher_macro::apply_macro_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Raw { .. } => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "unsupported pattern shape in v0.1 (only fn rename, struct/enum/trait/union/type/const/static/mod/impl item rename, `use`-path prefix rename, call rewrite, let-stmt/general-expr/type rewrite, item delete/insert, macro path rewrite)",
        )
        .with_rule(rule)),
    }
}

/// T62 whole-file probe: re-run the same matcher with scope overridden to
/// `File` and count matches without keeping edits.
///
/// Returns `None` when the probe itself errors (keep the original error's
/// generic hint) or the hunk kind is not probe-eligible: insertions use
/// anchors rather than pattern search, and `Raw` shapes never reach here
/// (rejected before dispatch). The probe fires only on the zero-match
/// path, so the happy path pays no extra matching cost.
fn probe_whole_file(
    state: &FileState,
    path: &str,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    captures: &CaptureStore,
) -> Option<usize> {
    if matches!(
        &compiled.kind,
        PatternKind::Insertion { .. } | PatternKind::Raw { .. }
    ) {
        return None;
    }
    let mut wide_hunk = rh.clone();
    wide_hunk.directives.push(Directive::Scope(Scope::File));
    let mut probe_edits = Vec::new();
    apply_rsmpl_hunk(
        state,
        path,
        &wide_hunk,
        compiled,
        captures,
        &mut probe_edits,
    )
    .ok()
}

/// T67 candidate preview: nearest same-kind item header (or nearest
/// non-empty source line as fallback) for zero-match diagnostics.
///
/// Diagnostic-only: returns a preformatted, pre-capped excerpt such as
/// `fn foo at line 5: 'fn foo() {}'`. Single line, at most 200 chars of
/// source plus an ellipsis marker. `None` when the file has nothing to
/// show (empty file). Callers fire this only on the hard zero-match path,
/// so successful and `# optional` hunks pay nothing.
fn candidate_preview(
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

/// v0.1 capability gate (SPEC 22-23): only `format=preserve` and AST matching
/// (plus macro token-tree patterns).
fn gate_directives(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    path: &str,
) -> Result<(), StructuredError> {
    if rh.format_mode() != rsmpl_patch::FormatMode::Preserve {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "only `# format=preserve` is supported in v0.1",
        )
        .with_rule(rh.rule_name())
        .with_file(path)
        .with_suggestion("remove the directive and run `cargo fmt` after apply"));
    }
    let is_macro = matches!(
        &compiled.kind,
        PatternKind::Macro { .. }
            | PatternKind::Expr {
                minus: rsmpl_pattern::ExprPatternKind::Macro { .. },
                ..
            }
    ) || rh.macro_mode() == rsmpl_patch::MacroMode::TokenTree;
    if rh.parse_mode() == rsmpl_patch::ParseMode::Tokens && !is_macro {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# mode=tokens` is only supported for macro token-tree patterns in v0.1",
        )
        .with_rule(rh.rule_name())
        .with_file(path));
    }
    Ok(())
}

fn unbound_plus(rh: &rsmpl_patch::RsmplHunk, names: &[String], path: &str) -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        format!(
            "metavariable(s) used in `+` pattern without binding: {}",
            names.join(", ")
        ),
    )
    .with_rule(rh.rule_name())
    .with_file(path)
    .with_suggestion("bind them in `-`/context or declare `# use=`")
}

/// Paste captured item text at a zero-length anchor (SPEC 10.5, 20.2).
/// Single pastes end with exactly one `\n`; same-anchor multi-pastes are
/// blank-joined after pass 2 (SPEC 20.4) and materialize in document order
/// (SPEC 23.5: first hunk topmost).
fn paste_capture(
    captures: &CaptureStore,
    name: &str,
    index: &LineIndex,
    anchor: u32,
    rule: &str,
    file: &str,
    edits: &mut Vec<TextEdit>,
) -> Result<(), StructuredError> {
    let text = captures.get(name, rule, file)?;
    let pos = if anchor == 0 {
        0
    } else {
        index.line_start(anchor + 1).unwrap_or_else(|| index.eof())
    };
    let mut ins = text.trim_end().to_string();
    if !ins.ends_with('\n') {
        ins.push('\n');
    }
    edits.push(TextEdit::new(pos, pos, ins));
    Ok(())
}

/// Per-match original-source line spans for one hunk's new edits (T27).
///
/// Each `TextEdit` covers `original[start..end]`; the span is the 1-based
/// inclusive `[line_of(start), line_of(end)]` pair. Two normalizations keep
/// spans tight: insertions (`start == end`) report `[anchor, anchor]`, and an
/// `end` landing exactly on a line start (whole-line deletes) is attributed
/// to the previous line so deleting line 5 reports `[5, 5]`, not `[5, 6]`.
fn edit_spans(index: &LineIndex, new_edits: &[TextEdit]) -> Vec<(u32, u32)> {
    new_edits
        .iter()
        .map(|e| {
            let s = index.line_of(e.start);
            let en = if e.end > e.start && index.line_start(index.line_of(e.end)) == Some(e.end) {
                index.line_of(e.end).saturating_sub(1).max(s)
            } else {
                index.line_of(e.end)
            };
            (s, en)
        })
        .collect()
}

/// Reject incompatible overlapping edits (SPEC 10.6, 23.5).
///
/// Same-point insertions are allowed (deterministic stable order, document
/// order). An insertion strictly inside a deleted/replaced range is rejected,
/// as are partially overlapping replacements.
fn check_overlaps(edits: &[TextEdit], path: &str) -> Result<(), StructuredError> {
    for i in 0..edits.len() {
        for j in (i + 1)..edits.len() {
            let (a, b) = (&edits[i], &edits[j]);
            if a.is_insertion() && b.is_insertion() {
                continue; // same-point insertions: deterministic order
            }
            if a.overlaps(b) {
                return Err(StructuredError::new(
                    ErrorCode::GitHunkConflict,
                    "overlapping edits cannot be merged",
                )
                .with_file(path)
                .with_suggestion(
                    "apply the overlapping rewrites in separate runs, or make the ranges disjoint",
                ));
            }
        }
    }
    Ok(())
}

fn range_err(rule: &str, file: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_file(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_multi(
        patch_text: &str,
        initial: Vec<(&str, &str)>,
    ) -> Result<HashMap<String, String>, String> {
        apply_multi_report(patch_text, initial).map(|(files, _)| files)
    }

    fn apply_multi_report(
        patch_text: &str,
        initial: Vec<(&str, &str)>,
    ) -> Result<(HashMap<String, String>, crate::ApplyReport), String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files: HashMap<String, String> = initial
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let report = apply_with_captures(&mut files, &patch).map_err(|e| e.render())?;
        Ok((files, report))
    }

    const UPWARD_MOVE: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +2,0 @@ rule=paste_helper_near_top\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -6,4 +6,0 @@ rule=cut_helper_from_bottom\n# lang=rust\n# item I\n# bind=Helper\n- I\n";

    #[test]
    fn upward_move_with_forward_ref() {
        // Paste hunk (anchor after line 2) precedes the cut hunk in document
        // order; the engine must still resolve `Helper` (SPEC 23.3).
        let src = "fn top() {}\nfn mid() {}\nfn keep() {}\nfn other() {}\nfn tail() {}\nfn helper(x: i32) -> i32 {\n    x + 1\n}\n";
        // helper spans lines 6-8; cut range 6,4 exceeds (8 lines: 6+4-1=9 > 8). Fix range below.
        let patch = UPWARD_MOVE.replace("@@ -6,4 +6,0 @@", "@@ -6,3 +6,0 @@");
        let files = apply_multi(&patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        // helper moved to after line 2.
        let helper_pos = out.find("fn helper").unwrap();
        let top_pos = out.find("fn top").unwrap();
        let mid_pos = out.find("fn mid").unwrap();
        let keep_pos = out.find("fn keep").unwrap();
        assert!(
            top_pos < mid_pos && mid_pos < helper_pos && helper_pos < keep_pos,
            "got:\n{out}"
        );
        assert_eq!(out.matches("fn helper").count(), 1);
    }

    #[test]
    fn cross_file_move_with_import_hunk() {
        let patch = "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -1,1 +1,0 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n\ndiff --git a/src/new.rs b/src/new.rs\n--- a/src/new.rs\n+++ b/src/new.rs\n@@ -1,0 +1,0 @@ rule=paste_helper\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -1,1 +1,2 @@\n use crate::common;\n+use crate::old_dependency;\n";
        let files = apply_multi(
            patch,
            vec![
                ("src/old.rs", "fn helper() {}\nfn stays() {}\n"),
                ("src/new.rs", "use crate::common;\n"),
            ],
        )
        .unwrap();
        assert!(
            !files["src/old.rs"].contains("fn helper"),
            "got: {}",
            files["src/old.rs"]
        );
        assert!(
            files["src/old.rs"].contains("fn stays"),
            "got: {}",
            files["src/old.rs"]
        );
        assert!(
            files["src/new.rs"].contains("fn helper"),
            "got: {}",
            files["src/new.rs"]
        );
        assert!(
            files["src/new.rs"].contains("use crate::old_dependency;"),
            "got: {}",
            files["src/new.rs"]
        );
    }

    #[test]
    fn unresolved_use_fails() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,0 @@ rule=paste\n# lang=rust\n# use=Missing\n+ Missing\n";
        let err = apply_multi(patch, vec![("src/lib.rs", "fn a() {}\n")]).unwrap_err();
        assert!(err.contains("RSMPL_CAPTURE_CONFLICT"), "got: {err}");
        assert!(err.contains("Missing"), "got: {err}");
    }

    #[test]
    fn second_hunk_uses_original_lines() {
        // SPEC 29.11: the second semantic hunk keeps its original line number
        // even though the first hunk inserts a larger block before it.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,1 +3,4 @@ rule=wrap_call\n# lang=rust\n# allow=many\n# expr E\n- old_function(E)\n+ new_function_that_spans(\n+     E,\n+ )\n\n@@ -5,1 +5,1 @@ rule=rename_second_function\n# lang=rust\n- fn second_function(...) {\n+ fn renamed_function(...) {\n      ...\n  }\n";
        let src =
            "fn top() {}\nfn old_wrapper() {\n    old_function(1);\n}\nfn second_function() {}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("new_function_that_spans("), "got: {out}");
        assert!(out.contains("fn renamed_function()"), "got: {out}");
    }

    #[test]
    fn use_on_non_insertion_hunk_rejected() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# expr E\n# use=Helper\n- foo(E)\n+ bar(E)\n";
        let err =
            apply_multi(patch, vec![("src/lib.rs", "fn a() {\n    foo(1);\n}\n")]).unwrap_err();
        assert!(err.contains("RSMPL_CAPTURE_CONFLICT"), "got: {err}");
    }

    #[test]
    fn non_preserve_format_rejected_with_cargo_fmt_pointer() {
        // T17: `# format=rustfmt` is rejected, not deferred — the
        // suggestion must name the permanent answer (`cargo fmt`).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# format=rustfmt\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let err = apply_multi(patch, vec![("src/lib.rs", "fn foo() {}\n")]).unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("cargo fmt"), "got: {err}");
    }

    #[test]
    fn scope_file_dummy_range_finds_late_item() {
        // T61: `@@ -1,1 @@` + `# scope=file` renames a fn on lines 5-7
        // with no file-read roundtrip.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn foo() {}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("fn bar()"), "got: {out}");
        assert!(!out.contains("fn foo()"), "got: {out}");
    }

    #[test]
    fn zero_match_unique_elsewhere_hints_scope_file() {
        // T62: tight wrong range (lines 1-1) but the only `foo` is on
        // line 5 — the suggestion must offer the zero-discovery retry.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn foo() {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("no match found"), "got: {err}");
        assert!(err.contains("# scope=file"), "got: {err}");
        assert!(err.contains("@@ -1,1 @@"), "got: {err}");
    }

    #[test]
    fn zero_match_many_elsewhere_hints_widen() {
        // T62: zero in range, several matches elsewhere — suggest widening
        // plus `# allow=many` instead of the single-match retry.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src =
            "fn a() {}\nfn b() {\n    foo(1);\n}\nfn c() {\n    foo(2);\n}\nfn d() {\n    foo(3);\n}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("multiple matches exist"), "got: {err}");
        assert!(err.contains("# allow=many"), "got: {err}");
    }

    #[test]
    fn zero_match_nowhere_keeps_generic_hint() {
        // T62: pattern matches nothing anywhere — no scope=file retry.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn nosuchfn(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "fn a() {}\nfn b() {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("read `src/lib.rs`"), "got: {err}");
        assert!(!err.contains("scope=file"), "got: {err}");
    }

    #[test]
    fn scope_file_whole_file_call_rewrite() {
        // T61: `allow=many` + `scope=file` rewrites calls anywhere.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert_eq!(out.matches("bar(").count(), 2, "got: {out}");
        assert!(!out.contains("foo("), "got: {out}");
    }

    #[test]
    fn zero_match_fn_preview_names_nearest() {
        // T67: wrong-name fn rename names the real fn + line in `reason`,
        // so the retry often needs no file read.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn nosuchfn(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "fn a() {}\nfn b() {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("no match found"), "got: {err}");
        assert!(err.contains("nearest"), "got: {err}");
        assert!(err.contains("line 1"), "got: {err}");
    }

    #[test]
    fn zero_match_block_level_preview_names_local_item() {
        // T141: a typo'd block-level rename previews the same-kind local,
        // not the enclosing fn — the retry needs no file read.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=r\n# lang=rust\n- struct Lcoal\n+ struct Fixed\n";
        let src = "fn outer() {\n    struct Local;\n    let _ = Local;\n}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("nearest struct Local at line 2"), "got: {err}");
    }

    #[test]
    fn zero_match_typo_prefers_similar_name_over_nearer() {
        // T142: a typo'd rename previews the most similar same-kind
        // header, not the nearest one — the retry needs no file read.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- struct Betta\n+ struct Gamma\n";
        let src = "struct Alpha;\nstruct Beta;\nfn outer() {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("nearest struct Beta at line 2"), "got: {err}");
    }

    #[test]
    fn zero_match_similarity_tie_breaks_on_distance() {
        // T142: equal edit distance falls back to line distance — both
        // `Zetta` and `Beta` are 1 edit from `Betta`, so the nearer line
        // (old_start=1) wins.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- struct Betta\n+ struct Gamma\n";
        let src = "struct Zetta;\nstruct Beta;\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("nearest struct Zetta at line 1"), "got: {err}");
    }

    #[test]
    fn zero_match_call_preview_shows_source_line() {
        // T67 fallback tier: a call rewrite with no same-kind header walk
        // still previews the nearest non-empty source line.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# expr E\n- nosuchcall(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1);\n}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("nearest source at line"), "got: {err}");
    }

    #[test]
    fn zero_match_impl_preview_names_trait_and_self_ty() {
        // T19b: an impl rename with the wrong name previews the real
        // header with both sides, so the retry needs no file read.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,1 @@ rule=r\n# lang=rust\n- impl Nosuch\n+ impl New\n";
        let src = "trait Old {}\nstruct Foo;\nimpl Old for Foo {}\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        assert!(err.contains("nearest impl Old for Foo"), "got: {err}");
    }

    #[test]
    fn report_carries_per_match_spans() {
        // T27: `spans` mirrors `matches` with original-source 1-based
        // inclusive line ranges — agents verify without re-reading.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,6 +1,6 @@ rule=r\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let src = "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\n";
        let (files, report) = apply_multi_report(patch, vec![("src/lib.rs", src)]).unwrap();
        assert_eq!(report.rules.len(), 1);
        let r = &report.rules[0];
        assert_eq!(r.matches, 2);
        assert_eq!(r.spans, vec![(2, 2), (5, 5)], "spans: {:?}", r.spans);
        assert!(files["src/lib.rs"].contains("bar(1)"));
        assert!(files["src/lib.rs"].contains("bar(2)"));
    }

    #[test]
    fn report_spans_cover_cut_and_paste() {
        // T27: cut hunks span the removed item lines; paste hunks span the
        // anchor line ([L, L] for zero-width insertions).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -4,1 +4,1 @@ rule=paste_h\n# lang=rust\n# use=H\n+ H\n\n@@ -6,3 +6,0 @@ rule=cut_h\n# lang=rust\n# item I\n# bind=H\n- I\n";
        let src = "fn top() {}\nfn mid() {}\nfn keep() {}\nfn other() {}\nfn tail() {}\nfn helper(x: i32) -> i32 {\n    x + 1\n}\n";
        let (_, report) = apply_multi_report(patch, vec![("src/lib.rs", src)]).unwrap();
        assert_eq!(report.rules.len(), 2);
        // Document order: paste first, cut second (T45). The paste anchor
        // `-4,1` inserts after line 4 (SPEC 10.5), i.e. at line 5.
        assert_eq!(report.rules[0].rule, "paste_h");
        assert_eq!(report.rules[0].spans, vec![(5, 5)]);
        assert_eq!(report.rules[1].rule, "cut_h");
        assert_eq!(report.rules[1].spans, vec![(6, 8)]);
        for r in &report.rules {
            assert_eq!(r.spans.len(), r.matches, "rule {}", r.rule);
        }
    }

    #[test]
    fn preview_truncates_long_lines() {
        // T67 caps: a 300-char call line previews at most 200 chars +
        // ellipsis (fallback tier keeps the label fixed, isolating the
        // excerpt cap; pathological idents are bound by `cap_preview`).
        let long_arg = "a".repeat(300);
        let src = format!("fn a() {{\n    foo({long_arg});\n}}\n");
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- nosuchcall(E)\n+ bar(E)\n";
        let err = apply_multi(patch, vec![("src/lib.rs", src.as_str())]).unwrap_err();
        assert!(err.contains("nearest source at line 2"), "got: {err}");
        let excerpt = err.split("nearest ").nth(1).expect("preview");
        assert!(excerpt.chars().count() <= 330, "got: {err}");
        assert!(excerpt.contains('…'), "got: {err}");
    }

    #[test]
    fn ordinary_only_markdown_applies_without_rust_parse() {
        // T137: ordinary-only sections are language-agnostic — no Rust
        // parse of the original or the rewritten file.
        let patch = "diff --git a/SPEC.md b/SPEC.md\n--- a/SPEC.md\n+++ b/SPEC.md\n@@ -1,2 +1,2 @@\n # Title\n-old line\n+new line\n";
        let src = "# Title\nold line\n";
        let files = apply_multi(patch, vec![("SPEC.md", src)]).unwrap();
        assert_eq!(files["SPEC.md"], "# Title\nnew line\n");
    }

    #[test]
    fn mixed_rust_and_markdown_sections() {
        // T137: one Rust section with an RSmPL hunk + one markdown
        // ordinary-only section apply together.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n\ndiff --git a/NOTES.md b/NOTES.md\n--- a/NOTES.md\n+++ b/NOTES.md\n@@ -1,1 +1,1 @@\n-old note\n+new note\n";
        let files = apply_multi(
            patch,
            vec![("src/lib.rs", "fn foo() {}\n"), ("NOTES.md", "old note\n")],
        )
        .unwrap();
        assert!(
            files["src/lib.rs"].contains("fn bar()"),
            "got: {}",
            files["src/lib.rs"]
        );
        assert_eq!(files["NOTES.md"], "new note\n");
    }

    #[test]
    fn ordinary_only_invalid_rust_applies() {
        // T137 negative shape: the skip is per-file semantic presence,
        // not extension sniffing — invalid Rust with only ordinary hunks
        // still applies.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-this is not rust {{{\n+fixed line\n";
        let files = apply_multi(patch, vec![("src/lib.rs", "this is not rust {{{\n")]).unwrap();
        assert_eq!(files["src/lib.rs"], "fixed line\n");
    }

    #[test]
    fn semantic_hunk_on_non_rust_still_parse_conflicts() {
        // T137 negative control: an RSmPL hunk on markdown still fails
        // with RSMPL_PARSE_CONFLICT — the skip covers ordinary-only
        // files, not a blanket removal of the Rust gate.
        let patch = "diff --git a/NOTES.md b/NOTES.md\n--- a/NOTES.md\n+++ b/NOTES.md\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let err = apply_multi(patch, vec![("NOTES.md", "# notes\n")]).unwrap_err();
        assert!(err.contains("RSMPL_PARSE_CONFLICT"), "got: {err}");
        assert!(err.contains("does not parse as Rust"), "got: {err}");
    }

    #[test]
    fn same_anchor_multi_paste_keeps_document_order_with_blanks() {
        // T249: three pastes at one anchor land in document order
        // (first hunk topmost), joined with exactly one blank line.
        // Mirrors the R2 stdin-trio move: pastes [one, two, three] must
        // read [one, two, three], not reversed, with blanks restored.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=paste_one\n# use=One\n+ One\n\n@@ -1,0 @@ rule=paste_two\n# use=Two\n+ Two\n\n@@ -1,0 @@ rule=paste_three\n# use=Three\n+ Three\n\n@@ -2,1 @@ rule=cut_one\n# item I\n# bind=One\n- I\n\n@@ -4,1 @@ rule=cut_two\n# item I\n# bind=Two\n- I\n\n@@ -6,1 @@ rule=cut_three\n# item I\n# bind=Three\n- I\n";
        let src = "fn top() {}\nfn one() {}\nfn mid() {}\nfn two() {}\nfn keep() {}\nfn three() {}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        let p1 = out.find("fn one() {}").expect("one");
        let p2 = out.find("fn two() {}").expect("two");
        let p3 = out.find("fn three() {}").expect("three");
        assert!(p1 < p2 && p2 < p3, "document order, got:\n{out}");
        assert!(
            out.contains("fn one() {}\n\nfn two() {}\n\nfn three() {}"),
            "blank-joined, got:\n{out}"
        );
        assert_eq!(out.matches("fn one() {}").count(), 1);
        assert_eq!(out.matches("fn two() {}").count(), 1);
        assert_eq!(out.matches("fn three() {}").count(), 1);
    }

    #[test]
    fn single_paste_gains_no_extra_blanks() {
        // T249: the blank-join fires only for N>1 pastes at one anchor.
        // A lone paste keeps the historical `trim_end + "\n"` bytes.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=paste_h\n# lang=rust\n# use=H\n+ H\n\n@@ -3,1 @@ rule=cut_h\n# lang=rust\n# item I\n# bind=H\n- I\n";
        let src = "fn top() {}\nfn mid() {}\nfn helper() {}\n";
        let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert_eq!(out, "fn top() {}\nfn helper() {}\nfn mid() {}\n");
    }
}
