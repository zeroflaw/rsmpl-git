//! WI-6a: same-file `# follow=uses` resolver (SPEC 14.8).
//!
//! After a fn/item rename hunk rewrites its definition ident(s), this
//! module collects the same-file *use sites* of the old name — type paths,
//! value/call paths (bare and qualified, incl. turbofish), struct-literal
//! paths, pattern paths, and `use`-tree segments — and returns their ident
//! spans for rewriting under the same rule.
//!
//! Precision-first (SPEC 2.6, D3): anything the syntactic walk cannot prove
//! is a veto, not a guess. A second same-named definition, a local binding,
//! or a macro/attribute mention the resolver cannot attribute makes the
//! whole hunk fail with a follow veto — unless the hunk also carries
//! `# fallback=syntax`, in which case the unprovable sites are skipped and
//! listed for the WI-2 `follow_skipped` binding. Field accesses, method
//! idents, and `qself` paths are never rewritten (different namespaces /
//! declined, WI-3 scope note); they never veto either, except a `qself`
//! path naming `old`, which is opaque.
//!
//! v0.1 scope is the definition's own file (WI-6b owns the workspace
//! index). The hunk's range is ignored for uses: a follow rewrites the
//! whole file, which is the point (no range discovery).

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};
use rsmpl_pattern::PatternKind;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::edits::{edit_spans, TextEdit};
use crate::lines::{FileState, LineIndex};
use crate::matcher_item::span_bytes;

/// Successful resolution: byte ranges to rewrite plus skip notes.
#[derive(Debug)]
pub struct FollowOutcome {
    /// Ident byte ranges (`original[a..b] == old`), sorted, deduped.
    pub spans: Vec<(usize, usize)>,
    /// `line N: ...` notes for sites left untouched under fallback.
    pub skipped: Vec<String>,
}

/// A fail-loud veto: the first unprovable site (no `# fallback=syntax`).
#[derive(Debug)]
pub struct FollowVeto {
    /// Human reason naming the site (embeds the 1-based line).
    pub reason: String,
}

