//! WI-27: `# follow=uses` for associated-item (`type`/`const`) renames
//! (SPEC 14.8).
//!
//! Sibling of `use_graph` (D5: neither `use_graph.rs` nor
//! `matcher_assoc.rs` grows). The scalar item collector claims *every*
//! `old` path segment, which is wrong for assoc items: `Item` must only
//! be claimed when its container proves it names the renamed trait's
//! item — `Self::Old` inside a domain impl block, `S::Old`/`Tr::Old`
//! naming the domain self type / trait, or `<S as Tr>::Old` naming the
//! trait explicitly. Anything else same-named is a different item.
//!
//! Precision-first (SPEC 2.6, D3) with the WI-13 per-block shadow rule:
//! a second trait (or an inherent impl, for `const`) defining the same
//! assoc name never vetoes the run — sites enclosed in (or naming) its
//! blocks are skipped with reason `shadowed` while provable sites still
//! rewrite. Truly opaque sites (macro-token / attribute interiors
//! mentioning `old`) veto without `# fallback=syntax` and skip with
//! `macro-hidden` under it (scalar/member parity). Bare `Old` idents
//! are not assoc syntax and are ignored outright — never claimed, never
//! vetoed, never orphaned.

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::edits::TextEdit;
use crate::follow_reasons::{reason as skip_reason, FollowSkip};
use crate::lines::{FileState, LineIndex};
use crate::matcher_item::span_bytes;
use crate::use_graph::FollowVeto;

/// Provenance domain of one assoc rename: which containers name the
/// renamed item. Derived from the definition hunk's header filter.
#[derive(Debug, Clone)]
pub(crate) struct AssocDomain {
    /// Renamed trait ident (`trait Tr` header, or `impl Tr for S`
    /// trait-path tail). `None` for inherent `impl S` renames.
    pub trait_name: Option<String>,
    /// Self-type tail for `impl` headers (`None` for trait headers).
    pub self_name: Option<String>,
    /// `true` for `type`, `false` for `const` (separate namespaces:
    /// only same-kind definitions shadow).
    pub is_type: bool,
    pub old: String,
}

/// Last `::` segment, cut at generics/whitespace (`"S < T >"` → `"S"`).
fn last_ident(normed: &str) -> String {
    let seg = normed.rsplit("::").next().unwrap_or(normed).trim();
    let mut s = String::new();
    for c in seg.chars() {
        if c.is_alphanumeric() || c == '_' {
            s.push(c);
        } else {
            break;
        }
    }
    s
}

