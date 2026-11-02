//! Non-function item renames (SPEC 14.8, T12).
//!
//! Rename-only rewrite for `struct`, `enum`, `trait`, `union`, `type`,
//! `const`, `static`, `mod` and `impl` self-ty: one bare
//! `<kind> <Old>` / `<kind> <New>` line pair replaces a single name-ident
//! span per match. Visibility, generics, bodies, attributes and formatting
//! are preserved byte-for-byte, exactly like the fn rename fast-path.
//! `impl` renames the self-ty ident and the trait-side path's last-segment
//! ident (T19b): in valid Rust one name cannot be both a type and a trait in
//! the same scope, so a bare `impl Old` pair targets whichever side(s) name it.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, ItemKind, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// One rename candidate with original byte range + line span.
struct ItemCandidate {
    name: String,
    ident: proc_macro2::Span,
    attrs: Vec<syn::Attribute>,
    /// Canonical rendered visibility (`""` when inherited or
    /// for kinds without visibility; `pub(in …)` keeps its space).
    vis_text: String,
    /// Start-bound and end-bound spans of the `pub…` run (`None` when
    /// inherited or for kinds without visibility).
    vis_span: Option<(proc_macro2::Span, proc_macro2::Span)>,
    start_line: u32,
    end_line: u32,
    anchor_line: u32,
    attr_start_line: u32,
}

fn push_candidate(
    out: &mut Vec<ItemCandidate>,
    name: String,
    ident: proc_macro2::Span,
    attrs: Vec<syn::Attribute>,
    vis: Option<&syn::Visibility>,
    item_span: proc_macro2::Span,
) {
    let (vis_text, vis_span) = match vis {
        Some(v) => (norm_vis_text(v), vis_line_span(v)),
        None => (String::new(), None),
    };
    out.push(ItemCandidate {
        name,
        attr_start_line: item_span.start().line as u32,
        start_line: item_span.start().line as u32,
        end_line: item_span.end().line as u32,
        anchor_line: ident.start().line as u32,
        ident,
        attrs,
        vis_text,
        vis_span,
    });
}

/// Whitespace-insensitive rendered visibility (`""` when inherited),
/// canonicalized so the `pub(in …)` space survives (`pub(in crate)`,
/// never the unparseable `pub(incrate)` the engine emits verbatim).
fn norm_vis_text(vis: &syn::Visibility) -> String {
    match vis {
        syn::Visibility::Inherited => String::new(),
        _ => rsmpl_pattern::canonical_vis_prefix(
            &quote::quote!(#vis)
                .to_string()
                .split_whitespace()
                .collect::<String>(),
        ),
    }
}

/// Start-bound and end-bound spans of the `pub…` run (`None` when
/// inherited). `syn::Visibility` has no `Spanned` impl, so the arms
/// below name the token spans directly.
fn vis_line_span(vis: &syn::Visibility) -> Option<(proc_macro2::Span, proc_macro2::Span)> {
    match vis {
        syn::Visibility::Inherited => None,
        syn::Visibility::Public(t) => Some((t.span, t.span)),
        syn::Visibility::Restricted(r) => {
            let end = r.paren_token.span.join();
            Some((r.pub_token.span, end))
        }
    }
}

/// Last-segment ident of a plain path type (`Old`, `Old<T>`, `crate::Old`).
/// Anything else (references, tuples, `dyn`, ...) is not a rename target.
fn self_ty_ident(ty: &syn::Type) -> Option<&syn::Ident> {
    if let syn::Type::Path(tp) = ty {
        if tp.qself.is_none() {
            return tp.path.segments.last().map(|s| &s.ident);
        }
    }
    None
}

/// Collect rename candidates of `want` kind from file items, inline
/// modules, and block-level items (T23: fn bodies, closures, method
/// bodies — anywhere the syntactic walk reaches; macro bodies stay
/// excluded per T133). Each item node is visited once; the push logic
/// is unchanged, only the traversal widened.
fn collect(items: &[syn::Item], want: ItemKind, out: &mut Vec<ItemCandidate>) {
    struct Collector {
        want: ItemKind,
        found: Vec<ItemCandidate>,
    }
    impl<'ast> Visit<'ast> for Collector {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            consider(it, self.want, &mut self.found);
            visit::visit_item(self, it);
        }
    }
    let mut collector = Collector {
        want,
        found: Vec::new(),
    };
    for it in items {
        collector.visit_item(it);
    }
    out.extend(collector.found);
}

