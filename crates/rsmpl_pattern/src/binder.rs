//! WI-3: binder identity keyed by binding, not by spelling.
//!
//! A single entry point over the compiled pattern: any minus/plus pair
//! that renames exactly one bound binder — however spelled (`let`,
//! closure-param, match-arm, `for` / `if let` / `while let`, or a
//! closure parameter nested in a `let` init, the F3 spelling) — yields
//! a [`BinderRename`]. Spellings that cannot prove single-binder
//! identity (destructuring deltas, init/type/body changes, `move`
//! deltas, multi-binder diffs, bare-`P` annotation add/drop) yield
//! `None` and keep general-template behaviour with no binder
//! privileges (SPEC 18.1, D11).

use std::collections::HashMap;

use crate::{GeneralShape, PatternKind};

/// Which syntactic position the renamed binder occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinderScope {
    /// `let old = E;` → `let new = E;`.
    Let,
    /// `|old| Body` → `|new| Body`.
    Closure,
    /// `match S { old => B, … }` → `match S { new => B, … }`.
    MatchArm,
    /// `for old in E { … }` → `for new in E { … }`.
    For,
    /// `if let old = E { … }` → `if let new = E { … }`.
    IfLet,
    /// `while let old = E { … }` → `while let new = E { … }`.
    WhileLet,
    /// F3: `let g = |old| E;` → `let g = |sum| E;` — the renamed binder
    /// lives in the `let` init closure; the outer `let` name is unchanged.
    LetClosure,
    /// A same-name `fn` header with one renamed parameter. The pattern
    /// crate never produces this from a kind alone (params live in
    /// `FnPattern`, and the hunk is needed); the engine fills it in.
    FnParam,
}

/// A compiled single-binder rename: the old/new ident plus the scope
/// whose unshadowed value uses complete or become orphans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinderRename {
    pub old: String,
    pub new: String,
    pub scope: BinderScope,
}

/// The single binder entry: `Some` iff `kind` renames exactly one bound
/// binder and nothing else in the pattern changes.
pub fn binder_rename(kind: &PatternKind) -> Option<BinderRename> {
    let (old, new, scope) = let_binder(kind)
        .map(|(o, n)| (o, n, BinderScope::Let))
        .or_else(|| closure_param(kind).map(|(o, n)| (o, n, BinderScope::Closure)))
        .or_else(|| match_arm(kind).map(|(o, n)| (o, n, BinderScope::MatchArm)))
        .or_else(|| for_loop(kind).map(|(o, n)| (o, n, BinderScope::For)))
        .or_else(|| if_let(kind).map(|(o, n)| (o, n, BinderScope::IfLet)))
        .or_else(|| while_let(kind).map(|(o, n)| (o, n, BinderScope::WhileLet)))
        .or_else(|| let_closure_param(kind).map(|(o, n)| (o, n, BinderScope::LetClosure)))?;
    Some(BinderRename { old, new, scope })
}

/// Placeholder map (`__RSMPL_*` → declared name) for a general pattern.
fn ph_map(kind_placeholders: &[(String, String)]) -> HashMap<&str, &str> {
    kind_placeholders
        .iter()
        .map(|(name, ph)| (ph.as_str(), name.as_str()))
        .collect()
}

/// Map `__RSMPL_*` placeholder idents back to declared names, one whole
/// ident token at a time (single-hole `__RSMPL_E` is a string prefix of
/// the `...` marker `__RSMPL_ELLIPSIS_0`, so plain `str::replace`
/// would mangle markers).
fn normalize_placeholders(s: &str, ph_map: &HashMap<&str, &str>) -> String {
    if ph_map.is_empty() || !s.contains("__RSMPL_") {
        return s.to_string();
    }
    let mut keys: Vec<(&str, &str)> = ph_map.iter().map(|(k, v)| (*k, *v)).collect();
    keys.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let mut matched: Option<(usize, &str)> = None;
        for (ph, name) in &keys {
            let pb = ph.as_bytes();
            if i + pb.len() > bytes.len() {
                continue;
            }
            if &bytes[i..i + pb.len()] != pb {
                continue;
            }
            let prev_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let next_ok = i + pb.len() == bytes.len() || !is_ident_byte(bytes[i + pb.len()]);
            if prev_ok && next_ok {
                matched = Some((pb.len(), *name));
                break;
            }
        }
        if let Some((len, name)) = matched {
            out.push_str(name);
            i += len;
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// General-shape accessor: `(minus_sub, plus_body, placeholders)` when
/// `kind` is a `General` hunk of the wanted shape with equal attrs.
type GeneralParts<'a> = (&'a str, &'a str, &'a [(String, String)]);
fn general_parts(kind: &PatternKind, want: GeneralShape) -> Option<GeneralParts<'_>> {
    let PatternKind::General {
        shape,
        minus_sub,
        plus_body,
        placeholders,
        minus_attrs,
        plus_attrs,
        ..
    } = kind
    else {
        return None;
    };
    if *shape != want || *minus_attrs != *plus_attrs {
        return None;
    }
    Some((minus_sub, plus_body, placeholders))
}