fn norm_path(path: &syn::Path) -> String {
    quote::quote!(#path)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn norm_ty(ty: &syn::Type) -> String {
    quote::quote!(#ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the follow domain from an `AssocRename` header. `None` when the
/// header does not parse (the definition hunk then matches nothing, so
/// follow resolves nothing — the zero-match failure stays loud there).
pub(crate) fn assoc_domain(
    impl_header: &str,
    want_trait: bool,
    is_type: bool,
    old: &str,
) -> Option<AssocDomain> {
    let filter = rsmpl_pattern::parse_impl_header(impl_header)?;
    debug_assert_eq!(filter.want_trait, want_trait);
    let trait_name = if filter.want_trait {
        Some(filter.trait_path.clone())
    } else if filter.trait_path.is_empty() {
        None
    } else {
        let t = last_ident(&filter.trait_path);
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };
    let self_name = if filter.want_trait {
        None
    } else {
        let s = last_ident(&filter.self_ty);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    Some(AssocDomain {
        trait_name,
        self_name,
        is_type,
        old: old.to_string(),
    })
}

/// Workspace-wide assoc-follow resolution (definition-block proof rule).
///
/// `renamed` carries the definition hunk's ident-span edits in
/// `def_path`: claims overlapping them are the renamed definition
/// itself (unreachable for `::`-paths by construction, skipped anyway).
pub(crate) fn resolve_assoc_workspace(
    files: &[(String, &syn::File, &str, &LineIndex)],
    domain: &AssocDomain,
    filter: &rsmpl_pattern::ImplHeaderFilter,
    def_path: &str,
    renamed: &[(usize, usize)],
    fallback: bool,
) -> Result<AssocWorkspaceOutcome, FollowVeto> {
    let census = AssocCensus::collect(files, domain, filter);
    let def_claims = collect_impl_def_claims(files, domain, filter);
    let renamed_hit_in = |path: &str, a: usize, b: usize| {
        path == def_path && renamed.iter().any(|(s, e)| a < *e && *s < b)
    };
    let mut per_file: AssocFileEdits = Vec::new();
    let mut skipped: Vec<FollowSkip> = Vec::new();
    let mut opaque: Vec<(String, u32, String)> = Vec::new();
    // Index def claims by file for the per-file loop below.
    let mut def_by_file: std::collections::HashMap<&str, Vec<(usize, usize, u32)>> =
        std::collections::HashMap::new();
    for (path, a, b, line) in &def_claims {
        def_by_file
            .entry(path.as_str())
            .or_default()
            .push((*a, *b, *line));
    }
    // Inherent same-kind definitions are a different item: evidence only.
    for (path, line) in collect_inherent_def_lines(files, domain) {
        skipped.push(FollowSkip::new(
            format!(
                "{path}:{line}: inherent assoc item `{}` (different item) left untouched",
                domain.old,
            ),
            &path,
            line,
            skip_reason::SHADOWED,
        ));
    }
    for (path, parsed, original, index) in files {
        let mut walker = AssocWalker::new(domain, filter, &census);
        walker.visit_file(parsed);
        for (line, ctx) in &walker.opaque {
            opaque.push((path.clone(), *line, ctx.clone()));
        }
        let mut edits: Vec<(usize, usize, bool)> = Vec::new();
        for span in &walker.claims {
            let line = span.start().line as u32;
            let Some((a, b)) = span_bytes(original, index, *span) else {
                return Err(FollowVeto {
                    reason: format!(
                        "use of `{}` at {path}:{line} maps outside the file",
                        domain.old
                    ),
                });
            };
            if original.get(a..b) != Some(domain.old.as_str()) {
                return Err(FollowVeto {
                    reason: format!(
                        "use of `{}` at {path}:{line} is not the expected text",
                        domain.old
                    ),
                });
            }
            if renamed_hit_in(path, a, b) {
                continue;
            }
            edits.push((a, b, false));
        }
        // WI-1: trait-proven `impl Trait for S` definitions are definition
        // claims (labelled `rename`), not use sites. Same text guard and
        // renamed-overlap skip as path tails.
        if let Some(defs) = def_by_file.get(path.as_str()) {
            for (a, b, line) in defs {
                if original.get(*a..*b) != Some(domain.old.as_str()) {
                    continue;
                }
                if renamed_hit_in(path, *a, *b) {
                    continue;
                }
                let _ = line;
                edits.push((*a, *b, true));
            }
        }
        edits.sort_unstable();
        edits.dedup();
        if !edits.is_empty() {
            per_file.push((path.clone(), edits));
        }
        for line in &walker.shadowed {
            skipped.push(FollowSkip::new(
                format!(
                    "{path}:{line}: shadowed assoc item `{}` (same name defined in another block) left untouched",
                    domain.old,
                ),
                path,
                *line,
                skip_reason::SHADOWED,
            ));
        }
    }
    per_file.sort_by(|a, b| a.0.cmp(&b.0));
    // Opaque sites veto without fallback, skip with it (scalar parity);
    // shadowed sites always skip (WI-13 per-block rule, never a veto).
    opaque.sort();
    opaque.dedup();
    if !opaque.is_empty() && !fallback {
        let (path, line, ctx) = &opaque[0];
        return Err(FollowVeto {
            reason: format!(
                "use of `{}` inside {ctx} at {path}:{line}: cannot prove it names the renamed assoc item",
                domain.old
            ),
        });
    }
    for (path, line, ctx) in &opaque {
        skipped.push(FollowSkip::new(
            format!(
                "{path}:{line}: use of `{}` inside {ctx} left untouched (opaque)",
                domain.old
            ),
            path,
            *line,
            skip_reason::MACRO_HIDDEN,
        ));
    }
    Ok(AssocWorkspaceOutcome { per_file, skipped })
}

/// One assoc follow edit: byte span plus whether it is a WI-1
/// impl-definition claim (`true` → `rename` label) or a `::`-tail use
/// site (`false` → `follow` label).
pub(crate) type AssocEdit = (usize, usize, bool);

/// Per-file assoc follow edits.
pub(crate) type AssocFileEdits = Vec<(String, Vec<AssocEdit>)>;

/// Workspace outcome: path → (start, end, is_definition) byte spans
/// (rewritten to `new` by the caller) plus structured skips.
/// `is_definition == true` marks an `impl Trait for S` definition ident
/// (WI-1: labelled `rename`); `false` marks a `::`-tail use site
/// (labelled `follow`).
#[derive(Debug)]
pub(crate) struct AssocWorkspaceOutcome {
    pub per_file: AssocFileEdits,
    pub skipped: Vec<FollowSkip>,
}

/// WI-27: orphan evidence for definition-only assoc renames (SPEC
/// 14.8). Without follow, orphans list every container-proven use site
/// (`Self::`, `S::`, `Tr::`, qself tails) the rename leaves behind.
/// Mirrors `member_orphans`: the definition hunk's own ident-span edits
/// plus every loaded `.rs` file feed the assoc collector. An
/// unparseable header yields no orphans (the definition hunk itself
/// matches nothing, so the run already fails loud there).
/// (Relocated from `use_graph_local.rs`, its park after the `plan.rs`
/// gate move; behaviour-preserving.)
#[allow(clippy::too_many_arguments)]
pub(crate) fn assoc_orphans(
    states: &HashMap<String, FileState>,
    file_edits: &HashMap<String, Vec<TextEdit>>,
    path: &str,
    edits_before: usize,
    impl_header: &str,
    want_trait: bool,
    is_type: bool,
    old: &str,
) -> Vec<crate::OrphanReport> {
    let Some(domain) = assoc_domain(impl_header, want_trait, is_type, old) else {
        return Vec::new();
    };
    let Some(filter) = rsmpl_pattern::parse_impl_header(impl_header) else {
        return Vec::new();
    };
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
    collect_assoc_orphans(&refs, &domain, &filter, path, &renamed)
}

/// Report-only orphan collection for definition-only assoc renames
/// (WI-13 acceptance: without follow, orphans list every `Self::`
/// site; WI-1: plus every trait-proven `impl Trait for S` definition).
/// Same claim walk as follow, never vetoing: claimable sites
/// become `off-target` orphans, macro/attribute interiors
/// `macro-hidden`. Unparseable files are excluded (the follow tripwire
/// owns them).
pub(crate) fn collect_assoc_orphans(
    files: &[(String, &syn::File, &str, &LineIndex)],
    domain: &AssocDomain,
    filter: &rsmpl_pattern::ImplHeaderFilter,
    def_path: &str,
    renamed: &[(usize, usize)],
) -> Vec<crate::OrphanReport> {
    let census = AssocCensus::collect(files, domain, filter);
    let def_claims = collect_impl_def_claims(files, domain, filter);
    let mut def_by_file: std::collections::HashMap<&str, Vec<(usize, usize)>> =
        std::collections::HashMap::new();
    for (path, a, b, _) in &def_claims {
        def_by_file.entry(path.as_str()).or_default().push((*a, *b));
    }
    let renamed_hit_in = |path: &str, a: usize, b: usize| {
        path == def_path && renamed.iter().any(|(s, e)| a < *e && *s < b)
    };
    let mut out: Vec<crate::OrphanReport> = Vec::new();
    for (path, parsed, original, index) in files {
        let mut walker = AssocWalker::new(domain, filter, &census);
        walker.visit_file(parsed);
        let mut lines: Vec<(u32, &'static str)> = Vec::new();
        for span in &walker.claims {
            let line = span.start().line as u32;
            let Some((a, b)) = span_bytes(original, index, *span) else {
                continue;
            };
            if original.get(a..b) != Some(domain.old.as_str()) {
                continue;
            }
            if renamed_hit_in(path, a, b) {
                continue;
            }
            lines.push((line, "off-target"));
        }
        if let Some(defs) = def_by_file.get(path.as_str()) {
            for (a, b) in defs {
                if original.get(*a..*b) != Some(domain.old.as_str()) {
                    continue;
                }
                if renamed_hit_in(path, *a, *b) {
                    continue;
                }
                let line = index.line_of(*a);
                lines.push((line, "off-target"));
            }
        }
        for line in &walker.shadowed {
            lines.push((*line, "off-target"));
        }
        for (line, _) in &walker.opaque {
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

/// WI-1 definition set: for a trait-header rename, every
/// `impl <Trait> for <Self>` block whose trait tail names the renamed
/// trait contributes its same-kind, same-name `ImplItem` definition
/// ident. Returns (path, start, end, line). Inherent impls (`trait_:
/// None`) never claim — they are a different item. Gated on
/// `filter.want_trait`: impl-header renames own exactly one block via
/// the definition matcher already.
///
/// Proof discipline is syntactic (D8): last-segment equality on the
/// trait path, the same rule the `Self::`-site walker uses for
/// `trait_matches`. Glob/second-hop headers that do not textually name
/// the trait simply yield no claim here (skip, never veto).
fn collect_impl_def_claims(
    files: &[(String, &syn::File, &str, &LineIndex)],
    domain: &AssocDomain,
    filter: &rsmpl_pattern::ImplHeaderFilter,
) -> Vec<(String, usize, usize, u32)> {
    if !filter.want_trait {
        return Vec::new();
    }
    let Some(trait_name) = &domain.trait_name else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (path, parsed, original, index) in files {
        struct DefVisitor<'a> {
            trait_name: &'a str,
            is_type: bool,
            old: &'a str,
            original: &'a str,
            index: &'a LineIndex,
            out: Vec<(usize, usize, u32)>,
        }
        impl<'ast> Visit<'ast> for DefVisitor<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                let names_trait = match &node.trait_ {
                    Some((_, path, _)) => last_ident(&norm_path(path)) == *self.trait_name,
                    None => false,
                };
                if names_trait {
                    if self.is_type {
                        for it in &node.items {
                            if let syn::ImplItem::Type(ty) = it {
                                if ty.ident == self.old {
                                    if let Some((a, b)) =
                                        span_bytes(self.original, self.index, ty.ident.span())
                                    {
                                        if self.original.get(a..b) == Some(self.old) {
                                            let line = ty.ident.span().start().line as u32;
                                            self.out.push((a, b, line));
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        for it in &node.items {
                            if let syn::ImplItem::Const(c) = it {
                                if c.ident == self.old {
                                    if let Some((a, b)) =
                                        span_bytes(self.original, self.index, c.ident.span())
                                    {
                                        if self.original.get(a..b) == Some(self.old) {
                                            let line = c.ident.span().start().line as u32;
                                            self.out.push((a, b, line));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut visitor = DefVisitor {
            trait_name,
            is_type: domain.is_type,
            old: &domain.old,
            original,
            index,
            out: Vec::new(),
        };
        visitor.visit_file(parsed);
        for (a, b, line) in visitor.out {
            out.push((path.clone(), a, b, line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Inherent same-kind definitions of `old`: `impl S { type Item … }` with
/// no trait. A different item from the renamed trait item — follow
/// evidence (`shadowed`), never a claim. Returns (path, line).
fn collect_inherent_def_lines(
    files: &[(String, &syn::File, &str, &LineIndex)],
    domain: &AssocDomain,
) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (path, parsed, _, _) in files {
        struct InhVisitor<'a> {
            is_type: bool,
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for InhVisitor<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                if node.trait_.is_none() {
                    let defines = if self.is_type {
                        node.items
                            .iter()
                            .any(|it| matches!(it, syn::ImplItem::Type(ty) if ty.ident == self.old))
                    } else {
                        node.items
                            .iter()
                            .any(|it| matches!(it, syn::ImplItem::Const(c) if c.ident == self.old))
                    };
                    if defines {
                        for it in &node.items {
                            let ident_span = match it {
                                syn::ImplItem::Type(ty) if self.is_type && ty.ident == self.old => {
                                    Some(ty.ident.span())
                                }
                                syn::ImplItem::Const(c) if !self.is_type && c.ident == self.old => {
                                    Some(c.ident.span())
                                }
                                _ => None,
                            };
                            if let Some(span) = ident_span {
                                self.out.push(span.start().line as u32);
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut visitor = InhVisitor {
            is_type: domain.is_type,
            old: &domain.old,
            out: Vec::new(),
        };
        visitor.visit_file(parsed);
        for line in visitor.out {
            out.push((path.clone(), line));
        }
    }
    out.sort();
    out.dedup();
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

/// Workspace census: which self types implement the renamed trait, and
/// which self types carry an inherent (non-domain) same-kind `old`.
struct AssocCensus {
    /// Self-type tails of `impl` blocks whose trait tail == trait name.
    domain_self: Vec<String>,
    /// Self-type tails of inherent impls defining same-kind `old`
    /// outside header-matching blocks.
    inherent_definers: Vec<String>,
}

impl AssocCensus {
    fn collect(
        files: &[(String, &syn::File, &str, &LineIndex)],
        domain: &AssocDomain,
        filter: &rsmpl_pattern::ImplHeaderFilter,
    ) -> Self {
        let mut census = Self {
            domain_self: Vec::new(),
            inherent_definers: Vec::new(),
        };
        for (_, parsed, _, _) in files {
            let mut collector = CensusVisitor {
                domain,
                filter,
                census: &mut census,
            };
            collector.visit_file(parsed);
        }
        census.domain_self.sort();
        census.domain_self.dedup();
        census.inherent_definers.sort();
        census.inherent_definers.dedup();
        census
    }
}

struct CensusVisitor<'a> {
    domain: &'a AssocDomain,
    filter: &'a rsmpl_pattern::ImplHeaderFilter,
    census: &'a mut AssocCensus,
}

impl<'ast> Visit<'ast> for CensusVisitor<'_> {
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if let Some(trait_name) = &self.domain.trait_name {
            if let Some((_, path, _)) = &node.trait_ {
                if last_ident(&norm_path(path)) == *trait_name {
                    let s = last_ident(&norm_ty(&node.self_ty));
                    if !s.is_empty() {
                        self.census.domain_self.push(s);
                    }
                }
            }
        }
        // Inherent impls defining same-kind `old` outside the renamed
        // header shadow `S::Old` uses (const only in practice, but the
        // census stays kind-agnostic rather than guessing).
        if node.trait_.is_none()
            && !rsmpl_pattern::impl_block_matches(self.filter, &syn::Item::Impl(node.clone()))
        {
            let defines = if self.domain.is_type {
                node.items
                    .iter()
                    .any(|it| matches!(it, syn::ImplItem::Type(ty) if ty.ident == self.domain.old))
            } else {
                node.items
                    .iter()
                    .any(|it| matches!(it, syn::ImplItem::Const(c) if c.ident == self.domain.old))
            };
            if defines {
                let s = last_ident(&norm_ty(&node.self_ty));
                if !s.is_empty() {
                    self.census.inherent_definers.push(s);
                }
            }
        }
        visit::visit_item_impl(self, node);
    }
}

/// One enclosing trait/impl block on the walk stack.
struct BlockCtx {
    /// `Self::Old` here names the renamed item (header match, or an
    /// impl of the renamed trait).
    domain_impl: bool,
    /// A same-kind `old` defined by this block outside the renamed
    /// header (a shadowing definition for enclosed `Self::` sites).
    shadows: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ContainerStatus {
    Claim,
    Shadow,
    Ignore,
}

/// Claim walk: collects proven `old` ident spans (`claims`), shadowed
/// site lines (`shadowed`), and opaque macro/attribute lines (`opaque`).
struct AssocWalker<'a> {
    domain: &'a AssocDomain,
    filter: &'a rsmpl_pattern::ImplHeaderFilter,
    census: &'a AssocCensus,
    stack: Vec<BlockCtx>,
    claims: Vec<proc_macro2::Span>,
    shadowed: Vec<u32>,
    opaque: Vec<(u32, String)>,
}

impl<'a> AssocWalker<'a> {
    fn new(
        domain: &'a AssocDomain,
        filter: &'a rsmpl_pattern::ImplHeaderFilter,
        census: &'a AssocCensus,
    ) -> Self {
        Self {
            domain,
            filter,
            census,
            stack: Vec::new(),
            claims: Vec::new(),
            shadowed: Vec::new(),
            opaque: Vec::new(),
        }
    }

    /// Container verdict for a plain `X::Old` tail (no `Self`, no
    /// enclosing-block dependence): the name `X` either proves the
    /// renamed item, proves a shadowing item, or proves nothing.
    fn container_status(&self, container: &str) -> ContainerStatus {
        if let Some(t) = &self.domain.trait_name {
            if container == t {
                return ContainerStatus::Claim;
            }
        }
        if let Some(s) = &self.domain.self_name {
            if container == s {
                return self.inherent_shadow_status(s, ContainerStatus::Claim);
            }
        }
        if self.census.domain_self.iter().any(|s| s == container) {
            return self.inherent_shadow_status(container, ContainerStatus::Claim);
        }
        ContainerStatus::Ignore
    }

    /// An inherent same-kind definition of `old` on this self type
    /// outside the renamed header shadows the trait item at `S::Old`
    /// sites (const only in practice).
    fn inherent_shadow_status(&self, container: &str, proven: ContainerStatus) -> ContainerStatus {
        if self.census.inherent_definers.iter().any(|s| s == container) {
            ContainerStatus::Shadow
        } else {
            proven
        }
    }

    /// Classify one `old` segment at `index` (span = that ident, whose
    /// container is the preceding segment). `Self::Old` resolves through
    /// the enclosing block; explicit `<S as Tr>::Old` through its trait;
    /// plain `X::Old` through the container name. Index 0 is bare-`Old`
    /// syntax — ignored outright.
    fn classify_path(
        &mut self,
        segments: &[String],
        index: usize,
        has_qself: bool,
        line: u32,
        span: proc_macro2::Span,
    ) {
        if index == 0 || index >= segments.len() {
            return;
        }
        let container = segments[index - 1].clone();
        if container == "Self" && !has_qself {
            match self.stack.last() {
                Some(top) if top.domain_impl => self.claims.push(span),
                Some(top) if top.shadows => self.shadowed.push(line),
                // No enclosing block, or an unrelated block whose item
                // this provably is not: ignore, never veto.
                _ => {}
            }
            return;
        }
        if has_qself {
            // `<S as Tr>::Old` is explicit: only our trait claims it.
            if Some(container.as_str()) == self.domain.trait_name.as_deref() {
                self.claims.push(span);
            }
            return;
        }
        match self.container_status(&container) {
            ContainerStatus::Claim => self.claims.push(span),
            ContainerStatus::Shadow => self.shadowed.push(line),
            ContainerStatus::Ignore => {}
        }
    }
}

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

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

/// `use`-tree walker (free function: nested `fn` cannot reuse the
/// `Visit` lifetime). Claims `use C::Old` / `use C::{Old, …}` leaves
/// under the same container rule as paths; `use Self::Old` names the
/// module, never the type.
fn walk_use_tree(walker: &mut AssocWalker<'_>, tree: &syn::UseTree, parent: Option<String>) {
    match tree {
        syn::UseTree::Path(p) => {
            // `use …::Old::…` (old mid-path): not an assoc tail.
            walk_use_tree(walker, &p.tree, Some(p.ident.to_string()));
        }
        syn::UseTree::Name(n) => {
            if n.ident == walker.domain.old {
                if let Some(container) = parent {
                    if container != "Self" {
                        let line = n.ident.span().start().line as u32;
                        let span = n.ident.span();
                        walker.classify_path(
                            &[container, walker.domain.old.clone()],
                            1,
                            false,
                            line,
                            span,
                        );
                    }
                }
            }
        }
        syn::UseTree::Rename(r) => {
            if r.ident == walker.domain.old {
                if let Some(container) = parent {
                    if container != "Self" {
                        let line = r.ident.span().start().line as u32;
                        let span = r.ident.span();
                        walker.classify_path(
                            &[container, walker.domain.old.clone()],
                            1,
                            false,
                            line,
                            span,
                        );
                    }
                }
            }
        }
        syn::UseTree::Group(g) => {
            for member in &g.items {
                walk_use_tree(walker, member, parent.clone());
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

impl<'ast> Visit<'ast> for AssocWalker<'_> {
    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        let header_matches =
            rsmpl_pattern::impl_block_matches(self.filter, &syn::Item::Trait(node.clone()));
        let defines = node.items.iter().any(|it| match it {
            syn::TraitItem::Type(ty) => self.domain.is_type && ty.ident == self.domain.old,
            syn::TraitItem::Const(c) => !self.domain.is_type && c.ident == self.domain.old,
            _ => false,
        });
        self.stack.push(BlockCtx {
            domain_impl: header_matches,
            shadows: defines && !header_matches,
        });
        visit::visit_item_trait(self, node);
        self.stack.pop();
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let header_matches =
            rsmpl_pattern::impl_block_matches(self.filter, &syn::Item::Impl(node.clone()));
        let defines = node.items.iter().any(|it| match it {
            syn::ImplItem::Type(ty) => self.domain.is_type && ty.ident == self.domain.old,
            syn::ImplItem::Const(c) => !self.domain.is_type && c.ident == self.domain.old,
            _ => false,
        });
        let trait_matches = match (&node.trait_, &self.domain.trait_name) {
            (Some((_, path, _)), Some(t)) => last_ident(&norm_path(path)) == *t,
            _ => false,
        };
        self.stack.push(BlockCtx {
            domain_impl: header_matches || trait_matches,
            shadows: defines && !header_matches && !trait_matches,
        });
        visit::visit_item_impl(self, node);
        self.stack.pop();
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        walk_use_tree(self, &node.tree, None);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        // Every `old` segment past the head is a candidate tail:
        // `S::Old` and `S::Old::assoc_fn()` both name the assoc item
        // through `S` (the trailing path is preserved; only the `old`
        // ident span rewrites). The head itself (`Old::…`) is bare-`Old`
        // syntax — ignored outright.
        let segments = path_segments(node);
        for (i, seg) in node.segments.iter().enumerate() {
            if i > 0 && seg.ident == self.domain.old {
                let line = seg.ident.span().start().line as u32;
                let span = seg.ident.span();
                self.classify_path(&segments, i, false, line, span);
            }
        }
        visit::visit_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        // QSelf paths carry their trait explicitly: classify once with
        // it and skip the inner walk (the bare-path arm below would
        // otherwise re-read the same segments without the qualifier).
        if node.qself.is_some() {
            if let Some(last) = node.path.segments.last() {
                if last.ident == self.domain.old {
                    let line = last.ident.span().start().line as u32;
                    let span = last.ident.span();
                    let segments = path_segments(&node.path);
                    let idx = segments.len().saturating_sub(1);
                    self.classify_path(&segments, idx, true, line, span);
                }
            }
            return;
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if node.qself.is_some() {
            if let Some(last) = node.path.segments.last() {
                if last.ident == self.domain.old {
                    let line = last.ident.span().start().line as u32;
                    let span = last.ident.span();
                    let segments = path_segments(&node.path);
                    let idx = segments.len().saturating_sub(1);
                    self.classify_path(&segments, idx, true, line, span);
                }
            }
            return;
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // No recursion: macro interiors are opaque, never claimed.
        // One entry per ident line (WI-2b ident-line parity — not the
        // macro start line). Single-line macros are unchanged.
        for line in macro_ident_lines(&node.tokens, &self.domain.old) {
            self.opaque.push((line, "a macro invocation".to_string()));
        }
    }

    fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
        // No recursion: attribute paths are opaque, never claimed.
        if node
            .path()
            .segments
            .iter()
            .any(|s| s.ident == self.domain.old)
        {
            self.opaque
                .push((node.span().start().line as u32, "an attribute".to_string()));
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
}