/// Resolve same-file use sites of `old` for a `old` → `new` rename.
///
/// `renamed` carries the byte ranges the definition hunk already rewrote
/// (its ident-span edits): a same-named definition overlapping one of them
/// is the renamed definition itself, any other is shadowing. `fallback`
/// mirrors `# fallback=syntax` (skip + report instead of veto).
pub fn resolve(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    old: &str,
    renamed: &[(usize, usize)],
    fallback: bool,
) -> Result<FollowOutcome, FollowVeto> {
    let collector = collect(parsed, old);

    // Byte-map definitions (fail loud on unmappable spans, never guess).
    let mut def_ranges: Vec<(usize, usize, u32, &'static str)> = Vec::new();
    for (span, kind) in &collector.defs {
        let line = span.start().line as u32;
        match span_bytes(original, index, *span) {
            Some((a, b)) => def_ranges.push((a, b, line, kind)),
            None => {
                return Err(FollowVeto {
                    reason: format!(
                        "definition of `{old}` ({kind}) at line {line} maps outside the file"
                    ),
                });
            }
        }
    }
    let renamed_hit = |a: usize, b: usize| renamed.iter().any(|(s, e)| a < *e && *s < b);
    let shadow: Vec<(u32, &'static str)> = def_ranges
        .iter()
        .filter(|(a, b, _, _)| !renamed_hit(*a, *b))
        .map(|(_, _, line, kind)| (*line, *kind))
        .collect();

    // Map + verify use spans (text must be exactly `old`, never guessed).
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for span in &collector.uses {
        let line = span.start().line as u32;
        let (a, b) = span_bytes(original, index, *span).ok_or_else(|| FollowVeto {
            reason: format!("use of `{old}` at line {line} maps outside the file"),
        })?;
        if original.get(a..b) != Some(old) {
            return Err(FollowVeto {
                reason: format!("use of `{old}` at line {line} is not the expected text"),
            });
        }
        // The renamed definition's own header spans (e.g. `impl Old`
        // self-ty, already rewritten by the def edit) are not uses.
        if renamed_hit(a, b) {
            continue;
        }
        spans.push((a, b));
    }
    spans.sort_unstable();
    spans.dedup();

    let mut skipped: Vec<String> = Vec::new();
    if !shadow.is_empty() {
        // Every remaining use is ambiguous (which definition does it name?),
        // so under fallback all use rewriting is skipped, not just the
        // shadowed definition. Without fallback the hunk vetoes.
        if !fallback {
            let (line, kind) = shadow[0];
            return Err(FollowVeto {
                reason: format!(
                    "shadowed definition of `{old}` ({kind}) at line {line}: cannot tell which definition each use names"
                ),
            });
        }
        for (line, kind) in &shadow {
            skipped.push(format!(
                "line {line}: shadowed definition of `{old}` ({kind}); {} use sites left untouched",
                spans.len()
            ));
        }
        spans.clear();
    }
    if !collector.opaque.is_empty() {
        if !fallback {
            let (line, ctx) = &collector.opaque[0];
            return Err(FollowVeto {
                reason: format!(
                    "use of `{old}` inside {ctx} at line {line}: macro/attribute interiors are opaque"
                ),
            });
        }
        for (line, ctx) in &collector.opaque {
            skipped.push(format!(
                "line {line}: use of `{old}` inside {ctx} left untouched (opaque)"
            ));
        }
    }
    if !collector.globs.is_empty() && !spans.is_empty() {
        if !fallback {
            let line = collector.globs[0];
            return Err(FollowVeto {
                reason: format!(
                    "use of `{old}` with a glob import at line {line}: glob interiors are opaque"
                ),
            });
        }
        for line in &collector.globs {
            skipped.push(format!(
                "line {line}: glob import left untouched (opaque); {} use sites left untouched",
                spans.len()
            ));
        }
        spans.clear();
    }
    Ok(FollowOutcome { spans, skipped })
}

/// Workspace-wide resolution for WI-6b1 (SPEC 14.8): the unique-definition
/// cross-file fast path.
///
/// `files` maps path → (parsed AST, original text, line index) for every
/// loaded `.rs` file. `renamed_file` + `renamed` carry the definition hunk's
/// ident-span edits (same convention as `resolve`): a same-named definition
/// overlapping one of them is the renamed definition itself.
///
/// Unique-definition rule (precision-first, SPEC 2.6/D3): when exactly one
/// definition of `old` exists workspace-wide, every collected use segment
/// names it, so all files' use spans rewrite. A second definition anywhere
/// makes every use ambiguous: without `fallback` the hunk vetoes naming the
/// first shadow `path:line`; with it all use rewriting is skipped and each
/// shadow def is listed. Opaque (macro/attribute/qself) sites behave per
/// file like `resolve`: veto without fallback, skip-with-report with it.
/// Non-`.rs` files never reach here (the planner only passes parsed files).
///
/// Returns per-file sorted/deduped byte spans plus workspace-wide skip notes
/// (`path:line`-qualified for cross-file review).
#[derive(Debug)]
pub struct WorkspaceOutcome {
    /// Path → ident byte ranges (`original[a..b] == old`), sorted/deduped.
    /// Entries exist only for files with ≥1 rewrite span.
    pub per_file: Vec<(String, Vec<(usize, usize)>)>,
    /// `path:line`-qualified notes for sites left untouched under fallback.
    pub skipped: Vec<String>,
}

pub fn resolve_workspace<'a>(
    files: &[(String, &'a syn::File, &'a str, &'a LineIndex)],
    old: &str,
    renamed_file: &str,
    renamed: &[(usize, usize)],
    fallback: bool,
) -> Result<WorkspaceOutcome, FollowVeto> {
    if files.len() <= 1 {
        // Single-file fast path: the exact WI-6a outcome and messages
        // (`line N`, no path prefix). The planner always passes the
        // definition file, so an empty set means nothing to search.
        let Some((_, parsed, original, index)) = files.first() else {
            return Ok(WorkspaceOutcome {
                per_file: Vec::new(),
                skipped: Vec::new(),
            });
        };
        let out = resolve(parsed, original, index, old, renamed, fallback)?;
        let per_file = if out.spans.is_empty() {
            Vec::new()
        } else {
            vec![(renamed_file.to_string(), out.spans)]
        };
        return Ok(WorkspaceOutcome {
            per_file,
            skipped: out.skipped,
        });
    }
    struct Collected {
        path: String,
        original: String,
        defs: Vec<(proc_macro2::Span, &'static str)>,
        uses: Vec<proc_macro2::Span>,
        opaque: Vec<(u32, String)>,
        globs: Vec<u32>,
    }
    let mut all: Vec<Collected> = Vec::with_capacity(files.len());
    for (path, parsed, original, _) in files {
        let c = collect(parsed, old);
        all.push(Collected {
            path: path.clone(),
            original: (*original).to_string(),
            defs: c.defs,
            uses: c.uses,
            opaque: c.opaque,
            globs: c.globs,
        });
    }
    let index_of = |path: &str| {
        files
            .iter()
            .find(|(p, _, _, _)| p == path)
            .map(|(_, _, _, idx)| *idx)
    };
    // Byte-map definitions workspace-wide (fail loud, never guess).
    struct DefHit {
        path: String,
        line: u32,
        kind: &'static str,
        a: usize,
        b: usize,
    }
    let mut def_hits: Vec<DefHit> = Vec::new();
    for c in &all {
        let index = index_of(&c.path).expect("workspace index present");
        for (span, kind) in &c.defs {
            let line = span.start().line as u32;
            match span_bytes(&c.original, index, *span) {
                Some((a, b)) => def_hits.push(DefHit {
                    path: c.path.clone(),
                    line,
                    kind,
                    a,
                    b,
                }),
                None => {
                    return Err(FollowVeto {
                        reason: format!(
                            "definition of `{old}` ({kind}) at {}:{line} maps outside the file",
                            c.path,
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

    // Map + verify use spans per file.
    let mut per_file: Vec<(String, Vec<(usize, usize)>)> = Vec::new();
    for c in &all {
        let index = index_of(&c.path).expect("workspace index present");
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for span in &c.uses {
            let line = span.start().line as u32;
            let (a, b) = span_bytes(&c.original, index, *span).ok_or_else(|| FollowVeto {
                reason: format!("use of `{old}` at {}:{line} maps outside the file", c.path),
            })?;
            if c.original.get(a..b) != Some(old) {
                return Err(FollowVeto {
                    reason: format!(
                        "use of `{old}` at {}:{line} is not the expected text",
                        c.path
                    ),
                });
            }
            if renamed_hit_in(&c.path, a, b) {
                continue;
            }
            spans.push((a, b));
        }
        spans.sort_unstable();
        spans.dedup();
        if !spans.is_empty() {
            per_file.push((c.path.clone(), spans));
        }
    }
    per_file.sort_by(|a, b| a.0.cmp(&b.0));
    let use_sites: usize = per_file.iter().map(|(_, s)| s.len()).sum();

    let mut skipped: Vec<String> = Vec::new();
    if !shadow.is_empty() {
        if !fallback {
            let d = shadow[0];
            return Err(FollowVeto {
                reason: format!(
                    "shadowed definition of `{old}` ({}) at {}:{}: cannot tell which definition each use names ({} use sites workspace-wide)",
                    d.kind, d.path, d.line, use_sites,
                ),
            });
        }
        for d in &shadow {
            skipped.push(format!(
                "{}:{}: shadowed definition of `{old}` ({}); {use_sites} use sites left untouched",
                d.path, d.line, d.kind,
            ));
        }
        per_file.clear();
    }
    // Opaque sites: veto without fallback, skip-with-report with it. Under
    // shadowing all spans are already cleared; opaque notes still apply.
    let mut opaque: Vec<(String, u32, String)> = Vec::new();
    for c in &all {
        for (line, ctx) in &c.opaque {
            opaque.push((c.path.clone(), *line, ctx.clone()));
        }
    }
    opaque.sort();
    if !opaque.is_empty() {
        if !fallback {
            let (path, line, ctx) = &opaque[0];
            return Err(FollowVeto {
                reason: format!(
                    "use of `{old}` inside {ctx} at {path}:{line}: macro/attribute interiors are opaque"
                ),
            });
        }
        for (path, line, ctx) in &opaque {
            skipped.push(format!(
                "{path}:{line}: use of `{old}` inside {ctx} left untouched (opaque)"
            ));
        }
        if !shadow.is_empty() {
            // Already cleared; notes above carry the signal.
        } else {
            // Opaque sites are left untouched but provable sites still
            // rewrite: nothing to subtract here because opaque idents were
            // never collected as use spans (the walk records a note instead
            // of a span). The skip notes are the report.
        }
    }
    // WI-6b2-glob: a file pairing `use m::*;` with same-name uses is
    // unprovable (the glob may import a different `old`): veto loud
    // without fallback, else drop only that file's spans (shadowing
    // above already cleared everything globally when present).
    let mut globbed: Vec<(String, u32)> = Vec::new();
    for c in &all {
        if c.globs.is_empty() {
            continue;
        }
        if per_file.iter().any(|(p, s)| p == &c.path && !s.is_empty()) {
            globbed.push((c.path.clone(), c.globs[0]));
        }
    }
    globbed.sort();
    if !globbed.is_empty() && shadow.is_empty() {
        if !fallback {
            let (path, line) = &globbed[0];
            return Err(FollowVeto {
                reason: format!(
                    "use of `{old}` with a glob import at {path}:{line}: glob interiors are opaque"
                ),
            });
        }
        for (path, line) in &globbed {
            per_file.retain(|(p, _)| p != path);
            skipped.push(format!(
                "{path}:{line}: glob import left untouched (opaque); use sites in this file left untouched"
            ));
        }
    }
    Ok(WorkspaceOutcome { per_file, skipped })
}

/// Shared single-file collection behind `resolve` (no behavior change).
fn collect<'a>(parsed: &'a syn::File, old: &'a str) -> Collector<'a> {
    let mut collector = Collector {
        old,
        defs: Vec::new(),
        uses: Vec::new(),
        opaque: Vec::new(),
        globs: Vec::new(),
    };
    collector.visit_file(parsed);
    collector
}

/// Whole-ident mention test for unparseable files (WI-6b1 tripwire): lex
/// `text` with `proc_macro2` (comments vanish, strings stay literals) and
/// report whether `old` appears as a full ident. When even lexing fails,
/// assume a mention — fail loud, never a silent stale site.
pub fn raw_mentions(text: &str, old: &str) -> bool {
    match text.parse::<proc_macro2::TokenStream>() {
        Ok(tokens) => tokens_mention(&tokens, old),
        Err(_) => true,
    }
}

/// Whole-ident test on token text, recursing through groups: `old` must
/// appear as a full ident, not a substring (`older`, `my_old` do not
/// count). A missed nested group would be a silent stale site, so the
/// walk is total over the token tree.
fn tokens_mention(tokens: &proc_macro2::TokenStream, old: &str) -> bool {
    tokens.clone().into_iter().any(|t| match t {
        proc_macro2::TokenTree::Ident(id) => id == old,
        proc_macro2::TokenTree::Group(g) => tokens_mention(&g.stream(), old),
        _ => false,
    })
}

struct Collector<'a> {
    old: &'a str,
    /// (span, kind label) definition idents named `old`.
    defs: Vec<(proc_macro2::Span, &'static str)>,
    /// Use-site ident spans named `old`.
    uses: Vec<proc_macro2::Span>,
    /// (line, context) opaque mentions.
    opaque: Vec<(u32, String)>,
    /// 1-based lines of `use …::*` glob trees (WI-6b2-glob: opaque).
    globs: Vec<u32>,
}

impl Collector<'_> {
    fn is_old(&self, ident: &syn::Ident) -> bool {
        ident == self.old
    }

    fn push_def(&mut self, ident: &syn::Ident, kind: &'static str) {
        if self.is_old(ident) {
            self.defs.push((ident.span(), kind));
        }
    }

    /// Collect every segment spelling `old` (`Old`, `m::Old`, `Old::new`,
    /// `a::Old::assoc`). Any segment naming a *different* item implies a
    /// same-named definition, which the census turns into a veto — so
    /// rewriting every `old` segment is sound, never a guess. Callers at
    /// `Type::Path` / `Expr::Path` level split off `qself` first (a
    /// qualified `Old` is opaque, WI-3 scope note); a bare `Path` node
    /// never carries qualification itself. Turbofish lives on the
    /// segment, outside the ident span, so it is preserved verbatim.
    fn push_path_use(&mut self, path: &syn::Path) {
        for seg in &path.segments {
            if seg.ident == self.old {
                self.uses.push(seg.ident.span());
            }
        }
    }

    /// `qself` split: any `old` segment under qualification is opaque
    /// (declined); anything else recurses normally so uses nested inside
    /// the `qself` type still resolve. Returns true when the site was
    /// consumed as opaque.
    fn push_qself_path(&mut self, path: &syn::Path) -> bool {
        if path.segments.iter().any(|s| s.ident == self.old) {
            self.opaque
                .push((path.span().start().line as u32, "a qself path".to_string()));
            return true;
        }
        false
    }

    /// Walk a `use` tree collecting segments spelling `old` (WI-6a: any
    /// position — `use m::Old`, `use Old as X`, `use m::{Old, Y}`).
    /// Glob trees record their line as opaque (WI-6b2-glob).
    fn walk_use_tree(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => {
                if p.ident == self.old {
                    self.uses.push(p.ident.span());
                }
                self.walk_use_tree(&p.tree);
            }
            syn::UseTree::Name(n) => {
                if n.ident == self.old {
                    self.uses.push(n.ident.span());
                }
            }
            syn::UseTree::Rename(r) => {
                if r.ident == self.old {
                    self.uses.push(r.ident.span());
                }
            }
            syn::UseTree::Glob(g) => {
                self.globs.push(g.span().start().line as u32);
            }
            syn::UseTree::Group(g) => {
                for member in &g.items {
                    self.walk_use_tree(member);
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.push_def(&node.sig.ident, "fn");
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.push_def(&node.sig.ident, "fn");
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.push_def(&node.sig.ident, "fn");
        visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.push_def(&node.ident, "struct");
        visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.push_def(&node.ident, "enum");
        visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.push_def(&node.ident, "trait");
        visit::visit_item_trait(self, node);
    }

    fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
        self.push_def(&node.ident, "union");
        visit::visit_item_union(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.push_def(&node.ident, "type");
        visit::visit_item_type(self, node);
    }

    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.push_def(&node.ident, "const");
        visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.push_def(&node.ident, "static");
        visit::visit_item_static(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.push_def(&node.ident, "mod");
        visit::visit_item_mod(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        // Self-ty and trait-side last segments name the renamed item
        // itself (Rust coherence: `impl Old` implements `Old`), so they
        // are use sites to rewrite, never shadowing definitions. The
        // renamed-range filter drops the ones the def hunk already
        // rewrote; the rest follow here (an `impl Old` left stale would
        // break the build the hunk just fixed).
        if let syn::Type::Path(tp) = &*node.self_ty {
            if tp.qself.is_none() {
                if let Some(last) = tp.path.segments.last() {
                    if last.ident == self.old {
                        self.uses.push(last.ident.span());
                    }
                }
            }
        }
        if let Some((_, path, _)) = &node.trait_ {
            if let Some(last) = path.segments.last() {
                if last.ident == self.old {
                    self.uses.push(last.ident.span());
                }
            }
        }
        visit::visit_item_impl(self, node);
    }

    fn visit_variant(&mut self, node: &'ast syn::Variant) {
        // `E::Old` would otherwise misread as a use of a renamed `Old`.
        self.push_def(&node.ident, "variant");
        visit::visit_variant(self, node);
    }

    fn visit_impl_item_const(&mut self, node: &'ast syn::ImplItemConst) {
        self.push_def(&node.ident, "assoc const");
        visit::visit_impl_item_const(self, node);
    }

    fn visit_impl_item_type(&mut self, node: &'ast syn::ImplItemType) {
        self.push_def(&node.ident, "assoc type");
        visit::visit_impl_item_type(self, node);
    }

    fn visit_trait_item_const(&mut self, node: &'ast syn::TraitItemConst) {
        self.push_def(&node.ident, "assoc const");
        visit::visit_trait_item_const(self, node);
    }

    fn visit_trait_item_type(&mut self, node: &'ast syn::TraitItemType) {
        self.push_def(&node.ident, "assoc type");
        visit::visit_trait_item_type(self, node);
    }

    fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
        // A local binding named `old` shadows value-position uses.
        self.push_def(&node.ident, "local");
        visit::visit_pat_ident(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        self.walk_use_tree(&node.tree);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        // Covers type paths, value/call paths (bare, qualified, and
        // mid-path segments — `Old::new`, `a::Old::assoc`), struct-literal
        // and pattern paths. Method idents and field members are never
        // `Path`s, so they are untouched by construction. `qself` splits
        // happen at the `Type::Path` / `Expr::Path` parents below.
        self.push_path_use(node);
        visit::visit_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        if node.qself.is_some() && self.push_qself_path(&node.path) {
            return;
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if node.qself.is_some() && self.push_qself_path(&node.path) {
            return;
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // Token interiors are opaque: no recursion, mention recorded.
        if tokens_mention(&node.tokens, self.old) {
            self.opaque.push((
                node.span().start().line as u32,
                "a macro invocation".to_string(),
            ));
        }
    }

    fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
        // Attribute paths (`#[derive(Old)]`) live outside the item/value
        // namespaces the resolver proves; a mention is opaque, and there
        // is nothing to recurse into that the resolver would claim.
        if node.path().segments.iter().any(|s| s.ident == self.old) {
            self.opaque
                .push((node.span().start().line as u32, "an attribute".to_string()));
        }
    }
}

/// WI-6a `# follow=uses`: after the definition rewrite, resolve same-file
/// use sites and append their ident-span edits under the same rule
/// (SPEC 14.8). Zero new def edits (optional zero-match) means nothing to
/// WI-6b1: one follow-only entry per non-definition file touched by
/// `# follow=uses`: (file, rewritten use-site count, line spans).
pub(crate) type FollowExtras = Vec<(String, usize, Vec<(u32, u32)>)>;

/// WI-6b1 `# follow=uses` workspace step (SPEC 14.8): after the definition
/// rewrite, resolve use sites across every loaded `.rs` file (unique-
/// definition fast path) and append their ident-span edits under the same
/// rule. Zero new def edits (optional zero-match) means nothing to follow
/// — a no-op, never a veto.
///
/// Returns extra bindings (`follow_skipped` when `# fallback=syntax` left
/// sites untouched) plus the follow-only entries. The caller reports the
/// definition file as usual and each extra entry as its own `RuleReport`
/// with the same rule name (SPEC D.2).
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow_uses_workspace(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    states: &HashMap<String, FileState>,
    file_edits: &mut HashMap<String, Vec<TextEdit>>,
    unparseable: &[(String, String)],
    def_path: &str,
    edits_before: usize,
    found: usize,
) -> Result<(HashMap<String, String>, FollowExtras), StructuredError> {
    let mut bindings = HashMap::new();
    let no_follow: FollowExtras = Vec::new();
    if found == 0 {
        return Ok((bindings, no_follow));
    }
    let (old, new) = match &compiled.kind {
        PatternKind::Fn {
            minus_name,
            plus_name,
            ..
        } => (minus_name.clone(), plus_name.clone()),
        PatternKind::ItemRename {
            minus_name,
            plus_name,
            ..
        } => (minus_name.clone(), plus_name.clone()),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "# follow=uses is only supported on fn and item rename hunks in v0.1 (other shapes keep the SPEC 14.8 follow-up hunks)",
            )
            .with_rule(rh.rule_name())
            .with_file(def_path)
            .with_suggestion("remove `# follow=uses`, or rewrite this hunk as a fn/item rename")
            .with_repair(StructuredError::fix_pattern_repair(
                "C17",
                "remove `# follow=uses`, or rewrite this hunk as a fn/item rename",
            )));
        }
    };
    // Ranges this hunk already rewrote: a same-named definition outside
    // them shadows (the resolver decides veto vs skip, never guesses).
    let renamed: Vec<(usize, usize)> = file_edits
        .get(def_path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    // Deterministic file order for reports and vetoes.
    let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> = Vec::new();
    let mut paths: Vec<&String> = states.keys().collect();
    paths.sort();
    for p in paths {
        let st = &states[p];
        if let Some(parsed) = st.parsed.as_ref() {
            refs.push((p.clone(), parsed, st.original.as_str(), &st.index));
        }
    }
    let outcome = resolve_workspace(
        &refs,
        &old,
        def_path,
        &renamed,
        rh.fallback_syntax(),
    )
    .map_err(|veto| {
        StructuredError::new(ErrorCode::SymbolConflict, veto.reason)
            .with_rule(rh.rule_name())
            .with_file(def_path)
            .with_suggestion(
                "add `# fallback=syntax` to skip ambiguous sites (listed in `follow_skipped`) instead of failing",
            )
            .with_repair(StructuredError::add_directive_repair("# fallback=syntax"))
    })?;
    // Unparseable-workspace tripwire (SPEC 14.8): a file the resolver could
    // not search that names `old` outside strings/comments fails loud
    // without fallback (never a silent stale site), or is listed and left
    // untouched with it. Token-exact via `proc_macro2` lexing; when even
    // lexing fails, assume a mention (fail loud).
    let mut tripwire: Vec<String> = Vec::new();
    for (path, text) in unparseable {
        if raw_mentions(text, &old) {
            tripwire.push(path.clone());
        }
    }
    tripwire.sort();
    tripwire.dedup();
    if !tripwire.is_empty() && !rh.fallback_syntax() {
        let path = &tripwire[0];
        return Err(StructuredError::new(
            ErrorCode::SymbolConflict,
            format!("use of `{old}` may hide in unparseable file {path}: cannot search it syntactically"),
        )
        .with_rule(rh.rule_name())
        .with_file(def_path)
        .with_suggestion(
            "add `# fallback=syntax` to skip unsearchable files (listed in `follow_skipped`) instead of failing",
        )
        .with_repair(StructuredError::add_directive_repair("# fallback=syntax")));
    }
    // Append use-span edits per file; record pre-push lengths so extra
    // reports span exactly the pushed edits.
    let mut before_lens: HashMap<String, usize> = HashMap::new();
    for (f, spans) in &outcome.per_file {
        before_lens.insert(f.clone(), file_edits.get(f).map(|v| v.len()).unwrap_or(0));
        let vec = file_edits.get_mut(f).expect("workspace file state");
        for (a, b) in spans {
            vec.push(TextEdit::new(*a, *b, new.clone()));
        }
    }
    let mut extras: FollowExtras = Vec::new();
    for (f, spans) in &outcome.per_file {
        if f == def_path {
            continue;
        }
        if spans.is_empty() {
            continue;
        }
        let st = states.get(f).expect("workspace file state");
        let edits = file_edits.get(f).expect("workspace file state");
        let before = before_lens.get(f).copied().unwrap_or(0);
        let line_spans = edit_spans(&st.index, &edits[before..]);
        extras.push((f.clone(), spans.len(), line_spans));
    }
    extras.sort_by(|a, b| a.0.cmp(&b.0));
    let mut skipped = outcome.skipped;
    for path in &tripwire {
        skipped.push(format!(
            "{path}: unparseable file left untouched (may name `{old}`)"
        ));
    }
    if !skipped.is_empty() {
        bindings.insert("follow_skipped".to_string(), skipped.join("; "));
    }
    Ok((bindings, extras))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_for_test(
        src: &str,
        old: &str,
        renamed_site: &str,
        fallback: bool,
    ) -> Result<FollowOutcome, FollowVeto> {
        let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
        let index = LineIndex::new(src);
        // The renamed definition: byte range of `renamed_site`'s `old`.
        let off = src.find(renamed_site).expect("renamed site must exist");
        let a = off + renamed_site.find(old).expect("old inside site");
        resolve(&parsed, src, &index, old, &[(a, a + old.len())], fallback)
    }

    const STRUCT_SRC: &str = "use m::Old;\nstruct Old {\n    field: i32,\n}\nimpl Old {\n    fn make() -> Old {\n        Old { field: 0 }\n    }\n}\nfn f(x: Old) -> Old {\n    let y: Old = Old::make();\n    let z = Old;\n    z\n}\n";

    #[test]
    fn struct_follow_collects_type_literal_use_and_value_sites() {
        let out = resolve_for_test(STRUCT_SRC, "Old", "struct Old", false).unwrap();
        assert!(out.skipped.is_empty());
        // use-tree + impl self-ty (filtered: it is the renamed def range…
        // no — the renamed range is the struct header, so the self-ty IS a
        // use here) + return type + literal + param + return + ascription +
        // leading `Old::make` + value = 9 use spans.
        assert_eq!(out.spans.len(), 9, "spans: {:?}", out.spans);
    }

    #[test]
    fn struct_follow_skips_nothing_when_clean() {
        let out = resolve_for_test(STRUCT_SRC, "Old", "struct Old", true).unwrap();
        assert!(out.skipped.is_empty());
        assert_eq!(out.spans.len(), 9);
    }

    #[test]
    fn middle_segment_module_paths_rewrite() {
        // `crate::Old::make` names `Old` mid-path; every-segment rule
        // rewrites it.
        let src = "mod m;\nstruct Old;\nfn f() {\n    let x = crate::Old::make();\n    x\n}\n";
        let out = resolve_for_test(src, "Old", "struct Old", false).unwrap();
        assert!(out.skipped.is_empty());
        assert_eq!(out.spans.len(), 1, "spans: {:?}", out.spans);
    }

    #[test]
    fn enum_follow_rewrites_leading_variant_qualifier() {
        // Renaming `E` must reach `E::V`: the item names the first segment.
        let src = "enum E {\n    V,\n}\nfn f(x: E) -> E {\n    let y = E::V;\n    y\n}\n";
        let out = resolve_for_test(src, "E", "enum E", false).unwrap();
        assert!(out.skipped.is_empty());
        assert_eq!(out.spans.len(), 3, "spans: {:?}", out.spans);
    }

    #[test]
    fn nested_macro_groups_are_opaque() {
        let src = "struct Old;\nfn f() {\n    m!(vec![Old]);\n    let x = 1;\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("opaque"), "got: {}", err.reason);
    }

    #[test]
    fn fn_follow_rewrites_calls_but_not_methods_or_fields() {
        let src = "fn old(x: i32) -> i32 {\n    x + 1\n}\nfn g(s: S) -> i32 {\n    let a = old(1);\n    let b = m::old(2);\n    let c = s.old();\n    let d = s.old;\n    a + b\n}\n";
        let out = resolve_for_test(src, "old", "fn old", false).unwrap();
        assert!(out.skipped.is_empty());
        // `old(1)` + `m::old(2)` only: method call + field stay.
        assert_eq!(out.spans.len(), 2, "spans: {:?}", out.spans);
    }

    #[test]
    fn shadowed_definition_vetoes_without_fallback() {
        let src = "mod a {\n    pub struct Old;\n}\nmod b {\n    pub struct Old;\n}\n";
        let err = resolve_for_test(src, "Old", "pub struct Old", false).unwrap_err();
        assert!(
            err.reason.contains("shadowed definition"),
            "got: {}",
            err.reason
        );
    }

    #[test]
    fn shadowed_definition_skips_all_uses_with_fallback() {
        let src = "mod a {\n    pub struct Old;\n}\nmod b {\n    pub struct Old;\n    pub fn f(x: Old) -> Old {\n        x\n    }\n}\n";
        let out = resolve_for_test(src, "Old", "pub struct Old", true).unwrap();
        assert!(out.spans.is_empty(), "all uses ambiguous: {:?}", out.spans);
        assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
        assert!(out.skipped[0].contains("use sites left untouched"));
    }

    #[test]
    fn macro_mention_vetoes_without_fallback_and_skips_with_it() {
        let src = "struct Old;\nfn f() {\n    m!(Old);\n    let x: Old = make();\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("opaque"), "got: {}", err.reason);
        let out = resolve_for_test(src, "Old", "struct Old", true).unwrap();
        // Clean sites still rewrite; only the macro site is skipped.
        assert_eq!(out.spans.len(), 1, "spans: {:?}", out.spans);
        assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
    }

    #[test]
    fn qself_mention_is_opaque() {
        let src =
            "struct Old;\nfn f<T: Tr>() where T: Tr {\n    let x = <T as Tr>::Old;\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("qself"), "got: {}", err.reason);
    }

    #[test]
    fn variant_with_same_name_shadows() {
        let src = "struct Old;\nenum E {\n    Old,\n}\nfn f(x: E) -> E {\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("shadowed"), "got: {}", err.reason);
    }

    #[test]
    fn local_binding_named_old_shadows_fn_rename() {
        let src =
            "fn old() -> i32 {\n    1\n}\nfn g() -> i32 {\n    let old = 2;\n    old + old()\n}\n";
        let err = resolve_for_test(src, "old", "fn old", false).unwrap_err();
        assert!(err.reason.contains("shadowed"), "got: {}", err.reason);
    }

    fn workspace_for_test(
        files: &[(&str, &str)],
        old: &str,
        renamed_file: &str,
        renamed_site: &str,
        fallback: bool,
    ) -> Result<WorkspaceOutcome, FollowVeto> {
        let parsed: Vec<syn::File> = files
            .iter()
            .map(|(_, src)| syn::parse_str(src).expect("test src must parse"))
            .collect();
        let originals: Vec<String> = files.iter().map(|(_, s)| (*s).to_string()).collect();
        let indexes: Vec<LineIndex> = originals.iter().map(|s| LineIndex::new(s)).collect();
        let refs: Vec<(String, &syn::File, &str, &LineIndex)> = files
            .iter()
            .enumerate()
            .map(|(i, (path, _))| {
                (
                    (*path).to_string(),
                    &parsed[i],
                    originals[i].as_str(),
                    &indexes[i],
                )
            })
            .collect();
        let def_src = files
            .iter()
            .find(|(p, _)| *p == renamed_file)
            .expect("renamed file present")
            .1;
        let off = def_src.find(renamed_site).expect("renamed site must exist");
        let a = off + renamed_site.find(old).expect("old inside site");
        resolve_workspace(&refs, old, renamed_file, &[(a, a + old.len())], fallback)
    }

    #[test]
    fn workspace_rewrites_uses_across_files() {
        let out = workspace_for_test(
            &[
                ("a.rs", "pub struct Old;\n"),
                (
                    "b.rs",
                    "use crate::Old;\nfn f(x: Old) -> Old {\n    Old\n}\n",
                ),
            ],
            "Old",
            "a.rs",
            "struct Old",
            false,
        )
        .unwrap();
        assert!(out.skipped.is_empty());
        let total: usize = out.per_file.iter().map(|(_, s)| s.len()).sum();
        assert_eq!(total, 4, "per_file: {:?}", out.per_file);
        let b = out
            .per_file
            .iter()
            .find(|(p, _)| p == "b.rs")
            .expect("b.rs entry");
        assert_eq!(b.1.len(), 4);
        assert!(out.per_file.iter().all(|(p, _)| p != "a.rs"));
    }

    #[test]
    fn workspace_second_definition_vetoes_and_skips() {
        let files = [
            ("a.rs", "pub struct Old;\n"),
            ("b.rs", "pub struct Old;\nfn f(x: Old) -> Old {\n    x\n}\n"),
        ];
        let err = workspace_for_test(&files, "Old", "a.rs", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("shadowed"), "got: {}", err.reason);
        assert!(err.reason.contains("b.rs"), "got: {}", err.reason);
        let out = workspace_for_test(&files, "Old", "a.rs", "struct Old", true).unwrap();
        assert!(out.per_file.is_empty(), "got: {:?}", out.per_file);
        assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
        assert!(out.skipped[0].contains("b.rs"), "got: {:?}", out.skipped);
    }

    #[test]
    fn workspace_opaque_site_in_other_file_vetoes_then_skips() {
        let files = [
            ("a.rs", "pub struct Old;\n"),
            (
                "b.rs",
                "fn f() {\n    m!(Old);\n}\nfn g(x: Old) -> Old {\n    x\n}\n",
            ),
        ];
        let err = workspace_for_test(&files, "Old", "a.rs", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("opaque"), "got: {}", err.reason);
        assert!(err.reason.contains("b.rs"), "got: {}", err.reason);
        let out = workspace_for_test(&files, "Old", "a.rs", "struct Old", true).unwrap();
        let b = out
            .per_file
            .iter()
            .find(|(p, _)| p == "b.rs")
            .expect("b.rs entry");
        assert_eq!(b.1.len(), 2, "per_file: {:?}", out.per_file);
        assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
        assert!(out.skipped[0].contains("b.rs"), "got: {:?}", out.skipped);
    }

    #[test]
    fn glob_with_old_use_vetoes_without_fallback() {
        let src = "use other::*;\nstruct Old;\nfn f(x: Old) -> Old {\n    x\n}\n";
        let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("glob"), "got: {}", err.reason);
    }

    #[test]
    fn glob_with_old_use_skips_only_that_file_with_fallback() {
        let files = [
            ("a.rs", "pub struct Old;\n"),
            ("b.rs", "use other::*;\nfn f(x: Old) -> Old {\n    x\n}\n"),
            ("c.rs", "fn g(x: Old) -> Old {\n    x\n}\n"),
        ];
        let err = workspace_for_test(&files, "Old", "a.rs", "struct Old", false).unwrap_err();
        assert!(err.reason.contains("glob"), "got: {}", err.reason);
        assert!(err.reason.contains("b.rs"), "got: {}", err.reason);
        let out = workspace_for_test(&files, "Old", "a.rs", "struct Old", true).unwrap();
        assert!(
            out.per_file.iter().all(|(p, _)| p != "b.rs"),
            "globbed file must not rewrite: {:?}",
            out.per_file
        );
        let c = out
            .per_file
            .iter()
            .find(|(p, _)| p == "c.rs")
            .expect("clean file still rewrites");
        assert_eq!(c.1.len(), 2, "per_file: {:?}", out.per_file);
        assert!(
            out.skipped.iter().any(|s| s.contains("b.rs")),
            "got: {:?}",
            out.skipped
        );
    }

    #[test]
    fn glob_without_old_use_is_inert() {
        let src = "use other::*;\nstruct Old;\nfn f(x: i32) -> i32 {\n    x\n}\n";
        let out = resolve_for_test(src, "Old", "struct Old", false).unwrap();
        assert!(out.spans.is_empty(), "spans: {:?}", out.spans);
        assert!(out.skipped.is_empty(), "got: {:?}", out.skipped);
    }
}
