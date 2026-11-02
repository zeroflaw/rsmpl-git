//! WI-11 anchor growth: constructor chains, one-hop method-return
//! chains, and generic-bound receivers (SPEC 14.8).
//!
//! Sibling of [`crate::use_graph_method`] under the D5 file-size gate:
//! all new proof lives here; the walker in `use_graph_method` only
//! threads the maps through and renders the verdicts. Same-file only
//! (D8) — cross-file constructors/returns stay `chain-unproven` until
//! WI-12 re-keys the probe. No type inference beyond stated
//! signatures: constructor names only select candidates, the stated
//! return type is the proof; an unstated return is `chain-unproven`,
//! never a guess.

use std::collections::{HashMap, HashSet};

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::follow_reasons::reason;
use crate::use_graph_anchor::AnchorIndex;

/// Candidate constructor names. The name only selects candidates —
/// the same-file `impl C` + stated `-> C`/`Self` return is the proof.
const CTOR_NAMES: [&str; 3] = ["new", "make", "default"];

/// Per-file proof maps for WI-11 chain resolution.
#[derive(Debug, Default)]
pub(crate) struct ChainMaps {
    /// Proven same-file constructors: `(self_ty, ctor_name)`.
    ctors: HashSet<(String, String)>,
    /// Stated inherent-method returns: `(self_ty, method) -> ret_ty`
    /// (`Self` already normalised to the impl self type).
    returns: HashMap<(String, String), String>,
    /// Generic-bound scopes in document order: (start, end, param → bounds).
    bounds: Vec<(u32, u32, HashMap<String, Vec<String>>)>,
}

impl ChainMaps {
    /// Build the maps for one parsed file.
    pub(crate) fn for_file(parsed: &syn::File) -> Self {
        let mut maps = Self::default();
        let mut builder = MapBuilder { maps: &mut maps };
        builder.visit_file(parsed);
        maps
    }

    fn bounds_at(&self, param: &str, line: u32) -> Option<&Vec<String>> {
        for (start, end, map) in &self.bounds {
            if line >= *start && line <= *end {
                if let Some(b) = map.get(param) {
                    return Some(b);
                }
            }
        }
        None
    }
}

struct MapBuilder<'a> {
    maps: &'a mut ChainMaps,
}

impl<'ast> Visit<'ast> for MapBuilder<'_> {
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if node.trait_.is_none() {
            if let Some(self_ty) = ty_tail(&node.self_ty) {
                if self_ty != "Self" {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            let name = m.sig.ident.to_string();
                            if let Some(ret) = sig_ret_tail(&m.sig) {
                                let norm = if ret == "Self" { self_ty.clone() } else { ret };
                                if CTOR_NAMES.contains(&name.as_str()) && norm == self_ty {
                                    self.maps.ctors.insert((self_ty.clone(), name.clone()));
                                }
                                self.maps.returns.insert((self_ty.clone(), name), norm);
                            }
                        }
                    }
                }
            }
        }
        visit::visit_item_impl(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.collect_fn_bounds(
            &node.sig.generics,
            node.span().start().line as u32,
            node.span().end().line as u32,
        );
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.collect_fn_bounds(
            &node.sig.generics,
            node.span().start().line as u32,
            node.span().end().line as u32,
        );
        visit::visit_impl_item_fn(self, node);
    }
}

impl MapBuilder<'_> {
    fn collect_fn_bounds(&mut self, generics: &syn::Generics, start: u32, end: u32) {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for param in &generics.params {
            if let syn::GenericParam::Type(tp) = param {
                let mut bounds = Vec::new();
                for b in &tp.bounds {
                    if let syn::TypeParamBound::Trait(tb) = b {
                        if let Some(last) = tb.path.segments.last() {
                            let n = last.ident.to_string();
                            if !n.is_empty() {
                                bounds.push(n);
                            }
                        }
                    }
                }
                if !bounds.is_empty() {
                    map.insert(tp.ident.to_string(), bounds);
                }
            }
        }
        if let Some(where_clause) = &generics.where_clause {
            for pred in &where_clause.predicates {
                if let syn::WherePredicate::Type(pt) = pred {
                    if let syn::Type::Path(tp) = &pt.bounded_ty {
                        if tp.qself.is_none() && tp.path.segments.len() == 1 {
                            if let Some(seg) = tp.path.segments.first() {
                                let param = seg.ident.to_string();
                                let mut bounds = Vec::new();
                                for b in &pt.bounds {
                                    if let syn::TypeParamBound::Trait(tb) = b {
                                        if let Some(last) = tb.path.segments.last() {
                                            bounds.push(last.ident.to_string());
                                        }
                                    }
                                }
                                if !bounds.is_empty() {
                                    map.entry(param).or_default().extend(bounds);
                                }
                            }
                        }
                    }
                }
            }
        }
        if !map.is_empty() {
            self.maps.bounds.push((start, end, map));
        }
    }
}

