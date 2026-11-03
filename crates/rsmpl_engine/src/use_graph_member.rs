//! WI-9a: `# follow=uses` for struct-field and enum-variant renames
//! (SPEC 14.8).
//!
//! Sibling of `use_graph` (D5: new code lands in new modules). The scalar
//! item/fn collector claims *every* `old` path segment, which is correct
//! for item renames and wrong for members: a variant `Old` must only
//! claim `E::Old`-spelled paths, and a field `old` must only claim `.old`
//! accesses plus `S`-anchored literal/pattern labels. Field accesses and
//! method idents are different namespaces and never cross-claim.
//!
//! WI-14 anchored posture (SPEC 2.6, D3, D8): a second same-named member
//! definition never vetoes the run — the definition hunk always applies.
//! Each candidate site is classified by its strongest anchor: literal-path
//! (`S { … }` labels, `E::Old` tails), signature (`fn f(a: A)` params),
//! annotation (`let x: A`, `let x = S { … }`, `self` in `impl S`, all
//! flow-ordered with rebinding). Proven sites rewrite; everything else
//! same-name is `follow_skipped_details` evidence (`shadowed` when another
//! container defines the name, else `off-target`) — never a run failure.
//! Bare `Old` needs a `use …::E::*` glob anchor, else it is `second-hop`
//! evidence. Truly opaque sites (macro interiors, `qself` paths) still
//! veto without `# fallback=syntax` and skip with it (scalar parity).
//! Method idents and tuple `.0` accesses never match and never veto.

use std::collections::HashMap;

use syn::visit::{self, Visit};

use crate::edits::TextEdit;
use crate::follow_reasons::{reason as skip_reason, FollowSkip};
use crate::lines::{FileState, LineIndex};
use crate::matcher_item::span_bytes;
use crate::use_graph::FollowVeto;
use crate::use_graph_anchor::field_root;

/// Which member shape is being followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberKind {
    Field,
    Variant,
}

/// One follow edit: byte range plus replacement text. Shorthand labels
/// (`S { old }` → `S { new: old }`) carry a wider replacement than the
/// ident span they cover.
#[derive(Debug, Clone)]
pub(crate) struct MemberEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

/// Workspace-wide member-follow resolution (unique-definition fast path).
#[derive(Debug)]
pub(crate) struct MemberWorkspaceOutcome {
    /// Path → ident edits, sorted by path; entries exist only for files
    /// with ≥1 edit.
    pub per_file: Vec<(String, Vec<MemberEdit>)>,
    /// WI-2 structured skips for sites left untouched under fallback.
    pub skipped: Vec<FollowSkip>,
}

