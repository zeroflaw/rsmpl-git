//! WI-3/WI-21: `# follow=uses` for trait-method renames (SPEC 14.8).
//!
//! Definition follow plus anchored call-site follow (WI-21).
//! For a trait-header `ImplContext` rename `old` → `new`, every
//! `impl <Trait> for <Self>` block whose trait tail names the renamed
//! trait contributes its same-name `ImplItem::Fn` definition ident as a
//! *definition claim* (rewritten, labelled `rename`). Inherent impls
//! (`impl S { fn old }` with no trait) never claim — they are a different
//! item and become `shadowed` evidence. Anchored call sites
//! (`recv.old()` with a proven receiver, `S::old` tails) rewrite as
//! `follow` edits; unproven receivers are `off-target`/`shadowed`
//! evidence, never a veto.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::follow_reasons::{reason as skip_reason, FollowSkip};
use crate::lines::LineIndex;
use crate::matcher_item::span_bytes;
use crate::use_graph::FollowVeto;
use crate::use_graph_anchor::AnchorIndex;

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
/// only (`shadowed`), never claims.
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
/// evidence, never a veto. Opaque macro mentions of `old` veto without
/// fallback (assoc parity).
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

/// Receiver root for a method call (`s` in `s.old()`, `self` in
/// `self.old()`, `a` in `a.f.old()` via the field chain). `None` for
/// call/literal/composite bases (`make().old()` never claims).
fn method_receiver_root(recv: &syn::Expr) -> Option<String> {
    match recv {
        syn::Expr::Path(p) => {
            if p.qself.is_some() || p.path.segments.len() != 1 {
                return None;
            }
            p.path.segments.first().map(|s| s.ident.to_string())
        }
        syn::Expr::Field(_) => crate::use_graph_anchor::field_root(recv),
        _ => None,
    }
}

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

fn tokens_mention(tokens: &proc_macro2::TokenStream, old: &str) -> bool {
    tokens.clone().into_iter().any(|t| match t {
        proc_macro2::TokenTree::Ident(id) => id == old,
        proc_macro2::TokenTree::Group(g) => tokens_mention(&g.stream(), old),
        _ => false,
    })
}

/// Per-file anchored call claims plus per-site evidence.
struct MethodCallOutcome {
    claims: Vec<proc_macro2::Span>,
    shadowed: Vec<u32>,
    off_target: Vec<u32>,
    opaque: Vec<(u32, String)>,
}

/// Syntactic call walker (WI-21): `recv.old()` claims iff the receiver
/// anchor names a domain self type; `S::old` tails claim iff `S` names
/// a domain self type or the trait itself. Bare `old` is not method
/// syntax (ignored); composite bases never claim. Unproven method
/// receivers are evidence (`shadowed` when another item defines `old`,
/// else `off-target`); unproven `X::old` tails with an unrelated
/// container are ignored outright (assoc parity — never a veto).
struct MethodCallWalker<'a> {
    trait_name: &'a str,
    domain_self: &'a [String],
    old: &'a str,
    has_shadow: bool,
    anchors: AnchorIndex,
    impl_stack: Vec<String>,
    trait_stack: Vec<String>,
    claims: Vec<proc_macro2::Span>,
    shadowed: Vec<u32>,
    off_target: Vec<u32>,
    opaque: Vec<(u32, String)>,
}

impl<'a> MethodCallWalker<'a> {
    fn push_unproven(&mut self, line: u32) {
        if self.has_shadow {
            self.shadowed.push(line);
        } else {
            self.off_target.push(line);
        }
    }

    fn domain_has(&self, name: &str) -> bool {
        self.domain_self.iter().any(|s| s == name)
    }

    fn receiver_proven(&self, root: &str, line: u32) -> bool {
        if root == "self" || root == "Self" {
            if self
                .anchors
                .self_type_at(line)
                .is_some_and(|s| self.domain_has(s))
            {
                return true;
            }
            // `self` inside the defining `trait T` default body names
            // the trait's own method (Self: T) — proven without an anchor.
            if self
                .trait_stack
                .last()
                .is_some_and(|t| t == self.trait_name)
            {
                return true;
            }
            return false;
        }
        self.anchors
            .anchor_of(root, line)
            .is_some_and(|s| self.domain_has(&s))
    }
}

fn self_ty_last(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(tp) = ty {
        if tp.qself.is_none() {
            return tp.path.segments.last().map(|s| s.ident.to_string());
        }
    }
    None
}