/// Last type-path segment, peeling `&`/`&mut` (`Self` preserved so
/// callers can normalise it against the impl self type).
fn ty_tail(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(tp) => {
            if tp.qself.is_some() {
                return None;
            }
            tp.path.segments.last().map(|s| s.ident.to_string())
        }
        syn::Type::Reference(r) => ty_tail(&r.elem),
        _ => None,
    }
}

fn sig_ret_tail(sig: &syn::Signature) -> Option<String> {
    match &sig.output {
        syn::ReturnType::Type(_, ty) => ty_tail(ty),
        syn::ReturnType::Default => None,
    }
}

/// Chain verdict for one final-method receiver.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChainVerdict {
    /// Receiver type proven; the caller checks it against the domain.
    Proven(String),
    /// Generic-bound proof: the single bound names the trait itself —
    /// the caller claims without consulting `domain_self`.
    ProvenTrait,
    /// Deeper than the one-hop cap (`S::make().foo().run()`).
    Depth,
    /// Shape recognised but the proof is missing (unknown ctor,
    /// unstated intermediate return).
    Unproven,
    /// Multi-bound generic root (`T: Tr + Other`) — `ambiguous-bound`.
    Ambiguous,
    /// Not a chain shape at all — the walker's existing evidence owns it.
    Decline,
}

