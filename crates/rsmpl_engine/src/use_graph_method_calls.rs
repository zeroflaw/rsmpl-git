//! Method call-site walker (WI-21 claims + WI-11 chain fallback).
//!
//! Split from [`crate::use_graph_method`] under the D5 file-size gate:
//! the per-file anchored-claim walk (receiver roots, path/qself tails,
//! macro opacity) plus the WI-11 chain delegation. The workspace
//! resolver, orphan evidence, domains, and definition claims stay in
//! `use_graph_method`; both halves meet through
//! [`collect_method_calls`] and [`MethodCallOutcome`].

use syn::visit::{self, Visit};

use crate::use_graph_anchor::AnchorIndex;
use crate::use_graph_method::{macro_ident_lines, self_ty_last};

/// Receiver root for a method call (`s` in `s.old()`, `self` in
/// `self.old()`, `a` in `a.f.old()` via the field chain). `None` for
/// call/literal/composite bases — those route to the WI-11 chain
/// fallback, never to the direct anchor.
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

/// Per-file anchored call claims plus per-site evidence.
pub(crate) struct MethodCallOutcome {
    pub(crate) claims: Vec<proc_macro2::Span>,
    pub(crate) shadowed: Vec<u32>,
    pub(crate) off_target: Vec<u32>,
    /// WI-11 chain evidence: (line, reason) with its own vocabulary.
    pub(crate) chain_skipped: Vec<(u32, &'static str)>,
    pub(crate) opaque: Vec<(u32, String)>,
}

/// Syntactic call walker (WI-21): `recv.old()` claims iff the receiver
/// anchor names a domain self type; `S::old` tails claim iff `S` names
/// a domain self type or the trait itself. Bare `old` is not method
/// syntax (ignored); WI-11 chains claim through the sibling chain maps.
/// Unproven receivers are evidence (`shadowed`/`off-target`); unproven
/// `X::old` tails with an unrelated container are ignored outright
/// (assoc parity — never a veto).
struct MethodCallWalker<'a> {
    trait_name: &'a str,
    domain_self: &'a [String],
    old: &'a str,
    has_shadow: bool,
    anchors: AnchorIndex,
    chain: crate::use_graph_method_chain::ChainMaps,
    impl_stack: Vec<String>,
    trait_stack: Vec<String>,
    claims: Vec<proc_macro2::Span>,
    shadowed: Vec<u32>,
    off_target: Vec<u32>,
    chain_skipped: Vec<(u32, &'static str)>,
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

    /// WI-11 chain fallback (proof in `use_graph_method_chain`).
    fn chain_fallback(&mut self, recv: &syn::Expr, line: u32, root: Option<&str>) -> bool {
        use crate::use_graph_method_chain::{chain_action, ChainAction};
        match chain_action(
            recv,
            line,
            root,
            &self.anchors,
            &self.chain,
            self.trait_name,
            self.anchors.self_type_at(line),
            self.trait_stack.last().map(|s| s.as_str()),
            self.trait_name.is_empty(),
        ) {
            ChainAction::Claim => true,
            ChainAction::ClaimType(ty) => {
                if self.domain_has(&ty) {
                    return true;
                }
                self.push_unproven(line);
                false
            }
            ChainAction::Skip(reason) => {
                self.chain_skipped.push((line, reason));
                false
            }
            ChainAction::Unproven => {
                self.push_unproven(line);
                false
            }
        }
    }
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
                    let proven = self.receiver_proven(&root, line)
                        || self.chain_fallback(&node.receiver, line, Some(&root));
                    if proven {
                        self.claims.push(node.method.span());
                    }
                }
                _ => {
                    if self.chain_fallback(&node.receiver, line, None) {
                        self.claims.push(node.method.span());
                    }
                }
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
                    let is_trait =
                        !self.trait_name.is_empty() && segs.iter().any(|s| s == self.trait_name);
                    // WI-6: inherent qself (`<S as _>::old`, `<S>::old`)
                    // names the item through the self type instead.
                    let is_self = self.trait_name.is_empty()
                        && qself_ty_last(&node.qself).is_some_and(|s| self.domain_has(&s));
                    if is_trait || is_self {
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
                    let is_trait =
                        !self.trait_name.is_empty() && segs.iter().any(|s| s == self.trait_name);
                    // WI-6: inherent qself, self-type proof (see above).
                    let is_self = self.trait_name.is_empty()
                        && qself_ty_last(&node.qself).is_some_and(|s| self.domain_has(&s));
                    if is_trait || is_self {
                        self.claims.push(last.ident.span());
                    }
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
        for line in macro_ident_lines(&node.tokens, self.old) {
            self.opaque.push((line, "a macro invocation".to_string()));
        }
    }
}

/// Last ident of a `QSelf` position type (`S` in `<S as Tr>`).
/// `None` for non-path types. WI-6 inherent qself proof.
fn qself_ty_last(qself: &Option<syn::QSelf>) -> Option<String> {
    qself.as_ref().and_then(|q| self_ty_last(&q.ty))
}

/// Collect anchored call claims + evidence for one file.
/// `trait_name` is `""` for inherent domains (WI-6): the trait-half
/// checks never fire and qself claims prove through the self type.
pub(crate) fn collect_method_calls(
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
        chain: crate::use_graph_method_chain::ChainMaps::for_file(parsed),
        impl_stack: Vec::new(),
        trait_stack: Vec::new(),
        claims: Vec::new(),
        shadowed: Vec::new(),
        off_target: Vec::new(),
        chain_skipped: Vec::new(),
        opaque: Vec::new(),
    };
    w.visit_file(parsed);
    MethodCallOutcome {
        claims: w.claims,
        shadowed: w.shadowed,
        off_target: w.off_target,
        chain_skipped: w.chain_skipped,
        opaque: w.opaque,
    }
}