/// Push `it` as a rename candidate when it has the wanted kind.
/// Traversal (inline mods, bodies) is the caller's `Visit` default.
fn consider(it: &syn::Item, want: ItemKind, out: &mut Vec<ItemCandidate>) {
    match it {
        syn::Item::Struct(s) if want == ItemKind::Struct => push_candidate(
            out,
            s.ident.to_string(),
            s.ident.span(),
            s.attrs.clone(),
            Some(&s.vis),
            s.span(),
        ),
        syn::Item::Enum(e) if want == ItemKind::Enum => push_candidate(
            out,
            e.ident.to_string(),
            e.ident.span(),
            e.attrs.clone(),
            Some(&e.vis),
            e.span(),
        ),
        syn::Item::Trait(t) if want == ItemKind::Trait => push_candidate(
            out,
            t.ident.to_string(),
            t.ident.span(),
            t.attrs.clone(),
            Some(&t.vis),
            t.span(),
        ),
        syn::Item::Union(u) if want == ItemKind::Union => push_candidate(
            out,
            u.ident.to_string(),
            u.ident.span(),
            u.attrs.clone(),
            Some(&u.vis),
            u.span(),
        ),
        syn::Item::Type(t) if want == ItemKind::Type => push_candidate(
            out,
            t.ident.to_string(),
            t.ident.span(),
            t.attrs.clone(),
            Some(&t.vis),
            t.span(),
        ),
        syn::Item::Const(c) if want == ItemKind::Const => push_candidate(
            out,
            c.ident.to_string(),
            c.ident.span(),
            c.attrs.clone(),
            Some(&c.vis),
            c.span(),
        ),
        syn::Item::Static(s) if want == ItemKind::Static => push_candidate(
            out,
            s.ident.to_string(),
            s.ident.span(),
            s.attrs.clone(),
            Some(&s.vis),
            s.span(),
        ),
        syn::Item::Mod(m) if want == ItemKind::Mod => push_candidate(
            out,
            m.ident.to_string(),
            m.ident.span(),
            m.attrs.clone(),
            Some(&m.vis),
            m.span(),
        ),
        syn::Item::Macro(m) if want == ItemKind::MacroRules => {
            // `macro_rules! Old { ... }`: the definition name is the
            // `ident` (not the `macro_rules` path); attribute-style
            // macros (`my_attr!{...}` in item position) have other
            // paths and never match (WI-7b sibling predicate).
            if !super::use_graph_macro::is_macro_rules_path(&m.mac.path) {
                return;
            }
            if let Some(ident) = m.ident.as_ref() {
                push_candidate(
                    out,
                    ident.to_string(),
                    ident.span(),
                    m.attrs.clone(),
                    None,
                    m.span(),
                );
            }
        }
        syn::Item::Impl(imp) if want == ItemKind::Impl => {
            if let Some(ident) = self_ty_ident(&imp.self_ty) {
                push_candidate(
                    out,
                    ident.to_string(),
                    ident.span(),
                    imp.attrs.clone(),
                    None,
                    imp.span(),
                );
            }
            // T19b: trait-side path (`impl Old for Foo`). Match the last
            // segment ident so renaming a trait updates its impl headers;
            // generics/qualifiers on the path are preserved. The trait
            // position is a plain `syn::Path` (no qualified-self form
            // exists there), so no qself guard is needed.
            if let Some((_, path, _)) = &imp.trait_ {
                if let Some(seg) = path.segments.last() {
                    let ident = &seg.ident;
                    push_candidate(
                        out,
                        ident.to_string(),
                        ident.span(),
                        imp.attrs.clone(),
                        None,
                        imp.span(),
                    );
                }
            }
        }
        _ => {}
    }
}

/// Scope filtering (SPEC 17.3). `anchor` uses the ident line as the first
/// concrete non-ellipsis token; other modes use the attr-envelope span.
fn in_scope(c: &ItemCandidate, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.start_line >= old_start && c.end_line <= end.max(old_start),
        Scope::Anchor => c.anchor_line >= old_start && c.anchor_line <= end.max(old_start),
        Scope::Overlap => c.start_line <= end.max(old_start) && c.end_line >= old_start,
        Scope::Exact => old_len > 0 && c.start_line == old_start && c.end_line == end,
        Scope::File => true,
    }
}

/// Whitespace-insensitive attribute comparison shared by the Explicit
/// arm, the Preserve site filter, and the plus/minus mirror gate
/// (T235): pattern text (token-joined at parse, `///` normalized to
/// `#[doc]`) vs `quote!`-rendered target attributes or minus-side text.
fn norm_attr_text(s: &str) -> String {
    s.split_whitespace().collect()
}

pub(crate) fn attrs_equal(pattern_attrs: &[String], attrs: &[syn::Attribute]) -> bool {
    let want: Vec<String> = pattern_attrs.iter().map(|a| norm_attr_text(a)).collect();
    let have: Vec<String> = attrs
        .iter()
        .map(|a| norm_attr_text(&quote::quote!(#a).to_string()))
        .collect();
    want == have
}

fn attrs_text_equal(a: &[String], b: &[String]) -> bool {
    let norm = |ss: &[String]| ss.iter().map(|s| norm_attr_text(s)).collect::<Vec<_>>();
    norm(a) == norm(b)
}

// WI-10a helpers live in `matcher_attr_delta` (D5 sibling, no second copy).
use crate::matcher_attr_delta::{
    attr_delta as item_attr_delta, compact_attr, norm_pat_list, site_attr_texts,
    subsequence_positions as subsequence_indices, AttrDelta as ItemAttrDelta,
};

pub(crate) fn check_attrs(
    attrs: &[syn::Attribute],
    policy: AttrsPolicy,
    pattern_attrs: &[String],
    rule: &str,
    file: &str,
) -> Result<(), StructuredError> {
    let has = !attrs.is_empty();
    match policy {
        AttrsPolicy::Preserve => Ok(()),
        AttrsPolicy::Absent => {
            if has {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "item has attributes but `# attrs=absent` requires none",
                )
                .with_rule(rule)
                .with_file(file)
                .with_repair(StructuredError::fix_pattern_repair(
                    "attrs",
                    "remove `# attrs=absent` to ignore attributes (the default), or write the attributes into the pattern",
                )));
            }
            Ok(())
        }
        AttrsPolicy::Explicit => {
            if !attrs_equal(pattern_attrs, attrs) {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "attached attributes do not match explicitly written pattern attributes",
                )
                .with_rule(rule)
                .with_file(file)
                .with_suggestion("use `# attrs=preserve` (default) to ignore attributes")
                .with_repair(StructuredError::add_directive_repair("# attrs=preserve")));
            }
            Ok(())
        }
        AttrsPolicy::Strip => Ok(()),
    }
}

/// Byte range of `span` in the original text, or `None` outside the file.
pub(crate) fn span_bytes(
    original: &str,
    index: &LineIndex,
    span: proc_macro2::Span,
) -> Option<(usize, usize)> {
    let s = span.start();
    let e = span.end();
    let a = index.offset(s.line as u32, s.column)?;
    let b = index.offset(e.line as u32, e.column)?;
    if a <= b && b <= original.len() {
        Some((a, b))
    } else {
        None
    }
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_repair(StructuredError::internal_repair())
}