impl<'ast> Visit<'ast> for MethodCallWalker<'_> {
    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.trait_stack.push(node.ident.to_string());
        visit::visit_item_trait(self, node);
        self.trait_stack.pop();
    }

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

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == self.old {
            let line = node.method.span().start().line as u32;
            match method_receiver_root(&node.receiver) {
                Some(root) if !root.is_empty() => {
                    if self.receiver_proven(&root, line) {
                        self.claims.push(node.method.span());
                    } else {
                        self.push_unproven(line);
                    }
                }
                _ => self.push_unproven(line),
            }
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        let segments = path_segments(node);
        for (i, seg) in node.segments.iter().enumerate() {
            if i > 0 && seg.ident == self.old {
                let container = segments[i - 1].clone();
                let line = seg.ident.span().start().line as u32;
                if container == "Self" {
                    match self.impl_stack.last() {
                        Some(s) if self.domain_has(s) => self.claims.push(seg.ident.span()),
                        Some(_) => self.push_unproven(line),
                        None => {
                            if self
                                .trait_stack
                                .last()
                                .is_some_and(|t| t == self.trait_name)
                            {
                                self.claims.push(seg.ident.span());
                            }
                        }
                    }
                } else if container == *self.trait_name || self.domain_has(&container) {
                    self.claims.push(seg.ident.span());
                }
                // Unrelated `X::old` tails: ignored outright (different
                // item, assoc parity) — method-call receivers own the
                // evidence channel, not every path tail.
            }
        }
        visit::visit_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        if node.qself.is_some() {
            if let Some(last) = node.path.segments.last() {
                if last.ident == self.old {
                    // `<S as Tr>::old`: explicit trait names the item.
                    let segs = path_segments(&node.path);
                    let is_trait = segs.iter().any(|s| s == self.trait_name);
                    if is_trait {
                        self.claims.push(last.ident.span());
                    }
                }
            }
            return;
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if node.qself.is_some() {
            if let Some(last) = node.path.segments.last() {
                if last.ident == self.old {
                    let segs = path_segments(&node.path);
                    let is_trait = segs.iter().any(|s| s == self.trait_name);
                    if is_trait {
                        self.claims.push(last.ident.span());
                    }
                }
            }
            return;
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if tokens_mention(&node.tokens, self.old) {
            self.opaque.push((
                node.span().start().line as u32,
                "a macro invocation".to_string(),
            ));
        }
    }
}

/// Collect anchored call claims + evidence for one file.
fn collect_method_calls(
    parsed: &syn::File,
    trait_name: &str,
    domain_self: &[String],
    old: &str,
    has_shadow: bool,
) -> MethodCallOutcome {
    let mut w = MethodCallWalker {
        trait_name,
        domain_self,
        old,
        has_shadow,
        anchors: AnchorIndex::for_file(parsed),
        impl_stack: Vec::new(),
        trait_stack: Vec::new(),
        claims: Vec::new(),
        shadowed: Vec::new(),
        off_target: Vec::new(),
        opaque: Vec::new(),
    };
    w.visit_file(parsed);
    MethodCallOutcome {
        claims: w.claims,
        shadowed: w.shadowed,
        off_target: w.off_target,
        opaque: w.opaque,
    }
}

pub(crate) fn resolve_method_workspace(
    files: &[(String, &syn::File, &str, &LineIndex)],
    trait_name: &str,
    old: &str,
    def_path: &str,
    renamed: &[(usize, usize)],
    fallback_syntax: bool,
) -> Result<MethodOutcome, FollowVeto> {
    let claims = collect_impl_fn_claims(files, trait_name, old);
    // Opaque gate: a macro-token / attribute mention of `old` vetoes
    // without fallback (assoc parity, never a silent skip).
    let mut opaque: Vec<(String, u32)> = Vec::new();
    for (path, parsed, _, _) in files {
        struct V<'a> {
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
                let toks = quote::quote!(#node).to_string();
                if toks
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|t| t == self.old)
                {
                    self.out.push(node.span().start().line as u32);
                }
            }
            fn visit_impl_item_macro(&mut self, node: &'ast syn::ImplItemMacro) {
                let toks = quote::quote!(#node).to_string();
                if toks
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|t| t == self.old)
                {
                    self.out.push(node.span().start().line as u32);
                }
            }
        }
        let mut v = V {
            old,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for line in v.out {
            opaque.push((path.clone(), line));
        }
    }
    if !opaque.is_empty() && !fallback_syntax {
        let (path, line) = &opaque[0];
        return Err(FollowVeto {
            reason: format!(
                "use of `{old}` hides in a macro body at {path}:{line}: cannot attribute it syntactically"
            ),
        });
    }
    let mut skipped: Vec<FollowSkip> = opaque
        .into_iter()
        .map(|(f, l)| {
            FollowSkip::new(
                format!("{f}:{l}: use of `{old}` inside a macro body left untouched (opaque)"),
                &f,
                l,
                skip_reason::MACRO_HIDDEN,
            )
        })
        .collect();
    for (f, l) in collect_inherent_fn_lines(files, old) {
        skipped.push(FollowSkip::new(
            format!("{f}:{l}: inherent method `{old}` (different item) left untouched"),
            &f,
            l,
            skip_reason::SHADOWED,
        ));
    }
    // WI-21: anchored call-site claims. Domain self types prove receivers
    // via the WI-14 anchor order; unproven receivers are evidence only.
    let domain_self = collect_domain_self(files, trait_name);
    let has_shadow = method_has_shadow(files, trait_name, old);
    // Other-trait `impl <Other> for _` definitions of `old` are shadow
    // evidence (a different item), mirroring inherent namesakes.
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
                trait_name,
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
                format!("{f}:{l}: other-trait method `{old}` (different item) left untouched"),
                &f,
                l,
                skip_reason::SHADOWED,
            ));
        }
    }
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
        let calls = collect_method_calls(parsed, trait_name, &domain_self, old, has_shadow);
        for (line, ctx) in &calls.opaque {
            skipped.push(FollowSkip::new(
                format!("{path}:{line}: use of `{old}` inside {ctx} left untouched (opaque)"),
                path,
                *line,
                skip_reason::MACRO_HIDDEN,
            ));
        }
        for line in &calls.shadowed {
            skipped.push(FollowSkip::new(
                format!(
                    "{path}:{line}: unanchored `{old}` call left untouched (receiver does not prove the trait domain)"
                ),
                path,
                *line,
                skip_reason::SHADOWED,
            ));
        }
        for line in &calls.off_target {
            skipped.push(FollowSkip::new(
                format!(
                    "{path}:{line}: unanchored `{old}` call left untouched (receiver does not prove the trait domain)"
                ),
                path,
                *line,
                skip_reason::OFF_TARGET,
            ));
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

/// Follow target for a trait-header `ImplContext` rename hunk:
/// (old, new, header). `None` for inherent headers, evolutions, and
/// name holes. Single call so `use_graph.rs` stays small (D5).
pub(crate) fn method_follow_target(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<(String, String, String)> {
    let rsmpl_pattern::PatternKind::ImplContext { impl_header } = &compiled.kind else {
        return None;
    };
    if !rsmpl_pattern::is_trait_header(impl_header) {
        return None;
    }
    method_rename_names(rh, compiled).map(|(o, n)| (o, n, impl_header.clone()))
}

/// Resolve trait-method definition follow to engine edits + skips.
/// Single call so `use_graph.rs` keeps only the label mapping (D5).
pub(crate) fn resolve_method_follow(
    refs: &[(String, &syn::File, &str, &crate::lines::LineIndex)],
    impl_header: &str,
    old: &str,
    def_path: &str,
    renamed: &[(usize, usize)],
    fallback_syntax: bool,
) -> Result<(MethodFollowSpans, Vec<FollowSkip>), FollowVeto> {
    let Some(trait_name) = method_trait_name(impl_header) else {
        return Err(FollowVeto {
            reason: "method follow header does not parse as a `trait` block header".to_string(),
        });
    };
    let outcome =
        resolve_method_workspace(refs, &trait_name, old, def_path, renamed, fallback_syntax)?;
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

/// Plan-layer orphan entry: trait-header `ImplContext` renames list
/// leftover impl definitions; everything else lists nothing. Keeps
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
    let Some(trait_name) = method_trait_name(impl_header) else {
        return Vec::new();
    };
    let Some((old, _)) = method_rename_names(rh, compiled) else {
        return Vec::new();
    };
    method_orphans(states, file_edits, path, edits_before, &trait_name, &old)
}

/// Orphan evidence for definition-only trait-method renames: leftover
/// trait-proven impl `fn` definitions. Mirrors `assoc_orphans` (WI-1b).
#[allow(clippy::too_many_arguments)]
pub(crate) fn method_orphans(
    states: &std::collections::HashMap<String, crate::lines::FileState>,
    file_edits: &std::collections::HashMap<String, Vec<crate::edits::TextEdit>>,
    path: &str,
    edits_before: usize,
    trait_name: &str,
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
    let claims = collect_impl_fn_claims(&refs, trait_name, old);
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
    for (f, a, b, line) in collect_impl_fn_claims(&refs, trait_name, old) {
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
    // WI-21 orphan parity: anchored call sites the follow would rewrite
    // report as `off-target`; unproven receivers report as
    // `shadowed`/`off-target`; macro interiors as `macro-hidden`.
    {
        let domain_self = collect_domain_self(&refs, trait_name);
        let has_shadow = method_has_shadow(&refs, trait_name, old);
        for (f, parsed, _, _) in &refs {
            let calls = collect_method_calls(parsed, trait_name, &domain_self, old, has_shadow);
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

    #[test]
    fn inherent_header_follow_rejected() {
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  impl S {\n-     V Q fn old G (...) W {\n+     V Q fn new G (...) W {\n";
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
}
