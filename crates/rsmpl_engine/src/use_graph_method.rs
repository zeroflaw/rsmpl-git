//! WI-3/WI-21 + WI-6: `# follow=uses` for method renames (SPEC 14.8).
//!
//! Definition follow plus anchored call-site follow. For a trait-header
//! `ImplContext` rename `old` → `new`, every
//! `impl <Trait> for <Self>` block whose trait tail names the renamed
//! trait contributes its same-name `ImplItem::Fn` definition ident as a
//! *definition claim* (rewritten, labelled `rename`). For an inherent
//! `impl S {` rename, every inherent `impl S` block contributes its
//! same-name definition (WI-6). Inherent impls never claim trait
//! renames (and trait-side items never claim inherent renames) — a
//! different item becomes `shadowed` evidence. Anchored call sites
//! (`recv.old()` with a proven receiver, `S::old` tails) rewrite as
//! `follow` edits; unproven receivers are `off-target`/`shadowed`
//! evidence, never a veto.

use syn::visit::{self, Visit};

use crate::follow_reasons::{reason as skip_reason, FollowSkip};
use crate::lines::LineIndex;
use crate::matcher_item::span_bytes;
use crate::use_graph::FollowVeto;
use crate::use_graph_method_calls::{collect_method_calls, file_attr_opaque};