/// Single `x` binder (no subpattern); returns `(ident, has_mut)`.
/// Placeholder idents (`__RSMPL_*`) never count as binders.
fn single_ident_binder(pat: &syn::Pat) -> Option<(String, bool)> {
    match pat {
        syn::Pat::Ident(pid) => {
            if pid.subpat.is_some() {
                return None;
            }
            let name = pid.ident.to_string();
            if name.starts_with("__RSMPL_") {
                return None;
            }
            Some((name, pid.mutability.is_some()))
        }
        syn::Pat::Type(pt) => single_ident_binder(&pt.pat),
        _ => None,
    }
}

/// Type-annotation tokens of a pattern (`""` when unannotated).
/// NOTE: field access must happen outside `quote!` — `#pt.ty` would
/// interpolate `pt` plus a literal `.ty` (and the binder name with it,
/// so same-type renames would falsely veto).
fn pat_ty(pat: &syn::Pat, ph: &HashMap<&str, &str>) -> String {
    match pat {
        syn::Pat::Type(pt) => {
            let ty = &*pt.ty;
            normalize_placeholders(&quote::quote!(#ty).to_string(), ph)
        }
        _ => String::new(),
    }
}

fn render<T: quote::ToTokens>(node: &T) -> String {
    quote::quote!(#node).to_string()
}

/// Parse a general-expression pattern forgiving the block-tail `...`
/// ellipsis (not valid `syn` on its own): retry with the engine's
/// ellipsis-marker ident, which compares equal on both sides when both
/// were `...`.
fn parse_expr_forgiving(s: &str) -> Option<syn::Expr> {
    if let Ok(e) = syn::parse_str::<syn::Expr>(s) {
        return Some(e);
    }
    let fixed = s.replace("...", "__RSMPL_ELLIPSIS_0");
    syn::parse_str::<syn::Expr>(&fixed).ok()
}

/// `(old, new)` for a plain `let old = E;` → `let new = E;` rename with
/// identical init and type annotation.
fn let_binder(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Stmt)?;
    let ph = ph_map(placeholders);
    let Ok(syn::Stmt::Local(minus)) = syn::parse_str::<syn::Stmt>(minus_sub) else {
        return None;
    };
    let Ok(syn::Stmt::Local(plus)) = syn::parse_str::<syn::Stmt>(plus_body) else {
        return None;
    };
    let (old_id, old_mut) = single_ident_binder(&minus.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    if pat_ty(&minus.pat, &ph) != pat_ty(&plus.pat, &ph) {
        return None;
    }
    let minus_init = minus.init.as_ref().map(|i| render(&i.expr))?;
    let plus_init = plus.init.as_ref().map(|i| render(&i.expr))?;
    if normalize_placeholders(&minus_init, &ph) != normalize_placeholders(&plus_init, &ph) {
        return None;
    }
    Some((old_id, new_id))
}

/// Shared closure-parameter diff: exactly one single-ident parameter
/// may differ (same annotation discipline as `let`); qualifiers must
/// match; bodies are compared by the caller.
fn closure_param_diff(
    minus: &syn::ExprClosure,
    plus: &syn::ExprClosure,
    ph: &HashMap<&str, &str>,
) -> Option<(String, String)> {
    if render(&minus.asyncness) != render(&plus.asyncness)
        || render(&minus.movability) != render(&plus.movability)
        || render(&minus.capture) != render(&plus.capture)
        || render(&minus.lifetimes) != render(&plus.lifetimes)
        || render(&minus.constness) != render(&plus.constness)
        || render(&minus.output) != render(&plus.output)
        || minus.inputs.len() != plus.inputs.len()
    {
        return None;
    }
    let mut diff: Option<(String, String)> = None;
    for (mp, pp) in minus.inputs.iter().zip(plus.inputs.iter()) {
        let mn = normalize_placeholders(&render(mp), ph);
        let pn = normalize_placeholders(&render(pp), ph);
        if mn == pn {
            continue;
        }
        let (old_id, old_mut) = single_ident_binder(mp)?;
        let (new_id, new_mut) = single_ident_binder(pp)?;
        if old_id == new_id || old_mut != new_mut {
            return None;
        }
        if pat_ty(mp, ph) != pat_ty(pp, ph) {
            return None;
        }
        if diff.replace((old_id, new_id)).is_some() {
            return None;
        }
    }
    diff
}

/// `(old, new)` for `|old| Body` → `|new| Body` with
/// placeholder-normalized-equal bodies.
fn closure_param(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Ok(syn::Expr::Closure(minus)) = syn::parse_str::<syn::Expr>(minus_sub) else {
        return None;
    };
    let Ok(syn::Expr::Closure(plus)) = syn::parse_str::<syn::Expr>(plus_body) else {
        return None;
    };
    let (old, new) = closure_param_diff(&minus, &plus, &ph)?;
    if normalize_placeholders(&render(&*minus.body), &ph)
        != normalize_placeholders(&render(&*plus.body), &ph)
    {
        return None;
    }
    Some((old, new))
}

/// `(old, new)` for `match S { old => B, … }` → `match S { new => B, … }`.
fn match_arm(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Ok(syn::Expr::Match(minus)) = syn::parse_str::<syn::Expr>(minus_sub) else {
        return None;
    };
    let Ok(syn::Expr::Match(plus)) = syn::parse_str::<syn::Expr>(plus_body) else {
        return None;
    };
    if normalize_placeholders(&render(&*minus.expr), &ph)
        != normalize_placeholders(&render(&*plus.expr), &ph)
        || minus.arms.len() != plus.arms.len()
    {
        return None;
    }
    let mut diff: Option<(String, String)> = None;
    for (ma, pa) in minus.arms.iter().zip(plus.arms.iter()) {
        let mn = normalize_placeholders(&render(ma), &ph);
        let pn = normalize_placeholders(&render(pa), &ph);
        if mn == pn {
            continue;
        }
        let (old_id, old_mut) = single_ident_binder(&ma.pat)?;
        let (new_id, new_mut) = single_ident_binder(&pa.pat)?;
        if old_id == new_id || old_mut != new_mut {
            return None;
        }
        match (&ma.guard, &pa.guard) {
            (None, None) => {}
            (Some((_, mg)), Some((_, pg))) => {
                if normalize_placeholders(&render(&**mg), &ph)
                    != normalize_placeholders(&render(&**pg), &ph)
                {
                    return None;
                }
            }
            _ => return None,
        }
        if normalize_placeholders(&render(&*ma.body), &ph)
            != normalize_placeholders(&render(&*pa.body), &ph)
        {
            return None;
        }
        if diff.replace((old_id, new_id)).is_some() {
            return None;
        }
    }
    diff
}

/// `(old, new)` for `for old in E { … }` → `for new in E { … }`.
fn for_loop(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Some(syn::Expr::ForLoop(minus)) = parse_expr_forgiving(minus_sub) else {
        return None;
    };
    let Some(syn::Expr::ForLoop(plus)) = parse_expr_forgiving(plus_body) else {
        return None;
    };
    if render(&minus.label) != render(&plus.label) {
        return None;
    }
    let (old_id, old_mut) = single_ident_binder(&minus.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    if normalize_placeholders(&render(&*minus.expr), &ph)
        != normalize_placeholders(&render(&*plus.expr), &ph)
        || normalize_placeholders(&render(&minus.body), &ph)
            != normalize_placeholders(&render(&plus.body), &ph)
    {
        return None;
    }
    Some((old_id, new_id))
}

/// `(old, new)` for `if let old = E { … } …` → `if let new = E { … } …`.
fn if_let(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Some(syn::Expr::If(minus)) = parse_expr_forgiving(minus_sub) else {
        return None;
    };
    let Some(syn::Expr::If(plus)) = parse_expr_forgiving(plus_body) else {
        return None;
    };
    let syn::Expr::Let(minus_let) = &*minus.cond else {
        return None;
    };
    let syn::Expr::Let(plus_let) = &*plus.cond else {
        return None;
    };
    let (old_id, old_mut) = single_ident_binder(&minus_let.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus_let.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    if normalize_placeholders(&render(&*minus_let.expr), &ph)
        != normalize_placeholders(&render(&*plus_let.expr), &ph)
        || normalize_placeholders(&render(&minus.then_branch), &ph)
            != normalize_placeholders(&render(&plus.then_branch), &ph)
    {
        return None;
    }
    match (&minus.else_branch, &plus.else_branch) {
        (None, None) => {}
        (Some((_, a)), Some((_, b))) => {
            if normalize_placeholders(&render(&**a), &ph)
                != normalize_placeholders(&render(&**b), &ph)
            {
                return None;
            }
        }
        _ => return None,
    }
    Some((old_id, new_id))
}

/// `(old, new)` for `while let old = E { … }` → `while let new = E { … }`.
fn while_let(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Some(syn::Expr::While(minus)) = parse_expr_forgiving(minus_sub) else {
        return None;
    };
    let Some(syn::Expr::While(plus)) = parse_expr_forgiving(plus_body) else {
        return None;
    };
    if render(&minus.label) != render(&plus.label) {
        return None;
    }
    let syn::Expr::Let(minus_let) = &*minus.cond else {
        return None;
    };
    let syn::Expr::Let(plus_let) = &*plus.cond else {
        return None;
    };
    let (old_id, old_mut) = single_ident_binder(&minus_let.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus_let.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    if normalize_placeholders(&render(&*minus_let.expr), &ph)
        != normalize_placeholders(&render(&*plus_let.expr), &ph)
        || normalize_placeholders(&render(&minus.body), &ph)
            != normalize_placeholders(&render(&plus.body), &ph)
    {
        return None;
    }
    Some((old_id, new_id))
}

/// F3 headline: `(old, new)` for `let g = |old| E;` → `let g = |new| E;` —
/// the outer `let` name is unchanged, exactly one closure parameter in
/// the init is renamed, and init bodies are placeholder-equal.
fn let_closure_param(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Stmt)?;
    let ph = ph_map(placeholders);
    let Ok(syn::Stmt::Local(minus)) = syn::parse_str::<syn::Stmt>(minus_sub) else {
        return None;
    };
    let Ok(syn::Stmt::Local(plus)) = syn::parse_str::<syn::Stmt>(plus_body) else {
        return None;
    };
    // Outer `let` binder must be identical (this is not a `let` rename).
    if render(&minus.pat) != render(&plus.pat) {
        return None;
    }
    let minus_init = minus.init.as_ref().map(|i| render(&i.expr))?;
    let plus_init = plus.init.as_ref().map(|i| render(&i.expr))?;
    // A plain `let` rename has token-equal inits; here the inits differ
    // only in the closure parameter, so equal inits veto (owned by `let`).
    if normalize_placeholders(&minus_init, &ph) == normalize_placeholders(&plus_init, &ph) {
        return None;
    }
    let minus_expr = minus.init.as_ref().map(|i| &*i.expr)?;
    let plus_expr = plus.init.as_ref().map(|i| &*i.expr)?;
    let (Some(minus_cl), Some(plus_cl)) = (as_closure(minus_expr), as_closure(plus_expr)) else {
        return None;
    };
    let (old, new) = closure_param_diff(minus_cl, plus_cl, &ph)?;
    if normalize_placeholders(&render(&*minus_cl.body), &ph)
        != normalize_placeholders(&render(&*plus_cl.body), &ph)
    {
        return None;
    }
    Some((old, new))
}

/// Unwrap one layer: closures directly in the init, or parenthesised.
fn as_closure(expr: &syn::Expr) -> Option<&syn::ExprClosure> {
    match expr {
        syn::Expr::Closure(c) => Some(c),
        syn::Expr::Paren(p) => as_closure(&p.expr),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmt(minus_sub: &str, plus_body: &str) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Stmt,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders: vec![("E".to_string(), "__RSMPL_E0".to_string())],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }

    fn expr(minus_sub: &str, plus_body: &str) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Expr,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders: vec![("E".to_string(), "__RSMPL_E0".to_string())],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }

    fn renamed(kind: &PatternKind) -> Option<(String, String, BinderScope)> {
        binder_rename(kind).map(|b| (b.old, b.new, b.scope))
    }

    #[test]
    fn let_fires() {
        assert_eq!(
            renamed(&stmt("let total = __RSMPL_E0;", "let sum = E;")),
            Some(("total".to_string(), "sum".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn annotated_same_fires() {
        assert_eq!(
            renamed(&stmt("let total : u32 = __RSMPL_E0;", "let sum : u32 = E;")),
            Some(("total".to_string(), "sum".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn closure_fires() {
        assert_eq!(
            renamed(&expr("|total| __RSMPL_E0", "|sum| E")),
            Some(("total".to_string(), "sum".to_string(), BinderScope::Closure))
        );
    }

    #[test]
    fn match_arm_fires() {
        let kind = PatternKind::General {
            shape: GeneralShape::Expr,
            minus_sub: "match __RSMPL_E0 { total => __RSMPL_E1 , _ => 0 }".to_string(),
            plus_raw: "match E { sum => B , _ => 0 }".to_string(),
            plus_body: "match E { sum => B , _ => 0 }".to_string(),
            placeholders: vec![
                ("E".to_string(), "__RSMPL_E0".to_string()),
                ("B".to_string(), "__RSMPL_E1".to_string()),
            ],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        };
        assert_eq!(
            renamed(&kind),
            Some((
                "total".to_string(),
                "sum".to_string(),
                BinderScope::MatchArm
            ))
        );
    }

    #[test]
    fn for_fires() {
        assert_eq!(
            renamed(&expr(
                "for total in __RSMPL_E0 { ... }",
                "for sum in E { ... }"
            )),
            Some(("total".to_string(), "sum".to_string(), BinderScope::For))
        );
    }

    #[test]
    fn if_let_fires() {
        assert_eq!(
            renamed(&expr(
                "if let total = __RSMPL_E0 { ... }",
                "if let sum = E { ... }"
            )),
            Some(("total".to_string(), "sum".to_string(), BinderScope::IfLet))
        );
    }

    #[test]
    fn while_let_fires() {
        assert_eq!(
            renamed(&expr(
                "while let total = __RSMPL_E0 { ... }",
                "while let sum = E { ... }"
            )),
            Some((
                "total".to_string(),
                "sum".to_string(),
                BinderScope::WhileLet
            ))
        );
    }

    #[test]
    fn let_closure_fires_headline() {
        // F3: the escaping spelling now has an identity.
        assert_eq!(
            renamed(&stmt(
                "let g = | total | __RSMPL_E0 ;",
                "let g = | sum | E ;"
            )),
            Some((
                "total".to_string(),
                "sum".to_string(),
                BinderScope::LetClosure
            ))
        );
    }

    #[test]
    fn plain_let_rename_is_not_let_closure() {
        // Token-equal inits belong to `Let`, never `LetClosure`.
        assert_eq!(
            renamed(&stmt("let total = __RSMPL_E0;", "let sum = E;")).map(|r| r.2),
            Some(BinderScope::Let)
        );
    }

    #[test]
    fn init_change_vetoes() {
        assert_eq!(
            binder_rename(&stmt("let total = __RSMPL_E0;", "let sum = 2;")),
            None
        );
    }

    #[test]
    fn closure_body_change_vetoes() {
        assert_eq!(
            binder_rename(&expr("|total| __RSMPL_E0", "|sum| E + 1")),
            None
        );
    }

    #[test]
    fn move_delta_vetoes() {
        assert_eq!(
            binder_rename(&expr("move |total| __RSMPL_E0", "|sum| E")),
            None
        );
    }

    #[test]
    fn destructuring_vetoes() {
        assert_eq!(
            binder_rename(&stmt("let (a , b) = __RSMPL_E0;", "let (a , c) = E;")),
            None
        );
    }

    #[test]
    fn outer_let_change_vetoes_let_closure() {
        // Renaming the outer `let` at the same time is not a binder
        // rename at all (two binders move).
        assert_eq!(
            binder_rename(&stmt(
                "let g = | total | __RSMPL_E0 ;",
                "let h = | sum | E ;"
            )),
            None
        );
    }

    fn norm(s: &str) -> String {
        let mut map = HashMap::new();
        map.insert("__RSMPL_E0", "E");
        map.insert("__RSMPL_E", "F");
        map.insert("__RSMPL_ELLIPSIS_0", "...");
        normalize_placeholders(s, &map)
    }

    #[test]
    fn placeholder_normalization_is_token_exact() {
        // Single-hole `__RSMPL_E` is a string prefix of the `...` marker
        // `__RSMPL_ELLIPSIS_0`: longest-first, whole-token only.
        assert_eq!(norm("__RSMPL_E"), "F");
        assert_eq!(norm("__RSMPL_ELLIPSIS_0"), "...");
        assert_eq!(norm("x__RSMPL_E"), "x__RSMPL_E");
        assert_eq!(
            norm("for total in __RSMPL_E { __RSMPL_ELLIPSIS_0 }"),
            "for total in F { ... }"
        );
    }

    #[test]
    fn non_binder_template_vetoes() {
        assert_eq!(binder_rename(&expr("total + __RSMPL_E0", "sum + E")), None);
    }

    #[test]
    fn bare_annotation_drop_vetoes() {
        assert_eq!(
            binder_rename(&expr("|total : u32| __RSMPL_E0", "|sum| E")),
            None
        );
    }

    #[test]
    fn two_binder_diff_vetoes() {
        assert_eq!(
            binder_rename(&expr("|total , other| __RSMPL_E0", "|sum , changed| E")),
            None
        );
    }
}