/// Resolve member use sites across every loaded `.rs` file.
///
/// `renamed_file` + `renamed` carry the definition hunk's ident-span
/// edit(s); a same-named member definition overlapping one of them is
/// the renamed definition itself.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_member_workspace(
    files: &[(String, &syn::File, &str, &LineIndex)],
    kind: MemberKind,
    container: &str,
    old: &str,
    new: &str,
    renamed_file: &str,
    renamed: &[(usize, usize)],
    fallback: bool,
) -> Result<MemberWorkspaceOutcome, FollowVeto> {
    // Census member definitions workspace-wide (fail loud on unmappable
    // spans, never guess).
    struct DefHit {
        path: String,
        a: usize,
        b: usize,
    }
    let mut def_hits: Vec<DefHit> = Vec::new();
    for (path, parsed, original, index) in files {
        let mut collector = MemberCollector::new(kind, container, old);
        collector.visit_file(parsed);
        for (span, _def_container) in &collector.defs {
            match span_bytes(original, index, *span) {
                Some((a, b)) => def_hits.push(DefHit {
                    path: path.clone(),
                    a,
                    b,
                }),
                None => {
                    let line = span.start().line as u32;
                    return Err(FollowVeto {
                        reason: format!(
                            "definition of `{container}::{old}` at {path}:{line} maps outside the file"
                        ),
                    });
                }
            }
        }
    }
    let renamed_hit_in = |path: &str, a: usize, b: usize| {
        path == renamed_file && renamed.iter().any(|(s, e)| a < *e && *s < b)
    };
    let shadow: Vec<&DefHit> = def_hits
        .iter()
        .filter(|d| !renamed_hit_in(&d.path, d.a, d.b))
        .collect();
    let has_shadow = !shadow.is_empty();

    // Map + verify use spans per file (WI-14 anchored).
    let ws_parsed: Vec<&syn::File> = files.iter().map(|(_, p, _, _)| *p).collect();
    let ws_deref = crate::use_graph_anchor::collect_workspace_deref_targets(&ws_parsed);
    let mut per_file: Vec<(String, Vec<MemberEdit>)> = Vec::new();
    let mut opaque: Vec<(String, u32, String)> = Vec::new();
    let mut skipped: Vec<FollowSkip> = Vec::new();
    for (path, parsed, original, index) in files {
        let mut collector = MemberCollector::new(kind, container, old);
        collector.visit_file(parsed);
        for (line, ctx) in &collector.opaque {
            opaque.push((path.clone(), *line, ctx.clone()));
        }
        let anchors =
            crate::use_graph_anchor::AnchorIndex::for_file_with_workspace(parsed, &ws_deref);
        // Bare-`Old` variant mentions need a `use …::E::*` glob anchor
        // (WI-14): proven with the anchor, `second-hop` evidence without.
        if kind == MemberKind::Variant {
            for span in &collector.bare {
                let line = span.start().line as u32;
                if anchors.has_glob_anchor(container) {
                    let Some((a, b)) = span_bytes(original, index, *span) else {
                        return Err(FollowVeto {
                            reason: format!(
                                "use of `{old}` at {path}:{line} maps outside the file"
                            ),
                        });
                    };
                    if renamed_hit_in(path, a, b) {
                        continue;
                    }
                    // Bare edits carry the plain replacement (merged with
                    // the label/path edits below).
                    match per_file.iter_mut().find(|(p, _)| p == path) {
                        Some((_, existing)) => existing.push(MemberEdit {
                            start: a,
                            end: b,
                            replacement: new.to_string(),
                        }),
                        None => per_file.push((
                            (*path).clone(),
                            vec![MemberEdit {
                                start: a,
                                end: b,
                                replacement: new.to_string(),
                            }],
                        )),
                    };
                } else {
                    skipped.push(FollowSkip::new(
                        format!(
                            "{path}:{line}: bare `{old}` spelling left untouched (needs `use {container}::*` to prove it names `{container}::{old}`)"
                        ),
                        path,
                        line,
                        skip_reason::SECOND_HOP,
                    ));
                }
            }
        }
        let mut edits: Vec<MemberEdit> = Vec::new();
        for use_site in &collector.uses {
            let line = use_site.span.start().line as u32;
            let Some((a, b)) = span_bytes(original, index, use_site.span) else {
                return Err(FollowVeto {
                    reason: format!("use of `{old}` at {path}:{line} maps outside the file"),
                });
            };
            if original.get(a..b) != Some(use_site.text.as_str()) {
                return Err(FollowVeto {
                    reason: format!("use of `{old}` at {path}:{line} is not the expected text"),
                });
            }
            if renamed_hit_in(path, a, b) {
                continue;
            }
            // WI-14 anchor gate for field accesses (literal-path labels
            // and variant tails carry `receiver: None` and stay proven).
            if kind == MemberKind::Field {
                if let Some(root) = use_site.receiver.as_deref() {
                    let proven = if root == "self" {
                        anchors.self_type_at(line).is_some_and(|s| s == container)
                    } else {
                        anchors
                            .anchor_of(root, line)
                            .is_some_and(|s| s == container)
                    };
                    if !proven {
                        let code = if has_shadow {
                            skip_reason::SHADOWED
                        } else {
                            skip_reason::OFF_TARGET
                        };
                        skipped.push(FollowSkip::new(
                            format!(
                                "{path}:{line}: unanchored `{old}` access left untouched (receiver `{root}` does not prove `{container}`)"
                            ),
                            path,
                            line,
                            code,
                        ));
                        continue;
                    }
                }
            }
            // Shorthand labels keep the local binding: `S { old }`
            // → `S { new: old }`.
            let replacement = if use_site.shorthand {
                format!("{new}: {}", use_site.text)
            } else {
                new.to_string()
            };
            edits.push(MemberEdit {
                start: a,
                end: b,
                replacement,
            });
        }
        edits.sort_by_key(|e| (e.start, e.end));
        edits.dedup_by(|a, b| a.start == b.start && a.end == b.end);
        if !edits.is_empty() {
            // Merge with any bare-`Old` edits already recorded above.
            match per_file.iter_mut().find(|(p, _)| p == path) {
                Some((_, existing)) => {
                    existing.extend(edits);
                    existing.sort_by_key(|e| (e.start, e.end));
                    existing.dedup_by(|a, b| a.start == b.start && a.end == b.end);
                }
                None => per_file.push((path.clone(), edits)),
            }
        }
    }
    per_file.sort_by(|a, b| a.0.cmp(&b.0));

    // WI-14: shadow definitions never veto (D8). The definition hunk
    // always applies; unproven sites above are already `shadowed` /
    // `off-target` evidence. `use_sites` counts only rewritten sites.
    let _use_sites: usize = per_file.iter().map(|(_, s)| s.len()).sum();
    opaque.sort();
    opaque.dedup();
    if !opaque.is_empty() {
        if !fallback {
            let (path, line, ctx) = &opaque[0];
            return Err(FollowVeto {
                reason: format!("use of `{old}` inside {ctx} at {path}:{line}: cannot prove it names `{container}::{old}`"),
            });
        }
        for (path, line, ctx) in &opaque {
            // Bare-`Old` spellings are a possible second-hop import; macro
            // interiors stay macro-hidden. Both are opaque either way.
            let code = if ctx.contains("bare `Old`") {
                skip_reason::SECOND_HOP
            } else {
                skip_reason::MACRO_HIDDEN
            };
            skipped.push(FollowSkip::new(
                format!("{path}:{line}: use of `{old}` inside {ctx} left untouched (opaque)"),
                path,
                *line,
                code,
            ));
        }
        if has_shadow {
            // Shadow definitions are evidence, never a veto (D8): the
            // per-site skips above already name them.
        }
    }
    Ok(MemberWorkspaceOutcome { per_file, skipped })
}