/// Last `::` segment, cut at generics/whitespace (`"S < T >"` → `"S"`).
pub(crate) fn last_ident(normed: &str) -> String {
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

/// Trait name from a trait-header filter line (`trait Tr {` → `Tr`).
/// `None` when the header does not parse as a trait header.
pub(crate) fn method_trait_name(impl_header: &str) -> Option<String> {
    let filter = rsmpl_pattern::parse_impl_header(impl_header)?;
    if !filter.want_trait {
        return None;
    }
    if filter.trait_path.is_empty() {
        return None;
    }
    Some(filter.trait_path.clone())
}

/// WI-6 inherent domain (split to `use_graph_method_inherent.rs`
/// under the D5 file-size gate): re-exported here so the follow paths
/// below and the inline tests keep one import site.
pub(crate) use crate::use_graph_method_inherent::{
    collect_inherent_impl_claims, collect_trait_decl_fn_lines, collect_trait_impl_fn_lines,
    inherent_has_shadow, inherent_other_lines, method_domain, MethodDomain,
};

/// Extracted method rename names from an `ImplContext` hunk: (old, new).
/// Diet `V Q fn old …` spellings carry concrete idents, so a light
/// `fn <ident>` scan suffices; name holes (`N`, `$M`) decline (no claim).
pub(crate) fn method_rename_names(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<(String, String)> {
    let old_pat = crate::matcher_fn::minus_fn_pattern(rh, compiled).ok()?;
    let old = match &old_pat.name {
        rsmpl_pattern::NamePat::Concrete(n) => n.clone(),
        rsmpl_pattern::NamePat::Meta(_) => return None,
    };
    // Plus block mirrors the minus extraction (see matcher_impl).
    let full_plus: String = rh
        .lines
        .iter()
        .filter(|l| l.is_addition() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let (vis, qual, gen, wh, ty) = crate::matcher_fn::slot_metas(compiled);
    let ident = compiled.table.ident_slot_metas();
    let block =
        crate::matcher_fn::extract_fn_block(&full_plus, &[vis.clone(), qual.clone()].concat());
    let plus_pat = rsmpl_pattern::parse_fn_pattern(
        &block,
        &vis,
        &qual,
        &gen,
        &wh,
        &ty,
        &ident,
        rh.returns_none(),
    )
    .ok()?;
    let new = match &plus_pat.name {
        rsmpl_pattern::NamePat::Concrete(n) => n.clone(),
        rsmpl_pattern::NamePat::Meta(_) => return None,
    };
    if old.is_empty() || new.is_empty() || old == new {
        return None;
    }
    Some((old, new))
}

/// WI-1-style definition claims for trait methods: same-name `fn` idents
/// in `impl <Trait> for <Self>` blocks naming `trait_name`.
/// Returns (path, start, end, line). Inherent impls never claim.
pub(crate) fn collect_impl_fn_claims(
    files: &[(String, &syn::File, &str, &LineIndex)],
    trait_name: &str,
    old: &str,
) -> Vec<(String, usize, usize, u32)> {
    let mut out = Vec::new();
    for (path, parsed, original, index) in files {
        struct V<'a> {
            trait_name: &'a str,
            old: &'a str,
            original: &'a str,
            index: &'a LineIndex,
            out: Vec<(usize, usize, u32)>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                let names_trait = match &node.trait_ {
                    Some((_, p, _)) => last_ident(&norm_path(p)) == *self.trait_name,
                    None => false,
                };
                if names_trait {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                if let Some((a, b)) =
                                    span_bytes(self.original, self.index, m.sig.ident.span())
                                {
                                    if self.original.get(a..b) == Some(self.old) {
                                        let line = m.sig.ident.span().start().line as u32;
                                        self.out.push((a, b, line));
                                    }
                                }
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut v = V {
            trait_name,
            old,
            original,
            index,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for (a, b, line) in v.out {
            out.push((path.clone(), a, b, line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Inherent same-name `fn` definitions (`impl S { fn old }`): evidence
/// only (`shadowed`) under a *trait* rename, never claims.
pub(crate) fn collect_inherent_fn_lines(
    files: &[(String, &syn::File, &str, &LineIndex)],
    old: &str,
) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (path, parsed, _, _) in files {
        struct V<'a> {
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                if node.trait_.is_none() {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                self.out.push(m.sig.ident.span().start().line as u32);
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut v = V {
            old,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for line in v.out {
            out.push((path.clone(), line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// One method follow edit: byte span plus whether it is a definition claim.
pub(crate) type MethodEdit = (usize, usize, bool);

/// Per-file method follow spans (path + edits).
pub(crate) type MethodFollowSpans = Vec<(String, Vec<MethodEdit>)>;

/// Workspace outcome for trait-method follow: definition claims plus
/// anchored call-site claims (`recv.old()` with a proven receiver,
/// `S::old` tails). Unproven receivers are `off-target`/`shadowed`
/// evidence, never a veto. Opaque macro/attribute mentions of `old` veto
/// without fallback (assoc parity).
pub(crate) struct MethodOutcome {
    pub per_file: MethodFollowSpans,
    pub skipped: Vec<FollowSkip>,
}

/// Self-type tails of `impl <Trait> for <Self>` blocks naming `trait_name`.
/// Call-site anchors prove against this set (D8: text the change contains).
fn collect_domain_self(
    files: &[(String, &syn::File, &str, &LineIndex)],
    trait_name: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for (_, parsed, _, _) in files {
        struct V<'a> {
            trait_name: &'a str,
            out: Vec<String>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                if let Some((_, p, _)) = &node.trait_ {
                    if last_ident(&norm_path(p)) == *self.trait_name {
                        if let Some(s) = self_ty_last(&node.self_ty) {
                            if !s.is_empty() {
                                self.out.push(s);
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut v = V {
            trait_name,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        out.extend(v.out);
    }
    out.sort();
    out.dedup();
    out
}

/// True when another item defines same-name `fn old`: an inherent
/// `impl S { fn old }`, an `impl <Other> for _` with a different trait
/// tail, or another `trait` block declaring `fn old`.
fn method_has_shadow(
    files: &[(String, &syn::File, &str, &LineIndex)],
    trait_name: &str,
    old: &str,
) -> bool {
    for (_, parsed, _, _) in files {
        struct V<'a> {
            trait_name: &'a str,
            old: &'a str,
            hit: bool,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                let names_target = match &node.trait_ {
                    Some((_, p, _)) => last_ident(&norm_path(p)) == *self.trait_name,
                    None => false,
                };
                let is_other = match &node.trait_ {
                    None => true,
                    Some((_, p, _)) => last_ident(&norm_path(p)) != *self.trait_name,
                };
                if is_other {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                self.hit = true;
                            }
                        }
                    }
                }
                let _ = names_target;
                visit::visit_item_impl(self, node);
            }
            fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
                if node.ident != self.trait_name {
                    for it in &node.items {
                        if let syn::TraitItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                self.hit = true;
                            }
                        }
                    }
                }
                visit::visit_item_trait(self, node);
            }
        }
        let mut v = V {
            trait_name,
            old,
            hit: false,
        };
        v.visit_file(parsed);
        if v.hit {
            return true;
        }
    }
    false
}

// WI-21 call-site walk (split to `use_graph_method_calls.rs` under
// the D5 file-size gate): receiver roots, the walker, and the
// per-file claim collector live there; this module keeps domains,
// definition claims, the workspace resolver, and orphan evidence.

/// Per-ident macro lines mentioning `old` (WI-2b ident-line parity:
/// multi-line macros report the ident line, not the macro start line).
/// Mirrors orphan/assoc/member helpers: whole-ident match only, recurse groups.
pub(crate) fn macro_ident_lines(tokens: &proc_macro2::TokenStream, old: &str) -> Vec<u32> {
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

/// Per-ident lines for an `ItemMacro` node (`macro_rules!` definitions):
/// the macro name ident, the macro path segments, plus every `old` ident
/// inside the body tokens. Whole-ident only, mirroring `macro_ident_lines`.
fn item_macro_ident_lines(node: &syn::ItemMacro, old: &str) -> Vec<u32> {
    let mut out = Vec::new();
    if node.ident.as_ref().is_some_and(|i| i == old) {
        if let Some(ident) = node.ident.as_ref() {
            out.push(ident.span().start().line as u32);
        }
    }
    for seg in &node.mac.path.segments {
        if seg.ident == old {
            out.push(seg.ident.span().start().line as u32);
        }
    }
    out.extend(macro_ident_lines(&node.mac.tokens, old));
    out
}

/// Per-ident lines for an `ImplItemMacro` node: path segments plus body tokens.
fn impl_item_macro_ident_lines(node: &syn::ImplItemMacro, old: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for seg in &node.mac.path.segments {
        if seg.ident == old {
            out.push(seg.ident.span().start().line as u32);
        }
    }
    out.extend(macro_ident_lines(&node.mac.tokens, old));
    out
}

// Call-site walk types live in `use_graph_method_calls`.
pub(crate) fn self_ty_last(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(tp) = ty {
        if tp.qself.is_none() {
            return tp.path.segments.last().map(|s| s.ident.to_string());
        }
    }
    None
}

pub(crate) fn resolve_method_workspace(
    files: &[(String, &syn::File, &str, &LineIndex)],
    domain: &MethodDomain,
    old: &str,
    def_path: &str,
    renamed: &[(usize, usize)],
    fallback_syntax: bool,
) -> Result<MethodOutcome, FollowVeto> {
    // Definition claims per domain: trait-proven `impl <Tr> for <Self>`
    // defs, or inherent `impl S` defs of the domain self type (WI-6).
    let claims = match domain {
        MethodDomain::Trait(t) => collect_impl_fn_claims(files, t, old),
        MethodDomain::Inherent(s) => collect_inherent_impl_claims(files, s, old),
    };
    // Opaque gate: a macro-token / attribute mention of `old` vetoes
    // without fallback (assoc parity, never a silent skip).
    let mut opaque: Vec<(String, u32, String)> = Vec::new();
    for (path, parsed, _, _) in files {
        struct V<'a> {
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
                // Per-ident lines (WI-2b ident-line parity): the macro
                // name, path, and body idents each report their own line.
                self.out.extend(item_macro_ident_lines(node, self.old));
            }
            fn visit_impl_item_macro(&mut self, node: &'ast syn::ImplItemMacro) {
                self.out.extend(impl_item_macro_ident_lines(node, self.old));
            }
        }
        let mut v = V {
            old,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for line in v.out {
            opaque.push((path.clone(), line, "a macro invocation".to_string()));
        }
        // Attribute paths and values (`#[old]`, `#[doc = old]`,
        // `#[doc = S::old]`, `#[foo(old)]`) are equally opaque: they
        // never resolve to the renamed method.
        for (line, ctx) in file_attr_opaque(parsed, old) {
            opaque.push((path.clone(), line, ctx));
        }
    }
    opaque.sort();
    opaque.dedup();
    if !opaque.is_empty() && !fallback_syntax {
        let (path, line, ctx) = &opaque[0];
        return Err(FollowVeto {
            reason: format!(
                "use of `{old}` inside {ctx} at {path}:{line}: cannot attribute it syntactically"
            ),
        });
    }
    let mut skipped: Vec<FollowSkip> = opaque
        .into_iter()
        .map(|(f, l, ctx)| {
            FollowSkip::new(
                format!("{f}:{l}: use of `{old}` inside {ctx} left untouched (opaque)"),
                &f,
                l,
                skip_reason::MACRO_HIDDEN,
            )
        })
        .collect();
    // Namesake definitions outside the domain are `shadowed`
    // evidence (different items, never claims). Trait domain: inherent
    // defs + other-trait defs. Inherent domain (WI-6): trait decls,
    // trait-impl defs, and other-type inherent defs.
    match domain {
        MethodDomain::Trait(t) => {
            for (f, l) in collect_inherent_fn_lines(files, old) {
                skipped.push(FollowSkip::new(
                    format!("{f}:{l}: inherent method `{old}` (different item) left untouched"),
                    &f,
                    l,
                    skip_reason::SHADOWED,
                ));
            }
            // Other-trait `impl <Other> for _` definitions of `old` are
            // shadow evidence (a different item), mirroring inherent
            // namesakes.
            {
                let mut seen: Vec<(String, u32)> = Vec::new();
                for (path, parsed, _, _) in files {
                    struct V<'a> {
                        trait_name: &'a str,
                        old: &'a str,
                        out: Vec<u32>,
                    }
                    impl<'ast> Visit<'ast> for V<'_> {
                        fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                            let is_other = match &node.trait_ {
                                None => false,
                                Some((_, p, _)) => last_ident(&norm_path(p)) != *self.trait_name,
                            };
                            if is_other {
                                for it in &node.items {
                                    if let syn::ImplItem::Fn(m) = it {
                                        if m.sig.ident == self.old {
                                            self.out.push(m.sig.ident.span().start().line as u32);
                                        }
                                    }
                                }
                            }
                            visit::visit_item_impl(self, node);
                        }
                    }
                    let mut v = V {
                        trait_name: t,
                        old,
                        out: Vec::new(),
                    };
                    v.visit_file(parsed);
                    for l in v.out {
                        seen.push((path.clone(), l));
                    }
                }
                seen.sort();
                seen.dedup();
                for (f, l) in seen {
                    skipped.push(FollowSkip::new(
                        format!(
                            "{f}:{l}: other-trait method `{old}` (different item) left untouched"
                        ),
                        &f,
                        l,
                        skip_reason::SHADOWED,
                    ));
                }
            }
        }
        MethodDomain::Inherent(s) => {
            for (f, l) in collect_trait_decl_fn_lines(files, old)
                .into_iter()
                .chain(collect_trait_impl_fn_lines(files, old))
                .chain(inherent_other_lines(files, s, old))
            {
                skipped.push(FollowSkip::new(
                    format!(
                        "{f}:{l}: method `{old}` on another item (different item) left untouched"
                    ),
                    &f,
                    l,
                    skip_reason::SHADOWED,
                ));
            }
        }
    }
    // WI-21: anchored call-site claims. Domain self types prove receivers
    // via the WI-14 anchor order; unproven receivers are evidence only.
    // Inherent domains (WI-6) prove against the single self type.
    let walker_trait: &str = match domain {
        MethodDomain::Trait(t) => t,
        MethodDomain::Inherent(_) => "",
    };
    let domain_self: Vec<String> = match domain {
        MethodDomain::Trait(t) => collect_domain_self(files, t),
        MethodDomain::Inherent(s) => vec![s.clone()],
    };
    let has_shadow = match domain {
        MethodDomain::Trait(t) => method_has_shadow(files, t, old),
        MethodDomain::Inherent(s) => inherent_has_shadow(files, s, old),
    };
    // Claims outside already-renamed definition spans only.
    let mut per_file: MethodFollowSpans = Vec::new();
    for (path, a, b, _) in claims {
        if path == def_path && renamed.iter().any(|(s, e)| *s == a && *e == b) {
            continue;
        }
        match per_file.iter_mut().find(|(f, _)| f == &path) {
            Some((_, v)) => v.push((a, b, true)),
            None => per_file.push((path.clone(), vec![(a, b, true)])),
        }
    }
    // Anchored calls per file (never veto: unproven sites skip below).
    for (path, parsed, original, index) in files {
        let calls = collect_method_calls(parsed, walker_trait, &domain_self, old, has_shadow);
        for (line, ctx) in &calls.opaque {
            skipped.push(FollowSkip::new(
                format!("{path}:{line}: use of `{old}` inside {ctx} left untouched (opaque)"),
                path,
                *line,
                skip_reason::MACRO_HIDDEN,
            ));
        }
        let domain_word = match domain {
            MethodDomain::Trait(_) => "the trait domain",
            MethodDomain::Inherent(_) => "the inherent domain",
        };
        for line in &calls.shadowed {
            skipped.push(FollowSkip::new(
                format!(
                    "{path}:{line}: unanchored `{old}` call left untouched (receiver does not prove {domain_word})"
                ),
                path,
                *line,
                skip_reason::SHADOWED,
            ));
        }
        for line in &calls.off_target {
            skipped.push(FollowSkip::new(
                format!(
                    "{path}:{line}: unanchored `{old}` call left untouched (receiver does not prove {domain_word})"
                ),
                path,
                *line,
                skip_reason::OFF_TARGET,
            ));
        }
        for (line, reason) in &calls.chain_skipped {
            crate::use_graph_method_chain::push_chain_skip(&mut skipped, path, old, *line, reason);
        }
        let mut call_edits: Vec<(usize, usize, bool)> = Vec::new();
        for span in &calls.claims {
            let line = span.start().line as u32;
            let Some((a, b)) = span_bytes(original, index, *span) else {
                return Err(FollowVeto {
                    reason: format!("use of `{old}` at {path}:{line} maps outside the file"),
                });
            };
            if original.get(a..b) != Some(old) {
                return Err(FollowVeto {
                    reason: format!("use of `{old}` at {path}:{line} is not the expected text"),
                });
            }
            if *path == *def_path && renamed.iter().any(|(s, e)| a < *e && *s < b) {
                continue;
            }
            call_edits.push((a, b, false));
        }
        call_edits.sort_unstable();
        call_edits.dedup();
        for (a, b, is_def) in call_edits {
            match per_file.iter_mut().find(|(f, _)| f == path) {
                Some((_, v)) => v.push((a, b, is_def)),
                None => per_file.push(((*path).clone(), vec![(a, b, is_def)])),
            }
        }
    }
    for (_, v) in per_file.iter_mut() {
        v.sort_unstable();
        v.dedup();
    }
    per_file.sort_by(|a, b| a.0.cmp(&b.0));
    skipped.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    skipped.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.reason == b.reason);
    Ok(MethodOutcome { per_file, skipped })
}

/// Follow target for a method `ImplContext` rename hunk: (old, new,
/// header). Trait headers and bare inherent headers (`impl S {`)
/// qualify (WI-6); `impl T for S` headers, evolutions, and name holes
/// stay `None` (C17). Single call so `use_graph.rs` stays small (D5).
pub(crate) fn method_follow_target(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<(String, String, String)> {
    let rsmpl_pattern::PatternKind::ImplContext { impl_header } = &compiled.kind else {
        return None;
    };
    method_domain(impl_header)?;
    method_rename_names(rh, compiled).map(|(o, n)| (o, n, impl_header.clone()))
}

/// Resolve method definition follow to engine edits + skips.
/// Single call so `use_graph.rs` keeps only the label mapping (D5).
pub(crate) fn resolve_method_follow(
    refs: &[(String, &syn::File, &str, &crate::lines::LineIndex)],
    domain: &MethodDomain,
    old: &str,
    def_path: &str,
    renamed: &[(usize, usize)],
    fallback_syntax: bool,
) -> Result<(MethodFollowSpans, Vec<FollowSkip>), FollowVeto> {
    let outcome = resolve_method_workspace(refs, domain, old, def_path, renamed, fallback_syntax)?;
    Ok((outcome.per_file, outcome.skipped))
}

/// New name for a trait-method follow hunk, if it is a concrete rename.
/// Single call for `plan.rs`/`validate.rs` so the planners stay small.
pub(crate) fn method_follow_new_name(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<String> {
    method_rename_names(rh, compiled).map(|(_, n)| n)
}

/// Plan-layer orphan entry: method `ImplContext` renames list leftover
/// domain definitions; everything else lists nothing. Keeps
/// `plan.rs` to a one-line arm (D5 file-size gate).
pub(crate) fn method_orphans_for(
    states: &std::collections::HashMap<String, crate::lines::FileState>,
    file_edits: &std::collections::HashMap<String, Vec<crate::edits::TextEdit>>,
    path: &str,
    edits_before: usize,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Vec<crate::OrphanReport> {
    let rsmpl_pattern::PatternKind::ImplContext { impl_header } = &compiled.kind else {
        return Vec::new();
    };
    let Some(domain) = method_domain(impl_header) else {
        return Vec::new();
    };
    let Some((old, _)) = method_rename_names(rh, compiled) else {
        return Vec::new();
    };
    method_orphans(states, file_edits, path, edits_before, &domain, &old)
}

/// Orphan evidence for definition-only method renames: leftover
/// domain `fn` definitions. Mirrors `assoc_orphans` (WI-1b).
#[allow(clippy::too_many_arguments)]
pub(crate) fn method_orphans(
    states: &std::collections::HashMap<String, crate::lines::FileState>,
    file_edits: &std::collections::HashMap<String, Vec<crate::edits::TextEdit>>,
    path: &str,
    edits_before: usize,
    domain: &MethodDomain,
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
    // Definition claims per domain (WI-6 inherent arm mirrors the
    // trait arm: other-domain defs in the same file are orphans too).
    let domain_claims: Vec<(String, usize, usize, u32)> = match domain {
        MethodDomain::Trait(t) => collect_impl_fn_claims(&refs, t, old),
        MethodDomain::Inherent(s) => collect_inherent_impl_claims(&refs, s, old),
    };
    let claims = domain_claims;
    let mut out = Vec::new();
    for (f, _, _, line) in claims {
        // Skip spans the definition hunk already rewrote.
        if f == path {
            continue;
        }
        let text = states
            .get(&f)
            .map(|st| {
                let s = st.index.line_start(line).unwrap_or(0);
                let e = st.index.line_start(line + 1).unwrap_or(st.original.len());
                st.original.get(s..e).unwrap_or("").trim().to_string()
            })
            .unwrap_or_default();
        out.push(crate::OrphanReport {
            file: f,
            line,
            text,
            reason: "off-target".to_string(),
        });
    }
    // Same-file leftover claims (not in `renamed`) are orphans too.
    let domain_claims_same: Vec<(String, usize, usize, u32)> = match domain {
        MethodDomain::Trait(t) => collect_impl_fn_claims(&refs, t, old),
        MethodDomain::Inherent(s) => collect_inherent_impl_claims(&refs, s, old),
    };
    for (f, a, b, line) in domain_claims_same {
        if f != path {
            continue;
        }
        if renamed.iter().any(|(s, e)| *s == a && *e == b) {
            continue;
        }
        // The definition hunk owns exactly one block; any other claimed
        // def in the same file is leftover evidence.
        let _ = (a, b);
        let text = states
            .get(&f)
            .map(|st| {
                let s = st.index.line_start(line).unwrap_or(0);
                let e = st.index.line_start(line + 1).unwrap_or(st.original.len());
                st.original.get(s..e).unwrap_or("").trim().to_string()
            })
            .unwrap_or_default();
        out.push(crate::OrphanReport {
            file: f,
            line,
            text,
            reason: "off-target".to_string(),
        });
    }
    // WI-21 orphan parity: anchored calls report as `off-target`;
    // unproven receivers as `shadowed`/`off-target` (WI-11 chain sites
    // carry `chain-depth`/`chain-unproven`/`ambiguous-bound`); macros as `macro-hidden`.
    {
        let walker_trait: &str = match domain {
            MethodDomain::Trait(t) => t,
            MethodDomain::Inherent(_) => "",
        };
        let domain_self: Vec<String> = match domain {
            MethodDomain::Trait(t) => collect_domain_self(&refs, t),
            MethodDomain::Inherent(s) => vec![s.clone()],
        };
        let has_shadow = match domain {
            MethodDomain::Trait(t) => method_has_shadow(&refs, t, old),
            MethodDomain::Inherent(s) => inherent_has_shadow(&refs, s, old),
        };
        for (f, parsed, _, _) in &refs {
            let calls = collect_method_calls(parsed, walker_trait, &domain_self, old, has_shadow);
            let mut lines: Vec<(u32, &'static str)> = Vec::new();
            for span in &calls.claims {
                lines.push((span.start().line as u32, "off-target"));
            }
            for l in &calls.shadowed {
                lines.push((*l, "shadowed"));
            }
            for l in &calls.off_target {
                lines.push((*l, "off-target"));
            }
            for (l, reason) in &calls.chain_skipped {
                lines.push((*l, *reason));
            }
            for (l, _) in &calls.opaque {
                lines.push((*l, "macro-hidden"));
            }
            lines.sort_unstable();
            lines.dedup();
            for (line, reason) in lines {
                let text = states
                    .get(f)
                    .map(|st| {
                        let s = st.index.line_start(line).unwrap_or(0);
                        let e = st.index.line_start(line + 1).unwrap_or(st.original.len());
                        st.original.get(s..e).unwrap_or("").trim().to_string()
                    })
                    .unwrap_or_default();
                out.push(crate::OrphanReport {
                    file: f.clone(),
                    line,
                    text,
                    reason: reason.to_string(),
                });
            }
        }
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn apply(patch: &str, files: &[(&str, &str)]) -> Result<HashMap<String, String>, String> {
        let parsed = rsmpl_patch::parse_patch(patch).map_err(|e| format!("{e:?}"))?;
        let mut map: HashMap<String, String> = files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        crate::apply_patch_to_tree(&mut map, &parsed).map_err(|e| e.render())?;
        Ok(map)
    }

    fn apply_files(
        patch: &str,
        files: &[(&str, &str)],
    ) -> Result<(HashMap<String, String>, crate::ApplyReport), String> {
        let parsed = rsmpl_patch::parse_patch(patch).map_err(|e| format!("{e:?}"))?;
        let mut map: HashMap<String, String> = files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let report = crate::apply_patch_to_tree(&mut map, &parsed).map_err(|e| e.render())?;
        Ok((map, report))
    }

    const SRC: &str = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n";

    fn trait_patch() -> String {
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn new G (...) W ;\n"
            .to_string()
    }

    #[test]
    fn follow_rewrites_trait_proven_impl_def() {
        let (files, report) = apply_files(&trait_patch(), &[("src/lib.rs", SRC)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("fn new(&self);"), "got: {out}");
        assert!(out.contains("fn new(&self) {"), "got: {out}");
        assert!(!out.contains("fn old"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
    }

    #[test]
    fn follow_cross_file_impl_def() {
        let patch = trait_patch().replace("src/lib.rs", "src/a.rs");
        let (files, _) = apply_files(
            &patch,
            &[
                ("src/a.rs", "trait T {\n    fn old(&self);\n}\n"),
                (
                    "src/b.rs",
                    "struct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n",
                ),
            ],
        )
        .unwrap();
        assert!(files["src/a.rs"].contains("fn new(&self);"));
        assert!(files["src/b.rs"].contains("fn new(&self) {"));
        assert!(!files["src/b.rs"].contains("fn old"));
    }

    #[test]
    fn inherent_namesake_untouched_shadowed() {
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nstruct U;\nimpl U {\n    fn old(&self) {\n    }\n}\n";
        let (files, report) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("impl U {\n    fn old(&self) {"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "shadowed"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn no_follow_lists_impl_def_orphans() {
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn new G (...) W ;\n";
        let (_, report) = apply_files(p, &[("src/lib.rs", SRC)]).unwrap();
        assert!(
            report.rules[0]
                .orphans
                .iter()
                .any(|o| o.text.contains("fn old")),
            "got: {:?}",
            report.rules[0].orphans
        );
    }

    // WI-6: inherent-method follow block shape (`{` definitions carry
    // the `...` body + `}` closers, per `block_hunk_body`).
    const INHERENT_SRC: &str =
        "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\nfn f(s: S) {\n    s.old();\n}\n";

    fn inherent_patch() -> String {
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# follow=uses\n  impl S {\n      ...\n-     V Q fn old G (...) W {\n+     V Q fn new G (...) W {\n          ...\n      }\n  }\n"
            .to_string()
    }

    #[test]
    fn inherent_header_follow_accepts() {
        // WI-6: the F6 probe — def + param-anchored call rewrite.
        let (files, report) =
            apply_files(&inherent_patch(), &[("src/lib.rs", INHERENT_SRC)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("fn new(&self) {"), "got: {out}");
        assert!(!out.contains("fn old"), "got: {out}");
        assert!(out.contains("s.new();"), "got: {out}");
        assert!(!out.contains("s.old()"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
    }

    #[test]
    fn inherent_other_type_namesake_shadowed() {
        // Same-name method on another struct: untouched, `shadowed`.
        let src = "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\nstruct U;\nimpl U {\n    fn old(&self) {\n    }\n}\nfn f(s: S, u: U) {\n    s.old();\n    u.old();\n}\n";
        let (files, report) = apply_files(&inherent_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("s.new();"), "got: {out}");
        assert!(out.contains("u.old();"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "shadowed"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn inherent_trait_side_namesake_shadowed() {
        // A trait decl + trait-impl def of the same name are different
        // items: untouched, `shadowed` evidence.
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl S {\n    fn old(&self) {\n    }\n}\nimpl T for S {\n    fn old(&self) {\n    }\n}\n";
        let (files, report) = apply_files(&inherent_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("fn new(&self) {"), "got: {out}");
        assert!(out.contains("fn old(&self);"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "shadowed"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn inherent_unproven_receiver_off_target() {
        let src =
            "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\nfn f(q: u32) {\n    q.old();\n}\n";
        let (files, report) = apply_files(&inherent_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("q.old()"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "off-target"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn inherent_self_and_assoc_tails_rewrite() {
        // `self` inside `impl S` + `S::old` + `<S as T>::old` claim.
        let src = "trait T {}\nstruct S;\nimpl S {\n    fn old(&self) {\n    }\n    fn run(&self) {\n        self.old();\n    }\n}\nfn f() {\n    S::old();\n    <S as T>::old();\n}\n";
        let (files, _) = apply_files(&inherent_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("self.new();"), "got: {out}");
        assert!(out.contains("S::new();"), "got: {out}");
        assert!(out.contains("<S as T>::new();"), "got: {out}");
        assert!(!out.contains("old"), "got: {out}");
    }

    #[test]
    fn inherent_annotated_and_literal_anchors_rewrite() {
        let src = "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\nfn f() {\n    let x: S = other();\n    x.old();\n}\n";
        let (files, _) = apply_files(&inherent_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("x.new();"), "got: {out}");
        assert!(!out.contains("x.old()"), "got: {out}");
    }

    #[test]
    fn inherent_cross_file_defs_and_calls_rewrite() {
        let patch = inherent_patch().replace("src/lib.rs", "src/a.rs");
        let (files, _) = apply_files(
            &patch,
            &[
                ("src/a.rs", "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\n"),
                (
                    "src/b.rs",
                    "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\nfn f(s: S) {\n    s.old();\n}\n",
                ),
            ],
        )
        .unwrap();
        assert!(files["src/a.rs"].contains("fn new(&self) {"));
        assert!(files["src/b.rs"].contains("fn new(&self) {"));
        assert!(files["src/b.rs"].contains("s.new();"));
        assert!(!files["src/b.rs"].contains("old"));
    }

    #[test]
    fn inherent_no_follow_lists_def_and_call_orphans() {
        // Definition-only inherent rename: leftover defs + anchored
        // calls report as orphans (WI-1 blocks `verified`).
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n  impl S {\n      ...\n-     V Q fn old G (...) W {\n+     V Q fn new G (...) W {\n          ...\n      }\n  }\n";
        let (_, report) = apply_files(p, &[("src/lib.rs", INHERENT_SRC)]).unwrap();
        assert!(
            report.rules[0]
                .orphans
                .iter()
                .any(|o| o.text.contains("s.old()")),
            "call sites must orphan, got: {:?}",
            report.rules[0].orphans
        );
    }

    #[test]
    fn inherent_evolution_follow_rejected() {
        // Same name both sides: not a rename, follow stays C17.
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# follow=uses\n  impl S {\n      ...\n-     V Q fn old G (...) W {\n+     V Q fn old G (...) W {\n          ...\n      }\n  }\n";
        let err = apply(
            p,
            &[(
                "src/lib.rs",
                "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\n",
            )],
        )
        .unwrap_err();
        assert!(
            err.contains("C17") || err.contains("# follow=uses"),
            "got: {err}"
        );
    }

    #[test]
    fn trait_impl_header_follow_rejected() {
        // WI-6 non-goal: `impl T for S` headers name a trait-impl
        // block, not a renameable method owner — still C17.
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# follow=uses\n  impl T for S {\n      ...\n-     V Q fn old G (...) W {\n+     V Q fn new G (...) W {\n          ...\n      }\n  }\n";
        let err = apply(
            p,
            &[(
                "src/lib.rs",
                "trait T {}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n",
            )],
        )
        .unwrap_err();
        assert!(
            err.contains("C17") || err.contains("# follow=uses"),
            "got: {err}"
        );
    }

    #[test]
    fn evolution_follow_rejected() {
        // Same name both sides: not a rename, follow stays rejected.
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn old G (...) W ;\n";
        let err = apply(p, &[("src/lib.rs", SRC)]).unwrap_err();
        assert!(!err.is_empty(), "evolution follow must fail");
    }

    // WI-21: anchored call-site follow.
    const CALL_SRC: &str = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f(s: S) {\n    s.old();\n}\n";

    #[test]
    fn param_typed_call_rewrites() {
        let (files, _) = apply_files(&trait_patch(), &[("src/lib.rs", CALL_SRC)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("s.new()"), "got: {out}");
        assert!(!out.contains("s.old()"), "got: {out}");
        assert!(out.contains("fn new(&self);"), "got: {out}");
        assert!(out.contains("fn new(&self) {"), "got: {out}");
    }

    #[test]
    fn let_annotated_and_literal_init_rewrite() {
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f() {\n    let x: S = other();\n    x.old();\n    let y = S;\n    y.old();\n}\n";
        // `let x: S` anchors (annotation); `let y = S` is a unit init,
        // not a struct literal — only the annotated one claims.
        let (files, report) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("x.new()"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "off-target"),
            "unproven y.old() must be evidence, got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn struct_literal_init_rewrites() {
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nstruct W { s: S }\nfn f() {\n    let w = W { s: S };\n    w.s.old();\n}\n";
        // Field-chained receiver `w.s` roots at `w`; `w` has no `S`
        // anchor, so this stays evidence (pin: chains need WI-16).
        let (_, report) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "off-target" || d.reason == "shadowed"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn self_receiver_inside_domain_impl_rewrites() {
        let src = "trait T {\n    fn old(&self);\n    fn run(&self) {\n        self.old();\n    }\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n    fn run(&self) {\n        self.old();\n    }\n}\n";
        let (files, _) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(!out.contains("self.old()"), "got: {out}");
        assert!(out.contains("self.new()"), "got: {out}");
    }

    #[test]
    fn assoc_tail_rewrites() {
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f() {\n    S::old();\n}\n";
        let (files, _) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("S::new()"), "got: {out}");
        assert!(!out.contains("S::old"), "got: {out}");
    }

    #[test]
    fn unproven_receiver_stays_off_target() {
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f(q: u32) {\n    q.old();\n}\n";
        let (files, report) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("q.old()"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "off-target"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn shadowed_receiver_lists_shadowed() {
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nstruct U;\nimpl U {\n    fn old(&self) {\n    }\n}\nfn f(u: U) {\n    u.old();\n}\n";
        let (files, report) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("u.old()"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "shadowed"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn cross_file_call_rewrites() {
        let patch = trait_patch().replace("src/lib.rs", "src/a.rs");
        let (files, _) = apply_files(
            &patch,
            &[
                ("src/a.rs", "trait T {\n    fn old(&self);\n}\n"),
                (
                    "src/b.rs",
                    "struct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f(s: S) {\n    s.old();\n}\n",
                ),
            ],
        )
        .unwrap();
        assert!(
            files["src/b.rs"].contains("s.new()"),
            "got: {}",
            files["src/b.rs"]
        );
        assert!(
            !files["src/b.rs"].contains("s.old()"),
            "got: {}",
            files["src/b.rs"]
        );
    }

    #[test]
    fn no_follow_lists_call_orphans() {
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn new G (...) W ;\n";
        let (_, report) = apply_files(p, &[("src/lib.rs", CALL_SRC)]).unwrap();
        assert!(
            report.rules[0]
                .orphans
                .iter()
                .any(|o| o.text.contains("s.old()")),
            "call sites must orphan, got: {:?}",
            report.rules[0].orphans
        );
        assert!(
            report.rules[0]
                .orphans
                .iter()
                .any(|o| o.text.contains("fn old")),
            "got: {:?}",
            report.rules[0].orphans
        );
    }

    #[test]
    fn multiline_macro_reports_ident_line() {
        let src = "fn f() {\n    println!(\n        \"{}\",\n        old\n    );\n}\n";
        // `old` sits on line 4; the macro starts on line 2.
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
        assert_eq!(super::macro_ident_lines(&g.0[0], "old"), vec![4]);
    }

    #[test]
    fn single_line_macro_reports_same_line() {
        let src = "fn f() { println!(\"{}\", old); }\n";
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
        assert_eq!(super::macro_ident_lines(&g.0[0], "old"), vec![1]);
    }

    #[test]
    fn item_macro_body_reports_ident_line() {
        let src = "macro_rules! foo {\n    () => {\n        old\n    };\n}\n";
        // `old` sits on line 3; the macro starts on line 1.
        let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
        let node = parsed.items.first().expect("one item");
        let syn::Item::Macro(m) = node else {
            panic!("expected ItemMacro, got {node:?}");
        };
        assert_eq!(super::item_macro_ident_lines(m, "old"), vec![3]);
    }

    // WI-11: constructor chains, one-hop method-return chains, generic bounds.
    const CTOR_SRC: &str = "struct S;\nimpl S {\n    fn make() -> S {\n        S\n    }\n    fn run(&self) {\n    }\n}\nfn f() {\n    S::make().run();\n}\n";

    fn run_patch() -> String {
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# follow=uses\n  impl S {\n      ...\n-     V Q fn run G (...) W {\n+     V Q fn walk G (...) W {\n          ...\n      }\n  }\n"
            .to_string()
    }

    #[test]
    fn ctor_chain_rewrites_and_verifies() {
        let (files, report) = apply_files(&run_patch(), &[("src/lib.rs", CTOR_SRC)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("S::make().walk();"), "got: {out}");
        assert!(out.contains("fn walk(&self) {"), "got: {out}");
        assert!(!out.contains("run"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
        assert!(
            report.rules[0].follow_skipped_details.is_empty(),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn ctor_self_return_rewrites() {
        let src = CTOR_SRC.replace("fn make() -> S {", "fn make() -> Self {");
        let (files, _) = apply_files(&run_patch(), &[("src/lib.rs", &src)]).unwrap();
        assert!(files["src/lib.rs"].contains("S::make().walk();"));
    }

    #[test]
    fn ctor_wrong_return_is_chain_unproven() {
        let src = "struct S;\nstruct U;\nimpl S {\n    fn make() -> U {\n        U\n    }\n    fn run(&self) {\n    }\n}\nfn f() {\n    S::make().run();\n}\n";
        let (files, report) = apply_files(&run_patch(), &[("src/lib.rs", src)]).unwrap();
        assert!(files["src/lib.rs"].contains("S::make().run();"));
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "chain-unproven"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn one_hop_method_chain_rewrites() {
        let src = "struct S;\nimpl S {\n    fn clone2(&self) -> S {\n        S\n    }\n    fn run(&self) {\n    }\n}\nfn f(a: S) {\n    a.clone2().run();\n}\n";
        let (files, report) = apply_files(&run_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("a.clone2().walk();"), "got: {out}");
        assert!(
            report.rules[0].follow_skipped_details.is_empty(),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn unstated_return_is_chain_unproven() {
        let src = "struct S;\nimpl S {\n    fn clone2(&self) {\n    }\n    fn run(&self) {\n    }\n}\nfn f(a: S) {\n    a.clone2().run();\n}\n";
        let (_, report) = apply_files(&run_patch(), &[("src/lib.rs", src)]).unwrap();
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "chain-unproven"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn two_hop_chain_is_chain_depth() {
        let src = "struct S;\nimpl S {\n    fn make() -> S {\n        S\n    }\n    fn foo(&self) -> S {\n        S\n    }\n    fn run(&self) {\n    }\n}\nfn f() {\n    S::make().foo().run();\n}\n";
        let (_, report) = apply_files(&run_patch(), &[("src/lib.rs", src)]).unwrap();
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "chain-depth"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn self_rooted_chain_rewrites() {
        let src = "struct S;\nimpl S {\n    fn clone2(&self) -> S {\n        S\n    }\n    fn run(&self) {\n    }\n    fn go(&self) {\n        self.clone2().run();\n    }\n}\n";
        let (files, _) = apply_files(&run_patch(), &[("src/lib.rs", src)]).unwrap();
        assert!(files["src/lib.rs"].contains("self.clone2().walk();"));
    }

    #[test]
    fn generic_single_bound_rewrites_trait_call() {
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f<G: T>(t: G) {\n    t.old();\n}\n";
        let (files, report) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("t.new();"), "got: {out}");
        assert!(
            report.rules[0].follow_skipped_details.is_empty(),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn generic_multi_bound_is_ambiguous() {
        let src = "trait T {\n    fn old(&self);\n}\ntrait Other {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f<G: T + Other>(t: G) {\n    t.old();\n}\n";
        let (_, report) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "ambiguous-bound"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn shadowed_ctor_twin_stays_shadowed() {
        let src = "struct S;\nimpl S {\n    fn make() -> S {\n        S\n    }\n    fn run(&self) {\n    }\n}\nstruct U;\nimpl U {\n    fn make() -> U {\n        U\n    }\n    fn run(&self) {\n    }\n}\nfn f() {\n    S::make().run();\n    U::make().run();\n}\n";
        let (files, report) = apply_files(&run_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("S::make().walk();"), "got: {out}");
        assert!(out.contains("U::make().run();"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "shadowed"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn cross_file_ctor_stays_chain_unproven() {
        // WI-12 territory: a constructor proven in another file is not
        // consulted — the call skips `chain-unproven`, never guesses.
        let patch = run_patch().replace("src/lib.rs", "src/a.rs");
        let (_, report) = apply_files(
            &patch,
            &[
                (
                    "src/a.rs",
                    "struct S;\nimpl S {\n    fn make() -> S {\n        S\n    }\n    fn run(&self) {\n    }\n}\n",
                ),
                ("src/b.rs", "fn f() {\n    S::make().run();\n}\n"),
            ],
        )
        .unwrap();
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "chain-unproven"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn generic_bound_ignored_for_inherent() {
        // A `T: Tr` bound can never prove concrete `S`: inherent follow
        // leaves it to the existing evidence (off-target, no namesake).
        let src = "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\ntrait Tr {}\nfn f<G: Tr>(t: G) {\n    t.old();\n}\n";
        let (_, report) = apply_files(&inherent_patch(), &[("src/lib.rs", src)]).unwrap();
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "off-target"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .all(|d| d.reason != "ambiguous-bound"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }
}
