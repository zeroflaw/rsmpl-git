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
//! Precision-first (SPEC 2.6, D3), mirroring the scalar posture:
//! unique-definition workspace rule — a second same-named member
//! definition anywhere vetoes loud without `# fallback=syntax` (every
//! remaining use is ambiguous), or clears all use rewriting with
//! per-site skip notes when fallback is set. Anything the syntactic
//! walk cannot prove (bare `Old` that may be import-spelled, `qself`
//! paths, macro interiors mentioning the member name) is opaque: veto
//! without fallback, skip-with-report with it. Method idents and tuple
//! `.0` accesses never match and never veto.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::lines::LineIndex;
use crate::matcher_item::span_bytes;
use crate::use_graph::FollowVeto;

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
    /// `path:line`-qualified notes for sites left untouched under fallback.
    pub skipped: Vec<String>,
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
        line: u32,
        container: String,
        a: usize,
        b: usize,
    }
    let mut def_hits: Vec<DefHit> = Vec::new();
    for (path, parsed, original, index) in files {
        let mut collector = MemberCollector::new(kind, container, old);
        collector.visit_file(parsed);
        for (span, def_container) in &collector.defs {
            let line = span.start().line as u32;
            match span_bytes(original, index, *span) {
                Some((a, b)) => def_hits.push(DefHit {
                    path: path.clone(),
                    line,
                    container: def_container.clone(),
                    a,
                    b,
                }),
                None => {
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

    // Map + verify use spans per file.
    let mut per_file: Vec<(String, Vec<MemberEdit>)> = Vec::new();
    let mut opaque: Vec<(String, u32, String)> = Vec::new();
    for (path, parsed, original, index) in files {
        let mut collector = MemberCollector::new(kind, container, old);
        collector.visit_file(parsed);
        for (line, ctx) in &collector.opaque {
            opaque.push((path.clone(), *line, ctx.clone()));
        }
        // Bare-`Old` (possibly import-spelled) variant mentions are
        // unprovable: never edits, opaque notes like macro interiors.
        if kind == MemberKind::Variant {
            for span in &collector.bare {
                let line = span.start().line as u32;
                opaque.push((path.clone(), line, "a bare `Old` spelling".to_string()));
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
            per_file.push((path.clone(), edits));
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
                    "shadowed definition of `{old}` ({}::{}) at {}:{}: cannot tell which definition each use names ({use_sites} use sites workspace-wide)",
                    d.container, old, d.path, d.line,
                ),
            });
        }
        for d in &shadow {
            skipped.push(format!(
                "{}:{}: shadowed definition of `{old}` ({}::{old}); {use_sites} use sites left untouched",
                d.path, d.line, d.container,
            ));
        }
        per_file.clear();
    }
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
            skipped.push(format!(
                "{path}:{line}: use of `{old}` inside {ctx} left untouched (opaque)"
            ));
        }
        if shadow.is_empty() {
            // Opaque sites were never collected as edits; provable sites
            // still rewrite. The notes are the report.
        }
    }
    Ok(MemberWorkspaceOutcome { per_file, skipped })
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
    for (path, parsed, original, index) in files {
        let mut collector = MemberCollector::new(kind, container, old);
        collector.visit_file(parsed);
        let mut lines: Vec<u32> = Vec::new();
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
            lines.push(line);
        }
        if kind == MemberKind::Variant {
            for span in &collector.bare {
                lines.push(span.start().line as u32);
            }
        }
        for (line, _) in &collector.opaque {
            lines.push(*line);
        }
        lines.sort_unstable();
        lines.dedup();
        for line in lines {
            out.push(crate::OrphanReport {
                file: (*path).clone(),
                line,
                text: source_line_text(original, index, line),
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
        });
    }

    fn push_plain_use(&mut self, span: proc_macro2::Span) {
        let text = self.old.to_string();
        self.push_use(span, text, false);
    }
}

/// Whole-ident mention test for macro interiors (mirrors the scalar
/// `raw_mentions` posture: any mention is opaque, never attributed).
fn tokens_mention(tokens: &proc_macro2::TokenStream, old: &str) -> bool {
    tokens.clone().into_iter().any(|t| match t {
        proc_macro2::TokenTree::Ident(id) => id == old,
        proc_macro2::TokenTree::Group(g) => tokens_mention(&g.stream(), old),
        _ => false,
    })
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
                    self.push_plain_use(ident.span());
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
        if self.kind == MemberKind::Variant
            && node.qself.is_some()
            && node.path.segments.iter().any(|s| s.ident == self.old)
        {
            self.opaque.push((
                node.path.span().start().line as u32,
                "a qself path".to_string(),
            ));
            return;
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if self.kind == MemberKind::Variant
            && node.qself.is_some()
            && node.path.segments.iter().any(|s| s.ident == self.old)
        {
            self.opaque.push((
                node.path.span().start().line as u32,
                "a qself path".to_string(),
            ));
            return;
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
        if tokens_mention(&node.tokens, self.old) {
            self.opaque.push((
                node.span().start().line as u32,
                "a macro invocation".to_string(),
            ));
        }
    }

    fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
        // Attribute paths cannot spell field accesses or `E::Old`
        // variant paths in valid Rust; unlike scalar renames there is no
        // opacity class here — do not visit interiors either (macros in
        // attributes stay uncovered by design, same as scalar attribute
        // handling which only records the path itself).
        let _ = node;
    }
}