/// WI-9a: orphan evidence for definition-only member renames (SPEC
/// 14.8). Mirrors the scalar orphan block: the definition hunk's own
/// ident-span edits plus every loaded `.rs` file feed the member
/// collector. (Relocated from `use_graph_local.rs`, its park after the
/// `plan.rs` gate move; behaviour-preserving.)
#[allow(clippy::too_many_arguments)]
pub(crate) fn member_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    kind: MemberKind,
    container: &str,
    old: &str,
) -> Vec<crate::OrphanReport> {
    let renamed: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> = Vec::new();
    let mut paths: Vec<&String> = states.keys().collect();
    paths.sort();
    for p in paths {
        if let Some(st) = states.get(p) {
            if let Some(parsed) = st.parsed.as_ref() {
                refs.push((p.clone(), parsed, st.original.as_str(), &st.index));
            }
        }
    }
    collect_member_orphans(&refs, kind, container, old, path, &renamed)
}

/// Report-only orphan collection for definition-only member renames
/// (WI-7c mirror for members, SPEC 14.8). Same shapes the follow step
/// would rewrite, plus bare-`Old` spellings (variants) and macro
/// interiors — never vetoing, never failing loud. Unparseable files are
/// excluded (the follow tripwire owns them).
pub(crate) fn collect_member_orphans(
    files: &[(String, &syn::File, &str, &LineIndex)],
    kind: MemberKind,
    container: &str,
    old: &str,
    renamed_file: &str,
    renamed: &[(usize, usize)],
) -> Vec<crate::OrphanReport> {
    let renamed_hit_in = |path: &str, a: usize, b: usize| {
        path == renamed_file && renamed.iter().any(|(s, e)| a < *e && *s < b)
    };
    let mut out: Vec<crate::OrphanReport> = Vec::new();
    // WI-14: orphan reasons mirror the follow classifier (one classifier,
    // two consumers). Shadow = another container defines the name.
    let has_shadow = files.iter().any(|(_, parsed, _, _)| {
        let mut census = MemberCollector::new(kind, container, old);
        census.visit_file(parsed);
        census.defs.iter().any(|(_, c)| c != container)
    });
    let ws_parsed_o: Vec<&syn::File> = files.iter().map(|(_, p, _, _)| *p).collect();
    let ws_deref_o = crate::use_graph_anchor::collect_workspace_deref_targets(&ws_parsed_o);
    for (path, parsed, original, index) in files {
        let mut collector = MemberCollector::new(kind, container, old);
        collector.visit_file(parsed);
        let anchors =
            crate::use_graph_anchor::AnchorIndex::for_file_with_workspace(parsed, &ws_deref_o);
        // WI-14 anchored orphans: proven accesses are `off-target`;
        // unproven are `shadowed` (sibling defines the name) or
        // `off-target`; bare-without-glob is `second-hop`.
        let mut lines: Vec<(u32, &'static str)> = Vec::new();
        for use_site in &collector.uses {
            let line = use_site.span.start().line as u32;
            let Some((a, b)) = span_bytes(original, index, use_site.span) else {
                continue;
            };
            if original.get(a..b) != Some(use_site.text.as_str()) {
                continue;
            }
            if renamed_hit_in(path, a, b) {
                continue;
            }
            if kind == MemberKind::Field {
                if let Some(root) = use_site.receiver.as_deref() {
                    let proven = if root == "self" {
                        anchors.self_type_at(line).is_some_and(|s| s == container)
                    } else {
                        anchors
                            .anchor_of(root, line)
                            .is_some_and(|s| s == container)
                    };
                    if proven {
                        lines.push((line, "off-target"));
                    } else if has_shadow {
                        lines.push((line, "shadowed"));
                    } else {
                        lines.push((line, "off-target"));
                    }
                    continue;
                }
            }
            lines.push((line, "off-target"));
        }
        if kind == MemberKind::Variant {
            for span in &collector.bare {
                let line = span.start().line as u32;
                if anchors.has_glob_anchor(container) {
                    lines.push((line, "off-target"));
                } else {
                    lines.push((line, "second-hop"));
                }
            }
        }
        for (line, _) in &collector.opaque {
            lines.push((*line, "macro-hidden"));
        }
        lines.sort_unstable();
        lines.dedup();
        for (line, reason) in lines {
            out.push(crate::OrphanReport {
                file: (*path).clone(),
                line,
                text: source_line_text(original, index, line),
                reason: reason.to_string(),
            });
        }
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}

/// One trimmed source line (never fails: OOB lines yield empty text).
fn source_line_text(original: &str, index: &LineIndex, line: u32) -> String {
    let Some(start) = index.line_start(line) else {
        return String::new();
    };
    let end = index.line_start(line + 1).unwrap_or(original.len());
    original
        .get(start..end)
        .unwrap_or("")
        .trim_end_matches(['\n', '\r'])
        .trim()
        .to_string()
}

/// One collected use site: the ident span plus whether it is a
/// shorthand label (`S { old }`, expanded to `S { new: old }`).
struct MemberUse {
    span: proc_macro2::Span,
    text: String,
    shorthand: bool,
    /// Field-access receiver root (`Some("a")` in `a.old`,
    /// `Some("self")`, `Some("")` for unprovable bases like calls);
    /// `None` for literal-path sites (struct labels, variant tails)
    /// that need no anchor.
    receiver: Option<String>,
}

struct MemberCollector<'a> {
    kind: MemberKind,
    container: &'a str,
    old: &'a str,
    /// (member-ident span, container name) definitions named `old`.
    defs: Vec<(proc_macro2::Span, String)>,
    /// Provable use sites (edits).
    uses: Vec<MemberUse>,
    /// (line, context) opaque mentions.
    opaque: Vec<(u32, String)>,
    /// Bare single-segment `Old` paths (variant only: possibly
    /// import-spelled; recorded separately and reported as opaque).
    bare: Vec<proc_macro2::Span>,
    /// Current `impl` self-ty last segments (for `Self`-spelled uses).
    impl_stack: Vec<String>,
}

impl<'a> MemberCollector<'a> {
    fn new(kind: MemberKind, container: &'a str, old: &'a str) -> Self {
        Self {
            kind,
            container,
            old,
            defs: Vec::new(),
            uses: Vec::new(),
            opaque: Vec::new(),
            bare: Vec::new(),
            impl_stack: Vec::new(),
        }
    }

    fn is_old(&self, ident: &syn::Ident) -> bool {
        ident == self.old
    }

    /// Last path segment, if any.
    fn last_seg(path: &syn::Path) -> Option<String> {
        path.segments.last().map(|s| s.ident.to_string())
    }

    /// Does a struct-literal/pattern path name the container? Last
    /// segment equality, plus `Self` inside an `impl` of the container.
    fn names_container(&self, path: &syn::Path) -> bool {
        if Self::last_seg(path).is_some_and(|s| s == self.container) {
            return true;
        }
        if path.segments.len() == 1
            && path.segments.first().is_some_and(|s| s.ident == "Self")
            && self.impl_stack.last().is_some_and(|s| s == self.container)
        {
            return true;
        }
        false
    }

    /// Does an expression/pattern path name the variant? `E::Old` with
    /// any leading prefix, plus `Self::Old` inside an `impl` of `E`.
    fn names_variant(&self, path: &syn::Path) -> bool {
        let n = path.segments.len();
        if n < 2 {
            return false;
        }
        let Some(last) = path.segments.last() else {
            return false;
        };
        if last.ident != self.old {
            return false;
        }
        let prev = &path.segments[n - 2].ident;
        if prev == self.container {
            return true;
        }
        prev == "Self" && self.impl_stack.last().is_some_and(|s| s == self.container)
    }

    fn push_use(&mut self, span: proc_macro2::Span, text: String, shorthand: bool) {
        self.uses.push(MemberUse {
            span,
            text,
            shorthand,
            receiver: None,
        });
    }

    fn push_plain_use(&mut self, span: proc_macro2::Span) {
        let text = self.old.to_string();
        self.push_use(span, text, false);
    }
}

/// Per-ident macro lines mentioning `old` (WI-2b ident-line parity:
/// multi-line macros report the ident line, not the macro start line).
/// Mirrors orphan/assoc helpers: whole-ident match only, recurse groups.
fn macro_ident_lines(tokens: &proc_macro2::TokenStream, old: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for t in tokens.clone() {
        match t {
            proc_macro2::TokenTree::Ident(id) if id == old => {
                out.push(id.span().start().line as u32);
            }
            proc_macro2::TokenTree::Group(g) => {
                out.extend(macro_ident_lines(&g.stream(), old));
            }
            _ => {}
        }
    }
    out
}

/// Last segment ident of a plain type path (`S`, `S<T>`, `m::S`).
fn self_ty_last(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(tp) = ty {
        if tp.qself.is_none() {
            return tp.path.segments.last().map(|s| s.ident.to_string());
        }
    }
    None
}

impl<'ast> Visit<'ast> for MemberCollector<'_> {
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let pushed = if let Some(name) = self_ty_last(&node.self_ty) {
            self.impl_stack.push(name);
            true
        } else {
            false
        };
        visit::visit_item_impl(self, node);
        if pushed {
            self.impl_stack.pop();
        }
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        // Census: every named field `old` in every struct.
        if let syn::Fields::Named(fields) = &node.fields {
            for f in &fields.named {
                if f.ident.as_ref().is_some_and(|i| self.is_old(i)) {
                    let ident = f.ident.as_ref().expect("named field");
                    self.defs.push((ident.span(), node.ident.to_string()));
                }
            }
        }
        visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        // Census: every variant `old` in every enum.
        for v in &node.variants {
            if self.is_old(&v.ident) {
                self.defs.push((v.ident.span(), node.ident.to_string()));
            }
        }
        visit::visit_item_enum(self, node);
    }

    fn visit_expr_field(&mut self, node: &'ast syn::ExprField) {
        // `.old` accesses (field renames only; tuple `.0` never matches).
        if self.kind == MemberKind::Field {
            if let syn::Member::Named(ident) = &node.member {
                if self.is_old(ident) {
                    // WI-14: record the receiver root for anchor
                    // classification. `Some("")` marks an unprovable
                    // base (call/literal result — never claims);
                    // literal-path labels keep `None` (proven).
                    let receiver = Some(field_root(&node.base).unwrap_or_default());
                    self.uses.push(MemberUse {
                        span: ident.span(),
                        text: self.old.to_string(),
                        shorthand: false,
                        receiver,
                    });
                }
            }
        }
        visit::visit_expr_field(self, node);
    }

    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        // `S { old }` / `S { old: v }` labels (field renames only).
        if self.kind == MemberKind::Field && self.names_container(&node.path) {
            for field in &node.fields {
                if let syn::Member::Named(ident) = &field.member {
                    if self.is_old(ident) {
                        // Shorthand keeps the local binding: `S { old }`
                        // → `S { new: old }`.
                        let shorthand = field.colon_token.is_none();
                        self.push_use(ident.span(), self.old.to_string(), shorthand);
                    }
                }
            }
        }
        visit::visit_expr_struct(self, node);
    }

    fn visit_pat_struct(&mut self, node: &'ast syn::PatStruct) {
        // `S { old }` / `S { old: p }` pattern labels.
        if self.kind == MemberKind::Field && self.names_container(&node.path) {
            for field in &node.fields {
                if let syn::Member::Named(ident) = &field.member {
                    if self.is_old(ident) {
                        let shorthand = field.colon_token.is_none();
                        self.push_use(ident.span(), self.old.to_string(), shorthand);
                    }
                }
            }
        }
        visit::visit_pat_struct(self, node);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        // `E::Old` paths (variant renames only).
        if self.kind == MemberKind::Variant {
            if node.segments.len() == 1
                && node.segments.first().is_some_and(|s| s.ident == self.old)
            {
                // Possibly import-spelled: opaque, never an edit.
                self.bare
                    .push(node.segments.first().expect("one segment").ident.span());
            } else if self.names_variant(node) {
                let last = node.segments.last().expect("variant path");
                self.push_plain_use(last.ident.span());
            }
        }
        visit::visit_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        // `qself` (`<T as E>::Old`) is opaque, mirroring scalar.
        // One entry per matching ident line (WI-2b ident-line parity —
        // not the path start line). Single-line paths are unchanged.
        if self.kind == MemberKind::Variant && node.qself.is_some() {
            let mut hit = false;
            for seg in &node.path.segments {
                if seg.ident == self.old {
                    self.opaque.push((
                        seg.ident.span().start().line as u32,
                        "a qself path".to_string(),
                    ));
                    hit = true;
                }
            }
            if hit {
                return;
            }
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if self.kind == MemberKind::Variant && node.qself.is_some() {
            let mut hit = false;
            for seg in &node.path.segments {
                if seg.ident == self.old {
                    self.opaque.push((
                        seg.ident.span().start().line as u32,
                        "a qself path".to_string(),
                    ));
                    hit = true;
                }
            }
            if hit {
                return;
            }
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        // `use …::E::Old` import segments (variant renames only): the
        // `Old` segment rewrites; globs and bare-`Old` imports stay
        // opaque-by-omission (bare uses never edit).
        if self.kind == MemberKind::Variant {
            fn walk(tree: &syn::UseTree, this: &mut MemberCollector<'_>, prefix: &mut Vec<String>) {
                match tree {
                    syn::UseTree::Path(p) => {
                        prefix.push(p.ident.to_string());
                        walk(&p.tree, this, prefix);
                        prefix.pop();
                    }
                    syn::UseTree::Name(n) => {
                        if n.ident == this.old && prefix.last().is_some_and(|s| s == this.container)
                        {
                            this.push_plain_use(n.ident.span());
                        }
                    }
                    syn::UseTree::Rename(r) => {
                        if r.ident == this.old && prefix.last().is_some_and(|s| s == this.container)
                        {
                            this.push_plain_use(r.ident.span());
                        }
                    }
                    syn::UseTree::Group(g) => {
                        for member in &g.items {
                            walk(member, this, prefix);
                        }
                    }
                    syn::UseTree::Glob(_) => {}
                }
            }
            let mut prefix = Vec::new();
            walk(&node.tree, self, &mut prefix);
        }
        visit::visit_item_use(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // No recursion: macro interiors are opaque, never claimed.
        // One entry per ident line (WI-2b ident-line parity — not the
        // macro start line). Single-line macros are unchanged.
        for line in macro_ident_lines(&node.tokens, self.old) {
            self.opaque.push((line, "a macro invocation".to_string()));
        }
    }

    fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
        // Attribute paths and values cannot spell anchored member uses
        // in valid Rust, but a same-spelling ident inside one is
        // evidence, never a rewrite (scalar parity): record it opaque
        // at its own ident line instead of staying silent.
        for seg in node.path().segments.iter() {
            if seg.ident == self.old {
                self.opaque.push((
                    seg.ident.span().start().line as u32,
                    "an attribute".to_string(),
                ));
            }
        }
        match &node.meta {
            syn::Meta::NameValue(nv) => {
                struct ValuePaths<'o> {
                    old: &'o str,
                    opaque: Vec<u32>,
                }
                impl<'ast> Visit<'ast> for ValuePaths<'_> {
                    fn visit_path(&mut self, node: &'ast syn::Path) {
                        for seg in &node.segments {
                            if seg.ident == self.old {
                                self.opaque.push(seg.ident.span().start().line as u32);
                            }
                        }
                        visit::visit_path(self, node);
                    }
                    fn visit_macro(&mut self, node: &'ast syn::Macro) {
                        for line in macro_ident_lines(&node.tokens, self.old) {
                            self.opaque.push(line);
                        }
                    }
                }
                let mut values = ValuePaths {
                    old: self.old,
                    opaque: Vec::new(),
                };
                values.visit_expr(&nv.value);
                for line in values.opaque {
                    self.opaque.push((line, "an attribute".to_string()));
                }
            }
            syn::Meta::List(list) => {
                for line in macro_ident_lines(&list.tokens, self.old) {
                    self.opaque.push((line, "an attribute".to_string()));
                }
            }
            syn::Meta::Path(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident_lines_for_tokens(src: &str, old: &str) -> Vec<u32> {
        let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
        struct Grab(Vec<proc_macro2::TokenStream>);
        impl<'ast> syn::visit::Visit<'ast> for Grab {
            fn visit_macro(&mut self, node: &'ast syn::Macro) {
                self.0.push(node.tokens.clone());
                syn::visit::visit_macro(self, node);
            }
        }
        use syn::visit::Visit;
        let mut g = Grab(Vec::new());
        g.visit_file(&parsed);
        assert_eq!(g.0.len(), 1, "one macro expected");
        macro_ident_lines(&g.0[0], old)
    }

    #[test]
    fn multiline_macro_reports_ident_line() {
        let src = "fn f() {\n    println!(\n        \"{}\",\n        Item\n    );\n}\n";
        // `Item` sits on line 4; the macro starts on line 2.
        assert_eq!(ident_lines_for_tokens(src, "Item"), vec![4]);
    }

    #[test]
    fn single_line_macro_reports_same_line() {
        let src = "fn f() { println!(\"{}\", Item); }\n";
        assert_eq!(ident_lines_for_tokens(src, "Item"), vec![1]);
    }

    fn opaque_lines_for(src: &str, container: &str, old: &str) -> Vec<u32> {
        let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
        let mut c = MemberCollector::new(MemberKind::Variant, container, old);
        use syn::visit::Visit;
        c.visit_file(&parsed);
        let mut lines: Vec<u32> = c.opaque.into_iter().map(|(l, _)| l).collect();
        lines.sort_unstable();
        lines
    }

    #[test]
    fn multiline_qself_reports_ident_line() {
        // `Old` sits on line 3; the `<...>::a::` starts on line 2.
        let src = "enum E { Old }\nfn f(x: <T as Tr>::a::\n    Old) {}\n";
        assert_eq!(opaque_lines_for(src, "E", "Old"), vec![3]);
    }

    #[test]
    fn attribute_path_value_and_list_form_are_opaque() {
        // Member attributes were fully silent before the parity fix;
        // now path + value + list forms all surface as opaque.
        let src = "struct S { old: u32 }\n#[old]\nfn f() {}\n";
        assert_eq!(opaque_lines_for(src, "S", "old"), vec![2]);
        let src = "struct S { old: u32 }\n#[doc = old]\nfn f() {}\n";
        assert_eq!(opaque_lines_for(src, "S", "old"), vec![2]);
        let src = "struct S { old: u32 }\n#[foo(old)]\nfn f() {}\n";
        assert_eq!(opaque_lines_for(src, "S", "old"), vec![2]);
        // String literals are not idents: silent.
        let src = "struct S { old: u32 }\n#[doc = \"old\"]\nfn f() {}\n";
        assert_eq!(opaque_lines_for(src, "S", "old"), Vec::<u32>::new());
    }
}