/// Resolve the receiver of a final `.old()` call:
/// `C::ctor(…)` (ctor map), `base.m(…)` one-hop chains (return map),
/// or a generic-bound root (bounds map, trait domain only).
/// `self_ty_at`/`trait_top` mirror the walker's `self` handling so
/// `self.clone().run()` proves through the enclosing `impl S`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_chain_type(
    recv: &syn::Expr,
    line: u32,
    anchors: &AnchorIndex,
    maps: &ChainMaps,
    trait_name: &str,
    self_ty_at: Option<&str>,
    trait_top: Option<&str>,
    is_inherent: bool,
) -> ChainVerdict {
    // Generic-bound bare root: `t` with `anchor_of(t) = T`.
    if let syn::Expr::Path(p) = recv {
        if p.qself.is_none() && p.path.segments.len() == 1 {
            if let Some(root) = p.path.segments.first() {
                let r = root.ident.to_string();
                if r != "self" && r != "Self" {
                    if let Some(anchor_ty) = anchors.anchor_of(&r, line) {
                        if !is_inherent {
                            if let Some(bounds) = maps.bounds_at(&anchor_ty, line) {
                                if bounds.len() == 1 && bounds[0] == trait_name {
                                    return ChainVerdict::ProvenTrait;
                                }
                                if bounds.len() > 1 {
                                    return ChainVerdict::Ambiguous;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    match recv {
        syn::Expr::Call(call) => {
            // `C::ctor(…)` — func path with ≥2 segments, last in CTOR_NAMES.
            if let syn::Expr::Path(fp) = call.func.as_ref() {
                if fp.qself.is_none() && fp.path.segments.len() >= 2 {
                    let segs: Vec<String> = fp
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect();
                    let ctor = segs.last().cloned().unwrap_or_default();
                    if CTOR_NAMES.contains(&ctor.as_str()) {
                        let container = segs[segs.len() - 2].clone();
                        if maps.ctors.contains(&(container.clone(), ctor)) {
                            return ChainVerdict::Proven(container);
                        }
                        return ChainVerdict::Unproven;
                    }
                }
            }
            ChainVerdict::Decline
        }
        syn::Expr::MethodCall(inner) => {
            // One-hop cap: a chained base (`S::make().foo().run()`)
            // is `chain-depth`, never a guess.
            if !matches!(
                inner.receiver.as_ref(),
                syn::Expr::Path(_) | syn::Expr::Field(_)
            ) {
                return ChainVerdict::Depth;
            }
            let Some(base_ty) = base_concrete_type(
                &inner.receiver,
                line,
                anchors,
                maps,
                trait_name,
                self_ty_at,
                trait_top,
                is_inherent,
            ) else {
                return ChainVerdict::Decline;
            };
            let mid = inner.method.to_string();
            match maps.returns.get(&(base_ty.clone(), mid)) {
                Some(ret) => ChainVerdict::Proven(ret.clone()),
                None => ChainVerdict::Unproven,
            }
        }
        _ => ChainVerdict::Decline,
    }
}

/// Concrete type of a one-hop base (`a`, `self`, `a.field` via the
/// existing root shortcut, or a generic-bound root). `None` when the
/// existing anchor order proves nothing — the walker's evidence owns it.
#[allow(clippy::too_many_arguments)]
fn base_concrete_type(
    base: &syn::Expr,
    line: u32,
    anchors: &AnchorIndex,
    maps: &ChainMaps,
    trait_name: &str,
    self_ty_at: Option<&str>,
    trait_top: Option<&str>,
    is_inherent: bool,
) -> Option<String> {
    match base {
        syn::Expr::Path(p) => {
            if p.qself.is_some() || p.path.segments.len() != 1 {
                return None;
            }
            let root = p.path.segments.first()?.ident.to_string();
            if root == "self" || root == "Self" {
                if let Some(s) = self_ty_at {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
                if trait_top == Some(trait_name) && !trait_name.is_empty() {
                    return Some(trait_name.to_string());
                }
                return None;
            }
            let anchor_ty = anchors.anchor_of(&root, line)?;
            if !is_inherent {
                if let Some(bounds) = maps.bounds_at(&anchor_ty, line) {
                    if bounds.len() == 1 && bounds[0] == trait_name {
                        return Some(trait_name.to_string());
                    }
                }
            }
            Some(anchor_ty)
        }
        syn::Expr::Field(_) => crate::use_graph_anchor::field_root(base)
            .and_then(|root| anchors.anchor_of(&root, line)),
        _ => None,
    }
}

/// Walker-ready verdict: `Claim` rewrites, `ClaimType` rewrites iff
/// the walker proves the type in its domain, `Skip` records its
/// reason, `Unproven` falls back to the walker's shadowed/off-target
/// evidence.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChainAction {
    Claim,
    ClaimType(String),
    Skip(&'static str),
    Unproven,
}

/// Single entry point for the walker in `use_graph_method`: runs the
/// ambiguous-bound fast path, then [`resolve_chain_type`], mapping
/// the verdict to a [`ChainAction`] (the domain check stays with the
/// walker, which owns `domain_self`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn chain_action(
    recv: &syn::Expr,
    line: u32,
    root: Option<&str>,
    anchors: &AnchorIndex,
    maps: &ChainMaps,
    trait_name: &str,
    self_ty_at: Option<&str>,
    trait_top: Option<&str>,
    is_inherent: bool,
) -> ChainAction {
    if let Some(r) = root {
        if ambiguous_bound_at(r, line, anchors, maps, is_inherent) {
            return ChainAction::Skip(reason::AMBIGUOUS_BOUND);
        }
    }
    match resolve_chain_type(
        recv,
        line,
        anchors,
        maps,
        trait_name,
        self_ty_at,
        trait_top,
        is_inherent,
    ) {
        ChainVerdict::Proven(ty) => ChainAction::ClaimType(ty),
        ChainVerdict::ProvenTrait => ChainAction::Claim,
        v @ (ChainVerdict::Depth | ChainVerdict::Unproven | ChainVerdict::Ambiguous) => {
            ChainAction::Skip(chain_skip_reason(&v))
        }
        ChainVerdict::Decline => ChainAction::Unproven,
    }
}

/// Map a chain verdict to the walker's evidence reason.
pub(crate) fn chain_skip_reason(v: &ChainVerdict) -> &'static str {
    match v {
        ChainVerdict::Depth => reason::CHAIN_DEPTH,
        ChainVerdict::Unproven => reason::CHAIN_UNPROVEN,
        ChainVerdict::Ambiguous => reason::AMBIGUOUS_BOUND,
        _ => reason::OFF_TARGET,
    }
}

/// Render one chain skip as a [`crate::follow_reasons::FollowSkip`].
/// Single call so `use_graph_method.rs` stays under the D5 gate.
pub(crate) fn push_chain_skip(
    skipped: &mut Vec<crate::follow_reasons::FollowSkip>,
    path: &str,
    old: &str,
    line: u32,
    reason: &'static str,
) {
    skipped.push(crate::follow_reasons::FollowSkip::new(
        format!("{path}:{line}: unanchored `{old}` call left untouched ({reason})"),
        path,
        line,
        reason,
    ));
}

/// Bare-path generic check for the walker's direct-anchor path:
/// `t.run()` where `anchor_of(t) = T`, `T` bounded twice or more.
pub(crate) fn ambiguous_bound_at(
    root: &str,
    line: u32,
    anchors: &AnchorIndex,
    maps: &ChainMaps,
    is_inherent: bool,
) -> bool {
    if is_inherent || root == "self" || root == "Self" {
        return false;
    }
    let Some(anchor_ty) = anchors.anchor_of(root, line) else {
        return false;
    };
    matches!(maps.bounds_at(&anchor_ty, line), Some(b) if b.len() > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_str(src).expect("test src must parse")
    }

    const CTOR_SRC: &str = "struct S;\nstruct U;\nimpl S {\n    fn make() -> S { S }\n    fn new2() -> Self { S }\n    fn bad() -> U { U }\n    fn noret(&self) {}\n}\nimpl U {\n    fn make() -> U { U }\n}\n";

    #[test]
    fn ctor_map_proves_self_returns() {
        let maps = ChainMaps::for_file(&parse(CTOR_SRC));
        assert!(maps.ctors.contains(&("S".to_string(), "make".to_string())));
        assert!(!maps.ctors.contains(&("S".to_string(), "bad".to_string())));
        assert!(maps.ctors.contains(&("U".to_string(), "make".to_string())));
        // `new2` is not a candidate name, but `Self` returns still land
        // in the return map.
        assert_eq!(
            maps.returns.get(&("S".to_string(), "new2".to_string())),
            Some(&"S".to_string())
        );
        assert!(!maps
            .returns
            .contains_key(&("S".to_string(), "noret".to_string())));
    }

    const BOUND_SRC: &str = "trait Tr {}\ntrait Other {}\nfn f<T: Tr>(t: T) {\n    t;\n}\nfn g<T: Tr + Other>(t: T) {\n    t;\n}\nfn h<T>(t: T) where T: Tr {\n    t;\n}\n";

    #[test]
    fn bounds_map_inline_and_where() {
        let maps = ChainMaps::for_file(&parse(BOUND_SRC));
        assert_eq!(maps.bounds.len(), 3);
        let single = maps.bounds_at("T", 3).expect("fn f scope");
        assert_eq!(single, &vec!["Tr".to_string()]);
        let multi = maps.bounds_at("T", 6).expect("fn g scope");
        assert_eq!(multi.len(), 2);
        let wh = maps.bounds_at("T", 9).expect("fn h scope");
        assert_eq!(wh, &vec!["Tr".to_string()]);
        assert!(maps.bounds_at("T", 1).is_none());
    }

    #[test]
    fn chain_skip_reasons_pinned() {
        assert_eq!(chain_skip_reason(&ChainVerdict::Depth), "chain-depth");
        assert_eq!(chain_skip_reason(&ChainVerdict::Unproven), "chain-unproven");
        assert_eq!(
            chain_skip_reason(&ChainVerdict::Ambiguous),
            "ambiguous-bound"
        );
    }
}