/// Apply an item-rename hunk. Returns the number of matches rewritten.
pub fn apply_item_rename_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (kind, minus_name, plus_name, minus_attrs, minus_prefix, plus_prefix) = match &compiled.kind
    {
        PatternKind::ItemRename {
            kind,
            minus_name,
            plus_name,
            minus_attrs,
            minus_prefix,
            plus_prefix,
            ..
        } => (
            *kind,
            minus_name.clone(),
            plus_name.clone(),
            minus_attrs.clone(),
            minus_prefix.clone(),
            plus_prefix.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected item rename pattern".to_string(),
            ));
        }
    };
    // WI-5-R2: a vis-evolving pair rewrites the visibility slot alongside
    // the name; the pure-rename path (equal prefixes) is unchanged.
    let evolving = minus_prefix != plus_prefix;
    // A `+` side that writes attributes the `-` side omitted is ambiguous
    // (same rule as General stmts): reject instead of guessing. WI-10a:
    // under Preserve an add (minus empty) is the delta, not ambiguity.
    let (plus_attrs_owned, delta_active, minus_norm, plus_norm, delta) =
        if let PatternKind::ItemRename { plus_attrs, .. } = &compiled.kind {
            let preserve = hunk.attrs_policy() == AttrsPolicy::Preserve;
            let mn = norm_pat_list(&minus_attrs);
            let pn = norm_pat_list(plus_attrs);
            let d = item_attr_delta(&minus_attrs, plus_attrs);
            let ambiguous_len = preserve
                && mn != pn
                && !minus_attrs.is_empty()
                && !plus_attrs.is_empty()
                && minus_attrs.len() != plus_attrs.len();
            let active = preserve && !ambiguous_len && d != ItemAttrDelta::Mirror;
            (plus_attrs.clone(), active, mn, pn, d)
        } else {
            (
                Vec::new(),
                false,
                Vec::new(),
                Vec::new(),
                ItemAttrDelta::Mirror,
            )
        };
    if !delta_active {
        if let PatternKind::ItemRename { plus_attrs, .. } = &compiled.kind {
            if !plus_attrs.is_empty() && minus_attrs.is_empty() {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "plus side writes attributes the minus side omitted (ambiguous merge); write the attributes on both sides or neither",
                )
                .with_rule(hunk.rule_name())
                .with_repair(StructuredError::fix_pattern_repair(
                    "C11",
                    "write the attributes on both sides or neither",
                )));
            }
        }
    }
    // Plus-side attribute text must mirror the minus side (same contract
    // as fn/general hunks, T235): the item rewrite only replaces the
    // ident span, so differing plus-side attrs would be silently dropped.
    // `# attrs=strip` skips this gate — the envelope is deleted anyway.
    // WI-10a: Preserve delta hunks skip it (attr edits carry the change).
    if !delta_active {
        if let PatternKind::ItemRename { plus_attrs, .. } = &compiled.kind {
            if hunk.attrs_policy() != AttrsPolicy::Strip
                && !minus_attrs.is_empty()
                && !attrs_text_equal(&minus_attrs, plus_attrs)
            {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
                )
                .with_rule(hunk.rule_name())
                .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`")
                .with_repair(StructuredError::fix_pattern_repair(
                    "C12",
                    "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
                )));
            }
        }
    }
    let rule = hunk.rule_name();
    let file = "";
    let mut candidates = Vec::new();
    collect(&parsed.items, kind, &mut candidates);
    // Deterministic source order for `allow=many`.
    candidates.sort_by_key(|c| (c.start_line, c.anchor_line));

    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    // WI-10b: a `$`-sigiled name is a name hole (the classifier hole-ified
    // a bare diet/`# ident` name): it binds every site's name and replays
    // it byte-identical (attr-only evolution; the classifier rejects
    // hole/concrete mixes, so the plus side is a hole too). `$` never
    // occurs in concrete item names (`parse_side` rejects it), so the
    // sigil alone distinguishes holes — no table registration needed.
    let name_hole = minus_name.starts_with('$');
    let mut matched = 0;
    for c in &candidates {
        if name_hole {
            // Holes bind per site — no name filter (subset attr semantics
            // below still gate each site).
        } else if c.name != minus_name {
            continue;
        }
        if !in_scope(c, hunk.scope(), old_start, old_len) {
            continue;
        }
        // WI-5-R2: an evolving vis pair only matches targets carrying the
        // minus vis (whitespace-stripped render); anything else is a
        // different spelling -- skipped, never rewritten.
        if evolving && c.vis_text != minus_prefix {
            continue;
        }
        // Preserve plus written minus-side attributes constrains the match
        // (T235, SPEC 14.6 — same as fn hunks): sites carrying different
        // attributes are skipped, never silently rewritten. WI-10a: delta
        // hunks use subset semantics (add skips already-plus; remove/
        // replace require the minus lines, preserving the rest).
        if delta_active {
            let have = site_attr_texts(&c.attrs);
            match delta {
                ItemAttrDelta::Add => {
                    if subsequence_indices(&have, &plus_norm).is_some() {
                        continue;
                    }
                }
                ItemAttrDelta::Remove | ItemAttrDelta::Replace => {
                    if subsequence_indices(&have, &minus_norm).is_none() {
                        continue;
                    }
                }
                ItemAttrDelta::Mirror => {}
            }
        } else if hunk.attrs_policy() == AttrsPolicy::Preserve
            && !minus_attrs.is_empty()
            && !attrs_equal(&minus_attrs, &c.attrs)
        {
            continue;
        }
        check_attrs(&c.attrs, hunk.attrs_policy(), &minus_attrs, &rule, file)?;
        // WI-10a: same-name evolution still emits the (byte-identical) name
        // edit so reported spans overlap the ident and the WI-1.4
        // span-local postcondition verifies (WI-5 conforming-site
        // precedent); the attr edits below carry the visible change.
        // WI-10b: holes replay the bound site name (byte-identical).
        {
            let (a, b) = span_bytes(original, index, c.ident)
                .ok_or_else(|| range_err(&rule, c.ident.start().line as u32))?;
            let want_name = if name_hole { &c.name } else { &minus_name };
            if original.get(a..b) != Some(want_name.as_str()) {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "item name span mismatch (file changed during matching?)",
                )
                .with_rule(&rule)
                .with_repair(StructuredError::internal_repair()));
            }
            let replay = if name_hole {
                c.name.clone()
            } else {
                plus_name.clone()
            };
            edits.push(TextEdit::new(a, b, replay).with_label("rename"));
        }
        // WI-5-R2: concrete vis evolution rides the rename -- insert (no
        // target vis), delete (bare plus vis), or replace. Insertion lands
        // after the item-line indent so nested items keep their indent.
        if evolving {
            if let Some((vs, ve)) = c.vis_span {
                let (va, vb) = {
                    let a = index
                        .offset(vs.start().line as u32, vs.start().column)
                        .ok_or_else(|| range_err(&rule, vs.start().line as u32))?;
                    let b = index
                        .offset(ve.end().line as u32, ve.end().column)
                        .ok_or_else(|| range_err(&rule, ve.end().line as u32))?;
                    (a, b)
                };
                if plus_prefix.is_empty() {
                    let mut e = vb;
                    while original
                        .as_bytes()
                        .get(e)
                        .is_some_and(|ch| *ch == b' ' || *ch == b'\t')
                    {
                        e += 1;
                    }
                    edits.push(TextEdit::new(va, e, String::new()));
                } else if original.get(va..vb).is_some_and(|got| {
                    // Canonical compare: a `pub(in crate)` site must equal
                    // a `pub(in crate)` plus (stripped both sides they
                    // would read `pub(incrate)` vs `pub(in crate)` and a
                    // spurious byte-identical edit would be pushed).
                    rsmpl_pattern::canonical_vis_prefix(&got.split_whitespace().collect::<String>())
                        != plus_prefix
                }) {
                    edits.push(TextEdit::new(va, vb, plus_prefix.clone()));
                }
            } else if !plus_prefix.is_empty() {
                let line_at = index
                    .line_start(c.anchor_line)
                    .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                let indent: usize = original[line_at..]
                    .chars()
                    .take_while(|ch| *ch == ' ' || *ch == '\t')
                    .map(|ch| ch.len_utf8())
                    .sum();
                edits.push(TextEdit::new(
                    line_at + indent,
                    line_at + indent,
                    format!("{plus_prefix} "),
                ));
            }
        }
        // WI-10a: attr-delta edits (Preserve only). Add inserts above the
        // item line; remove deletes the matched lines; replace swaps each
        // matched line in place (site indent preserved).
        if delta_active {
            let have = site_attr_texts(&c.attrs);
            match delta {
                ItemAttrDelta::Add => {
                    let anchor_start = index
                        .line_start(c.anchor_line)
                        .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                    let anchor_line_text = &original[anchor_start..];
                    let anchor_line_text = anchor_line_text.split('\n').next().unwrap_or("");
                    let indent: String = anchor_line_text
                        .chars()
                        .take_while(|ch| *ch == ' ' || *ch == '\t')
                        .collect();
                    let mut text = String::new();
                    for p in &plus_attrs_owned {
                        text.push_str(&indent);
                        text.push_str(&compact_attr(p.trim()));
                        text.push('\n');
                    }
                    let at = index
                        .line_start(c.anchor_line)
                        .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                    edits.push(TextEdit::new(at, at, text));
                }
                ItemAttrDelta::Remove => {
                    if let Some(idxs) = subsequence_indices(&have, &minus_norm) {
                        for i in idxs {
                            let attr = &c.attrs[i];
                            let s = attr.span().start().line as u32;
                            let e = attr.span().end().line as u32;
                            let a = index.line_start(s).ok_or_else(|| range_err(&rule, s))?;
                            let b = index.line_start(e + 1).unwrap_or(original.len());
                            if b > a {
                                edits.push(TextEdit::new(a, b, String::new()));
                            }
                        }
                    }
                }
                ItemAttrDelta::Replace => {
                    if let Some(idxs) = subsequence_indices(&have, &minus_norm) {
                        for (k, i) in idxs.iter().enumerate() {
                            let Some(plus_line) = plus_attrs_owned.get(k) else {
                                continue;
                            };
                            let attr = &c.attrs[*i];
                            let s = attr.span().start().line as u32;
                            let e = attr.span().end().line as u32;
                            let a = index.line_start(s).ok_or_else(|| range_err(&rule, s))?;
                            let b = index.line_start(e + 1).unwrap_or(original.len());
                            let cur_start =
                                index.line_start(s).ok_or_else(|| range_err(&rule, s))?;
                            let cur_line = &original[cur_start..];
                            let cur_line = cur_line.split('\n').next().unwrap_or("");
                            let indent: String = cur_line
                                .chars()
                                .take_while(|ch| *ch == ' ' || *ch == '\t')
                                .collect();
                            let mut text = String::new();
                            text.push_str(&indent);
                            text.push_str(&compact_attr(plus_line.trim()));
                            text.push('\n');
                            if b > a {
                                edits.push(TextEdit::new(a, b, text));
                            }
                        }
                    }
                }
                ItemAttrDelta::Mirror => {}
            }
        }
        // `# attrs=strip`: delete attribute lines above the item.
        if hunk.attrs_policy() == AttrsPolicy::Strip && !c.attrs.is_empty() {
            let start = index
                .line_start(c.attr_start_line)
                .ok_or_else(|| range_err(&rule, c.attr_start_line))?;
            let end = index
                .line_start(c.anchor_line)
                .ok_or_else(|| range_err(&rule, c.anchor_line))?;
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

    fn rename_patch(minus: &str, plus: &str, range: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=rename\n# lang=rust\n- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn renames_struct_preserving_body_vis_generics_and_attrs() {
        let patch = rename_patch("struct Old", "struct New", "@@ -1,4 +1,4 @@");
        let src = "#[derive(Debug)]\npub struct Old<T> {\n    x: T,\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("pub struct New<T>"), "got: {out}");
        assert!(out.contains("#[derive(Debug)]"));
        assert!(out.contains("x: T,"));
        assert!(!out.contains("Old"));
    }

    #[test]
    fn explicit_doc_comment_mirrors_apply() {
        // T156: item parity with the T155 fn/general normalization —
        // `///` and single-line `/** */` mirrors compare like `#[doc]`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# attrs=explicit\n- /// Doc.\n- struct Old\n+ /// Doc.\n+ struct New\n";
        let out = apply(patch, "/// Doc.\nstruct Old {\n    x: i32,\n}\n").unwrap();
        assert!(out.contains("struct New {"), "got: {out}");
        assert!(out.contains("/// Doc."), "got: {out}");
        let block = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# attrs=explicit\n- /** Block doc. */\n- struct Old\n+ /** Block doc. */\n+ struct New\n";
        let out = apply(block, "/** Block doc. */\nstruct Old {\n    x: i32,\n}\n").unwrap();
        assert!(out.contains("struct New {"), "got: {out}");
        assert!(out.contains("/** Block doc. */"), "got: {out}");
        // Wrong doc text still fails instead of silently matching.
        let wrong = patch.replace("- /// Doc.", "- /// Other.");
        assert!(apply(&wrong, "/// Doc.\nstruct Old {\n    x: i32,\n}\n").is_err());
    }

    #[test]
    fn item_name_hole_evolves_attrs_across_hetero_names() {
        // WI-10b: a bare `N` in item name position binds per site and
        // replays byte-identical (attr-only evolution); vis/generics ride
        // along preserved like concrete renames.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# sweep\n- struct N\n+ #[derive(Debug)]\n+ struct N\n";
        let src = "struct A {\n    x: i32,\n}\npub struct B<T> {\n    y: T,\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(
            out,
            "#[derive(Debug)]\nstruct A {\n    x: i32,\n}\n#[derive(Debug)]\npub struct B<T> {\n    y: T,\n}\n",
            "got: {out:?}"
        );
        // Already-plus sites skip (no duplicate attr).
        assert!(apply(patch, "#[derive(Debug)]\nstruct A {}\n").is_err());
        // Mixed hole/concrete pairs fail fast (never a mass rename).
        let mixed = patch.replace("+ #[derive(Debug)]\n+ struct N", "+ struct New");
        let err = apply(&mixed, src).unwrap_err();
        assert!(
            err.contains("holes must appear on both sides"),
            "got: {err}"
        );
        // No-delta holes stay the rename-usage error.
        let nodelta = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# sweep\n- struct N\n+ struct N\n";
        let err = apply(nodelta, src).unwrap_err();
        assert!(err.contains("identical"), "got: {err}");
        // Declared `# ident` holes work the same way.
        let declared = patch.replace("- struct N", "- struct Whatever").replace(
            "+ #[derive(Debug)]\n+ struct N",
            "+ #[derive(Debug)]\n+ struct Whatever",
        );
        let declared = declared.replace("# sweep", "# sweep\n# ident Whatever");
        let out = apply(&declared, src).unwrap();
        assert!(out.contains("#[derive(Debug)]\nstruct A {"), "got: {out:?}");
        assert!(
            out.contains("#[derive(Debug)]\npub struct B<T> {"),
            "got: {out:?}"
        );
    }

    #[test]
    fn renames_each_supported_kind() {
        let cases = [
            (
                "enum Old",
                "enum New",
                "enum Old {\n    A,\n}\n",
                "enum New {",
                "@@ -1,3 +1,3 @@",
            ),
            (
                "trait Old",
                "trait New",
                "trait Old {\n    fn f(&self);\n}\n",
                "trait New {",
                "@@ -1,3 +1,3 @@",
            ),
            (
                "type Old",
                "type New",
                "type Old = Vec<u8>;\n",
                "type New = Vec<u8>;",
                "@@ -1,1 +1,1 @@",
            ),
            (
                "const Old",
                "const New",
                "const Old: usize = 10;\n",
                "const New: usize = 10;",
                "@@ -1,1 +1,1 @@",
            ),
            (
                "static Old",
                "static New",
                "static Old: u8 = 1;\n",
                "static New: u8 = 1;",
                "@@ -1,1 +1,1 @@",
            ),
            (
                "static mut Old",
                "static mut New",
                "static mut Old: u8 = 1;\n",
                "static mut New: u8 = 1;",
                "@@ -1,1 +1,1 @@",
            ),
            (
                "union Old",
                "union New",
                "union Old {\n    a: u8,\n}\n",
                "union New {",
                "@@ -1,3 +1,3 @@",
            ),
            (
                "mod old",
                "mod new",
                "mod old {}\n",
                "mod new {}",
                "@@ -1,1 +1,1 @@",
            ),
        ];
        for (minus, plus, src, want, range) in cases {
            let patch = rename_patch(minus, plus, range);
            let out = apply(&patch, src).unwrap();
            assert!(out.contains(want), "case {minus}: got: {out}");
        }
    }

    #[test]
    fn renames_impl_self_ty_only() {
        let patch = rename_patch("impl Old", "impl New", "@@ -1,4 +1,4 @@");
        let src = "struct Old;\nimpl Old {\n    fn f(&self) {}\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("impl New {"), "got: {out}");
        // Generic self-ty keeps its arguments.
        let patch2 = rename_patch("impl Old", "impl New", "@@ -2,3 +2,3 @@");
        let src2 = "struct Old<T>(T);\nimpl<T> Old<T> {\n    fn f(&self) {}\n}\n";
        let out2 = apply(&patch2, src2).unwrap();
        assert!(out2.contains("impl<T> New<T>"), "got: {out2}");
        // `impl Trait for Type`: self-ty side matches.
        let patch3 = rename_patch("impl Old", "impl New", "@@ -2,3 +2,3 @@");
        let src3 = "struct Old;\nimpl Clone for Old {\n    fn clone(&self) -> Self { Old }\n}\n";
        let out3 = apply(&patch3, src3).unwrap();
        assert!(out3.contains("impl Clone for New"), "got: {out3}");
    }

    #[test]
    fn impl_trait_side_matches() {
        // T19b: `impl Old for Foo` names Old as the trait — a bare
        // `impl Old` rename rewrites the trait-side ident too.
        let patch = rename_patch("impl Old", "impl New", "@@ -1,3 +1,3 @@");
        let src = "trait Old {}\nstruct Foo;\nimpl Old for Foo {}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("impl New for Foo"), "got: {out}");
        assert!(!out.contains("impl Old for Foo"), "got: {out}");
    }

    #[test]
    fn impl_trait_side_path_shapes() {
        // Generic, qualified, and negative trait paths: only the last
        // segment ident is replaced; arguments/qualifiers preserved.
        let cases = [
            (
                "impl<T> Old<T> for Foo {}\nstruct Foo;\n",
                "impl<T> New<T> for Foo {}",
            ),
            (
                "impl crate::Old for Foo {}\nstruct Foo;\n",
                "impl crate::New for Foo {}",
            ),
            (
                "impl !Old for Foo {}\nstruct Foo;\n",
                "impl !New for Foo {}",
            ),
        ];
        for (src, want) in cases {
            let patch = rename_patch("impl Old", "impl New", "@@ -1,1 +1,1 @@");
            let out = apply(&patch, src).unwrap();
            assert!(out.contains(want), "src {src:?}: got: {out}");
        }
    }

    #[test]
    fn impl_both_sides_match_with_allow_many() {
        // `impl Old for Old` names Old twice: two ident-span edits.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# allow=many\n- impl Old\n+ impl New\n";
        let out = apply(patch, "impl Old for Old {}\n").unwrap();
        assert!(out.contains("impl New for New"), "got: {out}");
    }

    #[test]
    fn impl_unrelated_trait_side_untouched() {
        // Only the named trait is rewritten; other headers do not match.
        let patch = rename_patch("impl Old", "impl New", "@@ -1,3 +1,3 @@");
        let src = "trait Other {}\nstruct Foo;\nimpl Other for Foo {}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn allow_many_renames_every_match() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=many\n# lang=rust\n# allow=many\n- struct Old\n+ struct New\n";
        let src = "struct Old;\nstruct Old;\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out.matches("struct New").count(), 2, "got: {out}");
    }

    #[test]
    fn nested_mod_items_found() {
        let patch = rename_patch("struct Helper", "struct Aid", "@@ -2,1 +2,1 @@");
        let src = "mod outer {\n    pub struct Helper;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("pub struct Aid;"), "got: {out}");
        assert!(out.contains("mod outer {"));
    }

    #[test]
    fn name_mismatch_is_no_match() {
        let patch = rename_patch("struct Old", "struct New", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, "struct Other;\n").is_err());
    }

    #[test]
    fn kind_mismatch_rejected_at_compile() {
        let patch = rename_patch("struct Old", "enum New", "@@ -1,1 +1,1 @@");
        let err = apply(&patch, "struct Old;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }

    #[test]
    fn concrete_body_rejected() {
        let patch = rename_patch("struct Old { x: i32 }", "struct New", "@@ -1,3 +1,3 @@");
        let err = apply(&patch, "struct Old {\n    x: i32,\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }

    #[test]
    fn attrs_policies() {
        // preserve (default) keeps attributes.
        let patch = rename_patch("struct Old", "struct New", "@@ -1,2 +1,2 @@");
        let out = apply(&patch, "#[derive(Debug)]\nstruct Old;\n").unwrap();
        assert!(out.contains("#[derive(Debug)]\nstruct New;"), "got: {out}");
        // absent rejects attributed items.
        let absent1 = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# attrs=absent\n- struct Old\n+ struct New\n";
        let absent2 = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# attrs=absent\n- struct Old\n+ struct New\n";
        assert!(apply(absent1, "struct Old;\n").is_ok());
        assert!(apply(absent2, "#[derive(Debug)]\nstruct Old;\n").is_err());
        // strip deletes attribute lines.
        let strip = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# attrs=strip\n- struct Old\n+ struct New\n";
        let out = apply(strip, "#[derive(Debug)]\nstruct Old;\n").unwrap();
        assert!(!out.contains("derive"), "got: {out}");
        assert!(out.contains("struct New;"), "got: {out}");
    }

    #[test]
    fn scope_modes_gate_matches() {
        let patch = rename_patch("struct Old", "struct New", "@@ -1,1 +1,1 @@");
        // WI-4: terse hunk cites line 1 while the item lives on line 3 —
        // pattern-anchored drift applies it (uniqueness + disjointness).
        let src = "fn a() {}\nfn b() {}\nstruct Old;\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("struct New;"), "got: {out}");
    }

    #[test]
    fn does_not_shadow_fn_or_call_shapes() {
        // `const fn` still routes to the fn classifier (strict vis applies).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- const fn foo(...) {\n+ const fn bar(...) {\n      ...\n  }\n";
        let out = apply(patch, "const fn foo() {}\n").unwrap();
        assert!(out.contains("const fn bar()"), "got: {out}");
        // Ordinary calls still route to the call matcher.
        let call = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let out = apply(call, "fn a() {\n    foo(1);\n}\n").unwrap();
        assert!(out.contains("bar(1)"), "got: {out}");
    }

    #[test]
    fn renames_block_level_struct() {
        // T23: items nested in fn bodies are collected by the syntactic walk.
        let patch = rename_patch("struct Local", "struct Renamed", "@@ -2,1 @@");
        let src = "fn outer() {\n    struct Local;\n    let _ = 1;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("struct Renamed;"), "got: {out}");
        assert!(out.contains("fn outer() {"));
        assert!(out.contains("let _ = 1;"));
        assert!(!out.contains("Local"));
    }

    #[test]
    fn allow_many_reaches_block_and_top_level() {
        // T23: one hunk, `# allow=many`, renames the top-level item and the
        // same-named local together (traversal + allow semantics compose).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=many\n# allow=many\n- struct Old\n+ struct New\n";
        let src = "struct Old;\nfn outer() {\n    struct Old;\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out.matches("struct New").count(), 2, "got: {out}");
        assert!(!out.contains("Old"));
    }

    #[test]
    fn preserve_written_attrs_constrain_and_mirror() {
        // T235: written minus-side attrs constrain the match even under
        // the default preserve policy (SPEC 14.6, fn parity via T165);
        // sites carrying different attrs are skipped, never rewritten.
        let mismatch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- /// Local thing.\n- struct Local\n+ /// Local thing.\n+ struct Remote\n";
        let err = apply(mismatch, "struct Local;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        // Mirrored attrs on a matching site apply byte-exact.
        let mirror = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- /// Local thing.\n- struct Local\n+ /// Local thing.\n+ struct Remote\n";
        let out = apply(mirror, "/// Local thing.\nstruct Local;\n").unwrap();
        assert!(
            out.contains("/// Local thing.\nstruct Remote;"),
            "got: {out}"
        );
        // WI-10a: same-length differing attrs are the Preserve replace
        // delta (rename + attr replace land together), not C12. Note the
        // `///` pattern normalizes to `#[doc]` at parse (T155), so the
        // replacement lands in `#[doc]` spelling.
        let differ = mirror.replace("+ /// Local thing.", "+ /// Changed thing.");
        let out = apply(&differ, "/// Local thing.\nstruct Local;\n").unwrap();
        assert!(out.contains("struct Remote;"), "got: {out}");
        assert!(out.contains("Changed thing."), "got: {out}");
        assert!(!out.contains("Local thing."), "got: {out}");
        // Differing non-empty lengths stay C12 (ambiguous partial overlap).
        let ambiguous = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- #[a]\n- struct Local\n+ #[b]\n+ #[c]\n+ struct Remote\n";
        let err = apply(ambiguous, "#[a]\nstruct Local;\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        // `# attrs=strip` skips the differ gate: the envelope is deleted.
        let strip = differ.replace("# lang=rust", "# attrs=strip\n# lang=rust");
        let out = apply(&strip, "/// Local thing.\nstruct Local;\n").unwrap();
        assert!(out.contains("struct Remote;"), "got: {out}");
        assert!(!out.contains("Local thing"), "got: {out}");
        // WI-10a: pure attr removal (minus N, plus empty) is the remove
        // delta (rename + removal land together), not C12.
        let drop = mirror.replace("+ /// Local thing.\n", "");
        let out = apply(&drop, "/// Local thing.\nstruct Local;\n").unwrap();
        assert!(out.contains("struct Remote;"), "got: {out}");
        assert!(!out.contains("Local thing"), "got: {out}");
    }

    #[test]
    fn attr_delta_add_replace_remove_fire() {
        // WI-10a: same-name attr evolution (no rename) + sweep add.
        // Add cites the item line with file scope (sites may carry other
        // attrs above it, so a tight range would miss them).
        let add = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# sweep\n- struct S\n+ #[derive(Clone)]\n+ struct S\n";
        let out = apply(add, "struct S;\n").unwrap();
        assert_eq!(out, "#[derive(Clone)]\nstruct S;\n", "got: {out:?}");
        // Add preserves an unrelated existing attr.
        let out = apply(add, "#[derive(Debug)]\nstruct S;\n").unwrap();
        assert!(out.contains("#[derive(Debug)]"), "got: {out:?}");
        assert!(out.contains("#[derive(Clone)]"), "got: {out:?}");
        // Already-plus skips (no duplicate).
        assert!(apply(add, "#[derive(Clone)]\nstruct S;\n").is_err());
        // Same-name replace.
        let rep = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- #[derive(Debug)]\n- struct S\n+ #[derive(Debug, Clone)]\n+ struct S\n";
        let out = apply(rep, "#[derive(Debug)]\nstruct S;\n").unwrap();
        assert!(out.contains("#[derive(Debug, Clone)]"), "got: {out:?}");
        assert!(!out.contains("#[derive(Debug)]\nstruct"), "got: {out:?}");
        // Sweep add over two items.
        let two = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# sweep\n# allow=many\n- struct A\n+ #[inline]\n+ struct A\n";
        let src = "struct A;\nstruct B;\nmod m {\n    struct A;\n}\n";
        let out = apply(two, "struct A;\nstruct A;\n").unwrap();
        assert_eq!(out.matches("#[inline]").count(), 2, "got: {out:?}");
        let _ = src;
        // Same-name without attr delta still fails (rename-usage error kept).
        let same = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- struct S\n+ struct S\n";
        let err = apply(same, "struct S;\n").unwrap_err();
        assert!(err.contains("identical"), "got: {err}");
    }

    #[test]
    fn renames_macro_rules_preserving_body_and_attrs() {
        // WI-7b: `macro_rules!` definitions rename the ident only.
        let patch = rename_patch("macro_rules! Old", "macro_rules! New", "@@ -1,4 +1,4 @@");
        let src = "#[macro_export]\nmacro_rules! Old {\n    ($x:expr) => { $x + 1 };\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("macro_rules! New {"), "got: {out}");
        assert!(out.contains("#[macro_export]"));
        assert!(out.contains("($x:expr) => { $x + 1 };"));
        assert!(!out.contains("macro_rules! Old"));
    }

    #[test]
    fn macro_rules_distractor_untouched() {
        let patch = rename_patch("macro_rules! Old", "macro_rules! New", "@@ -1,4 +1,4 @@");
        let src = "macro_rules! Old {\n    () => {};\n}\nmacro_rules! Other {\n    () => {};\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("macro_rules! New {"), "got: {out}");
        assert!(out.contains("macro_rules! Other {"), "got: {out}");
    }

    #[test]
    fn vis_evolution_add_remove_change_apply() {
        // WI-5-R2: rename + one vis step rewrites the vis slot alongside
        // the name, preserving generics/bodies/attrs.
        let add = rename_patch("struct Old", "pub struct New", "@@ -1,1 +1,1 @@");
        let out = apply(&add, "struct Old;\n").unwrap();
        assert_eq!(out, "pub struct New;\n", "got: {out:?}");
        let narrow = rename_patch("pub(crate) enum Old", "enum New", "@@ -1,4 +1,4 @@");
        let src = "#[derive(Debug)]\npub(crate) enum Old {\n    A,\n}\n";
        let out = apply(&narrow, src).unwrap();
        assert!(out.contains("enum New {"), "got: {out}");
        assert!(out.contains("#[derive(Debug)]"), "got: {out}");
        assert!(!out.contains("pub(crate)"), "got: {out}");
        let change = rename_patch("pub struct Old", "pub(crate) struct New", "@@ -1,3 +1,3 @@");
        let out = apply(&change, "pub struct Old<T> {\n    x: T,\n}\n").unwrap();
        assert!(out.contains("pub(crate) struct New<T> {"), "got: {out}");
        assert!(out.contains("x: T,"), "got: {out}");
        assert!(!out.contains("Old"), "got: {out}");
    }

    #[test]
    fn vis_evolution_wrong_vis_skips() {
        // The evolving pair only matches targets carrying the minus vis.
        let patch = rename_patch("struct Old", "pub struct New", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, "pub struct Old;\n").is_err());
        let patch = rename_patch("pub struct Old", "pub(crate) struct New", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, "struct Old;\n").is_err());
        // Pure renames still ignore vis (preserved, never rewritten).
        let pure = rename_patch("struct Old", "struct New", "@@ -1,1 +1,1 @@");
        let out = apply(&pure, "pub struct Old;\n").unwrap();
        assert!(out.contains("pub struct New;"), "got: {out}");
    }

    #[test]
    fn vis_evolution_nested_keeps_indent() {
        // Insertion lands after the item-line indent.
        let patch = rename_patch("struct Old", "pub(crate) struct New", "@@ -2,1 +2,1 @@");
        let src = "mod outer {\n    struct Old;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("    pub(crate) struct New;"), "got: {out}");
    }

    #[test]
    fn pub_in_vis_round_trips_byte_identical() {
        // `pub(in path)` keeps its space end to end: the canonical
        // prefix emits valid Rust (the stripped `pub(incrate)` is
        // unparseable — loud PARSE_CONFLICT before this fix), the
        // minus gate matches spaced sites, and the replace guard
        // pushes no edit when the site already equals the plus.
        let add = rename_patch("struct Old", "pub(in crate) struct New", "@@ -1,1 +1,1 @@");
        let out = apply(&add, "struct Old;\n").unwrap();
        assert_eq!(out, "pub(in crate) struct New;\n", "got: {out:?}");
        let change = rename_patch(
            "pub struct Old",
            "pub(in crate) struct New",
            "@@ -1,1 +1,1 @@",
        );
        let out = apply(&change, "pub struct Old;\n").unwrap();
        assert_eq!(out, "pub(in crate) struct New;\n", "got: {out:?}");
        // A `pub(in crate)` site matches a `pub(in crate)` minus (the
        // canonical gate), and a pure rename preserves it byte-for-byte.
        let keep = rename_patch(
            "pub(in crate) struct Old",
            "pub(in crate) struct New",
            "@@ -1,1 +1,1 @@",
        );
        let out = apply(&keep, "pub(in crate) struct Old;\n").unwrap();
        assert_eq!(out, "pub(in crate) struct New;\n", "got: {out:?}");
        // Wrong-vis sites skip, as with every other vis spelling.
        let patch = rename_patch("struct Old", "pub(in crate) struct New", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, "pub struct Old;\n").is_err());
    }

    #[test]
    fn item_name_hole_evolves_vis_across_hetero_names() {
        // Vis-sweep residue: a bare `N` on both sides with one vis step
        // binds per site, replays each name, and rewrites the vis slot
        // (add, remove, and replace), skipping wrong-vis sites.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=r\n# lang=rust\n# allow=many\n- struct N\n+ pub struct N\n";
        let src = "struct A;\nstruct B(u32);\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out, "pub struct A;\npub struct B(u32);\n", "got: {out:?}");
        let remove = patch.replace("- struct N\n+ pub struct N", "- pub struct N\n+ struct N");
        let out = apply(&remove, "pub struct A;\npub struct B(u32);\n").unwrap();
        assert_eq!(out, "struct A;\nstruct B(u32);\n", "got: {out:?}");
        let change = patch.replace(
            "- struct N\n+ pub struct N",
            "- pub struct N\n+ pub(crate) struct N",
        );
        let out = apply(&change, "pub struct A;\npub struct B;\n").unwrap();
        assert_eq!(
            out, "pub(crate) struct A;\npub(crate) struct B;\n",
            "got: {out:?}"
        );
        // Wrong-vis sites skip (the minus vis gates each site); a file
        // with no minus-vis site fails to match.
        assert!(apply(patch, "pub struct A;\npub struct B;\n").is_err());
        // Differing holes are still a rename+vis joint with holes: veto.
        let differ = patch
            .replace("# allow=many", "# allow=many\n# ident M")
            .replace("+ pub struct N", "+ pub struct M");
        let err = apply(&differ, src).unwrap_err();
        assert!(err.contains("must match on both sides"), "got: {err}");
        // Same-hole same-vis is still the rename-usage error.
        let nodelta = patch.replace("+ pub struct N", "+ struct N");
        let err = apply(&nodelta, src).unwrap_err();
        assert!(err.contains("identical"), "got: {err}");
    }
}
