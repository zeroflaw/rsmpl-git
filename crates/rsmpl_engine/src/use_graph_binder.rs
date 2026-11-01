//! WI-2d: remaining binder-kind detectors (IMPROVE.md WI-2 residue).
//!
//! Sibling of [`crate::use_graph_local`] (D5: new code lands in new
//! modules). Detects single-binder renames whose holes preserve the
//! body, so leftover uses dangle with no orphan duty today:
//! `for old in …`, `if let` / `while let` patterns (general-expression
//! shapes), and same-name `fn` headers with one renamed parameter.
//! Destructuring, scrutinee/iter/body/label/else deltas, and
//! multi-binder diffs veto (today's behaviour is kept). No completion,
//! no schema change.

use std::collections::HashMap;

use rsmpl_pattern::{GeneralShape, PatternKind};

use crate::use_graph_local::single_ident_binder;
use crate::use_graph_normalize::normalize_placeholders;

/// Parse a general-expression pattern forgiving the block-tail `...`
/// ellipsis (not valid `syn` on its own): retry with the engine's
/// ellipsis-marker ident, which parses as a plain expression statement
/// and compares equal on both sides when both were `...`.
fn parse_expr_forgiving(s: &str) -> Option<syn::Expr> {
    if let Ok(e) = syn::parse_str::<syn::Expr>(s) {
        return Some(e);
    }
    let fixed = s.replace("...", "__RSMPL_ELLIPSIS_0");
    syn::parse_str::<syn::Expr>(&fixed).ok()
}

/// True when `compiled` is a WI-2d binder rename: a `for` / `if let` /
/// `while let` binder-only diff (general-expression shape). The fn-param
/// case needs the hunk (params live in `FnPattern`, not in
/// `PatternKind::Fn`), so it is owned by
/// [`fn_param_binder_rename_from_hunk`] instead.
pub fn is_wi2d_expr_binder_rename(compiled: &rsmpl_pattern::CompiledPattern) -> bool {
    for_loop_binder_rename(&compiled.kind).is_some()
        || if_let_binder_rename(&compiled.kind).is_some()
        || while_let_binder_rename(&compiled.kind).is_some()
}

/// `(old, new)` when `kind` is a `for`-loop binder rename —
/// `for old in E { ... }` → `for new in E { ... }` with the same
/// iterator and body (placeholder-normalized). Labels must agree; only
/// a single-ident pattern may change.
pub fn for_loop_binder_rename(kind: &PatternKind) -> Option<(String, String)> {
    let PatternKind::General {
        shape: GeneralShape::Expr,
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
    if *minus_attrs != *plus_attrs {
        return None;
    }
    let Some(syn::Expr::ForLoop(minus)) = parse_expr_forgiving(minus_sub) else {
        return None;
    };
    let Some(syn::Expr::ForLoop(plus)) = parse_expr_forgiving(plus_body) else {
        return None;
    };
    if crate::matcher_general::render(&minus.label) != crate::matcher_general::render(&plus.label) {
        return None;
    }
    let (old_id, old_mut) = single_ident_binder(&minus.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    let ph_map: HashMap<&str, &str> = placeholders
        .iter()
        .map(|(name, ph)| (ph.as_str(), name.as_str()))
        .collect();
    let minus_iter = crate::matcher_general::render(&*minus.expr);
    let plus_iter = crate::matcher_general::render(&*plus.expr);
    if normalize_placeholders(&minus_iter, &ph_map) != normalize_placeholders(&plus_iter, &ph_map) {
        return None;
    }
    let minus_body = crate::matcher_general::render(&minus.body);
    let plus_body_r = crate::matcher_general::render(&plus.body);
    if normalize_placeholders(&minus_body, &ph_map) != normalize_placeholders(&plus_body_r, &ph_map)
    {
        return None;
    }
    Some((old_id, new_id))
}

/// `(old, new)` when `kind` is an `if let` binder rename —
/// `if let old = E { ... } …` → `if let new = E { ... } …` with the
/// same scrutinee, then-branch, and else-branch. The condition must be
/// a direct `let` on both sides (chains vetoed); labels must agree.
pub fn if_let_binder_rename(kind: &PatternKind) -> Option<(String, String)> {
    let PatternKind::General {
        shape: GeneralShape::Expr,
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
    if *minus_attrs != *plus_attrs {
        return None;
    }
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
    let ph_map: HashMap<&str, &str> = placeholders
        .iter()
        .map(|(name, ph)| (ph.as_str(), name.as_str()))
        .collect();
    let minus_scrut = crate::matcher_general::render(&*minus_let.expr);
    let plus_scrut = crate::matcher_general::render(&*plus_let.expr);
    if normalize_placeholders(&minus_scrut, &ph_map) != normalize_placeholders(&plus_scrut, &ph_map)
    {
        return None;
    }
    let minus_then = crate::matcher_general::render(&minus.then_branch);
    let plus_then = crate::matcher_general::render(&plus.then_branch);
    if normalize_placeholders(&minus_then, &ph_map) != normalize_placeholders(&plus_then, &ph_map) {
        return None;
    }
    match (&minus.else_branch, &plus.else_branch) {
        (None, None) => {}
        (Some((_, a)), Some((_, b))) => {
            let ma = crate::matcher_general::render(&**a);
            let pb = crate::matcher_general::render(&**b);
            if normalize_placeholders(&ma, &ph_map) != normalize_placeholders(&pb, &ph_map) {
                return None;
            }
        }
        _ => return None,
    }
    Some((old_id, new_id))
}

/// `(old, new)` when `kind` is a `while let` binder rename —
/// `while let old = E { ... }` → `while let new = E { ... }` with the
/// same scrutinee and body. Labels must agree.
pub fn while_let_binder_rename(kind: &PatternKind) -> Option<(String, String)> {
    let PatternKind::General {
        shape: GeneralShape::Expr,
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
    if *minus_attrs != *plus_attrs {
        return None;
    }
    let Some(syn::Expr::While(minus)) = parse_expr_forgiving(minus_sub) else {
        return None;
    };
    let Some(syn::Expr::While(plus)) = parse_expr_forgiving(plus_body) else {
        return None;
    };
    if crate::matcher_general::render(&minus.label) != crate::matcher_general::render(&plus.label) {
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
    let ph_map: HashMap<&str, &str> = placeholders
        .iter()
        .map(|(name, ph)| (ph.as_str(), name.as_str()))
        .collect();
    let minus_scrut = crate::matcher_general::render(&*minus_let.expr);
    let plus_scrut = crate::matcher_general::render(&*plus_let.expr);
    if normalize_placeholders(&minus_scrut, &ph_map) != normalize_placeholders(&plus_scrut, &ph_map)
    {
        return None;
    }
    let minus_body = crate::matcher_general::render(&minus.body);
    let plus_body_r = crate::matcher_general::render(&plus.body);
    if normalize_placeholders(&minus_body, &ph_map) != normalize_placeholders(&plus_body_r, &ph_map)
    {
        return None;
    }
    Some((old_id, new_id))
}

/// `(old, new)` when `hunk` renames exactly one `fn` parameter and
/// nothing else: same fn name, all other signature slots equal, both
/// bodies `...`, both param lists concrete with the same arity and
/// pairwise-equal params except one single-ident `old: T` → `new: T`
/// pair (same type). Anything else (name rename, slot evolution,
/// `...` params, arity/type/self/multi-param diffs) yields `None` —
/// today's behaviour is kept.
pub fn fn_param_binder_rename_from_hunk(hunk: &rsmpl_patch::RsmplHunk) -> Option<(String, String)> {
    fn_param_binder_inner(hunk)
}

fn fn_param_binder_inner(hunk: &rsmpl_patch::RsmplHunk) -> Option<(String, String)> {
    // Rebuild the full patterns exactly like the fn matcher does, then
    // compare slot-wise. Concrete-only probes carry no slot metas, so
    // empty meta lists parse them; a meta-bearing hunk fails to parse
    // here and vetoes (safe direction).
    let full_minus: String = hunk
        .lines
        .iter()
        .filter(|l| l.is_removal() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let full_plus: String = hunk
        .lines
        .iter()
        .filter(|l| l.is_addition() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let empty: Vec<String> = Vec::new();
    let minus_block = crate::matcher_fn::extract_fn_block(&full_minus, &empty);
    let plus_block = crate::matcher_fn::extract_fn_block(&full_plus, &empty);
    let minus = rsmpl_pattern::parse_fn_pattern(
        &minus_block,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        false,
    )
    .ok()?;
    let plus = rsmpl_pattern::parse_fn_pattern(
        &plus_block,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        false,
    )
    .ok()?;
    // Same-name gate: a fn rename is a different item (never a binder).
    let minus_name = match &minus.name {
        rsmpl_pattern::NamePat::Concrete(n) => n.clone(),
        rsmpl_pattern::NamePat::Meta(_) => return None,
    };
    let plus_name = match &plus.name {
        rsmpl_pattern::NamePat::Concrete(n) => n.clone(),
        rsmpl_pattern::NamePat::Meta(_) => return None,
    };
    if minus_name != plus_name {
        return None;
    }
    // All other slots equal; bodies both `...` (declarations count).
    if minus.visibility != plus.visibility
        || minus.qualifiers != plus.qualifiers
        || minus.generics != plus.generics
        || minus.ret != plus.ret
        || minus.where_clause != plus.where_clause
    {
        return None;
    }
    if !minus.body_ellipsis || !plus.body_ellipsis {
        return None;
    }
    if minus.params_ellipsis || plus.params_ellipsis {
        return None;
    }
    let minus_params = minus.params_text?;
    let plus_params = plus.params_text?;
    let minus_inputs = parse_fn_inputs(&minus_params)?;
    let plus_inputs = parse_fn_inputs(&plus_params)?;
    if minus_inputs.len() != plus_inputs.len() {
        return None;
    }
    let mut diff: Option<(String, String)> = None;
    for (mp, pp) in minus_inputs.iter().zip(plus_inputs.iter()) {
        match (mp, pp) {
            (syn::FnArg::Receiver(_), syn::FnArg::Receiver(_)) => {
                if crate::matcher_general::render(mp) != crate::matcher_general::render(pp) {
                    return None;
                }
            }
            (syn::FnArg::Typed(mt), syn::FnArg::Typed(pt)) => {
                let m = crate::matcher_general::render(mt);
                let p = crate::matcher_general::render(pt);
                if m == p {
                    continue;
                }
                let (old_id, old_mut) = single_ident_binder(&mt.pat)?;
                let (new_id, new_mut) = single_ident_binder(&pt.pat)?;
                if old_id == new_id || old_mut != new_mut {
                    return None;
                }
                if crate::matcher_general::render(&mt.ty) != crate::matcher_general::render(&pt.ty)
                {
                    return None;
                }
                if diff.replace((old_id, new_id)).is_some() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    diff
}

fn parse_fn_inputs(params: &str) -> Option<Vec<syn::FnArg>> {
    if params.trim().is_empty() {
        return Some(Vec::new());
    }
    let src = format!("fn _dummy({params}) {{}}");
    let item: syn::ItemFn = syn::parse_str(&src).ok()?;
    Some(item.sig.inputs.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_pattern::{CompiledPattern, GeneralShape};

    fn general_expr(minus_sub: &str, plus_body: &str) -> PatternKind {
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

    fn hunk_of(minus: &str, plus: &str) -> rsmpl_patch::RsmplHunk {
        let text = format!(
            "diff --git a/src/main.rs b/src/main.rs\n@@ -1,3 @@ rule=x\n- {minus}\n+ {plus}\n"
        );
        let patch = rsmpl_patch::parse_patch(&text).unwrap();
        let hunk = patch
            .files
            .into_iter()
            .next()
            .unwrap()
            .hunks
            .into_iter()
            .next()
            .unwrap();
        match hunk {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("expected RSmPL hunk"),
        }
    }

    #[test]
    fn for_plain_fires() {
        let kind = general_expr("for total in __RSMPL_E0 { ... }", "for sum in E { ... }");
        assert_eq!(
            for_loop_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
    }

    #[test]
    fn for_iter_change_vetoes() {
        let kind = general_expr(
            "for total in __RSMPL_E0 { ... }",
            "for sum in E + 1 { ... }",
        );
        assert_eq!(for_loop_binder_rename(&kind), None);
    }

    #[test]
    fn for_label_change_vetoes() {
        let kind = general_expr(
            "'a : for total in __RSMPL_E0 { ... }",
            "for sum in E { ... }",
        );
        assert_eq!(for_loop_binder_rename(&kind), None);
    }

    #[test]
    fn for_destructuring_vetoes() {
        let kind = general_expr(
            "for (a , total) in __RSMPL_E0 { ... }",
            "for (a , sum) in E { ... }",
        );
        assert_eq!(for_loop_binder_rename(&kind), None);
    }

    #[test]
    fn for_non_loop_vetoes() {
        let kind = general_expr("total + __RSMPL_E0", "sum + E");
        assert_eq!(for_loop_binder_rename(&kind), None);
    }

    #[test]
    fn if_let_plain_fires() {
        let kind = general_expr(
            "if let total = __RSMPL_E0 { ... }",
            "if let sum = E { ... }",
        );
        assert_eq!(
            if_let_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
    }

    #[test]
    fn if_let_scrutinee_change_vetoes() {
        let kind = general_expr(
            "if let total = __RSMPL_E0 { ... }",
            "if let sum = E + 1 { ... }",
        );
        assert_eq!(if_let_binder_rename(&kind), None);
    }

    #[test]
    fn if_let_else_change_vetoes() {
        let kind = general_expr(
            "if let total = __RSMPL_E0 { ... } else { 1 }",
            "if let sum = E { ... } else { 2 }",
        );
        assert_eq!(if_let_binder_rename(&kind), None);
    }

    #[test]
    fn if_let_chain_vetoes() {
        // `if let … && …` cond is not a direct `let`.
        let kind = general_expr(
            "if let total = __RSMPL_E0 && true { ... }",
            "if let sum = E && true { ... }",
        );
        // Parses as a binary cond (not `Expr::Let`) on at least one side
        // or fails to parse as `Expr::If` with let conds — either way no
        // binder claim.
        assert_eq!(if_let_binder_rename(&kind), None);
    }

    #[test]
    fn while_let_plain_fires() {
        let kind = general_expr(
            "while let total = __RSMPL_E0 { ... }",
            "while let sum = E { ... }",
        );
        assert_eq!(
            while_let_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
    }

    #[test]
    fn while_let_body_change_vetoes() {
        let kind = general_expr(
            "while let total = __RSMPL_E0 { ... }",
            "while let sum = E { break ; }",
        );
        assert_eq!(while_let_binder_rename(&kind), None);
    }

    #[test]
    fn while_plain_cond_vetoes() {
        let kind = general_expr("while __RSMPL_E0 { ... }", "while E { ... }");
        assert_eq!(while_let_binder_rename(&kind), None);
    }

    #[test]
    fn fn_param_plain_fires() {
        let h = hunk_of("fn foo(total : u32) { ... }", "fn foo(sum : u32) { ... }");
        assert_eq!(
            fn_param_binder_rename_from_hunk(&h),
            Some(("total".to_string(), "sum".to_string()))
        );
    }

    #[test]
    fn fn_param_name_change_vetoes() {
        // A fn rename is never a binder rename.
        let h = hunk_of("fn foo(total : u32) { ... }", "fn bar(sum : u32) { ... }");
        assert_eq!(fn_param_binder_rename_from_hunk(&h), None);
    }

    #[test]
    fn fn_param_type_change_vetoes() {
        let h = hunk_of("fn foo(total : u32) { ... }", "fn foo(sum : u64) { ... }");
        assert_eq!(fn_param_binder_rename_from_hunk(&h), None);
    }

    #[test]
    fn fn_param_count_change_vetoes() {
        let h = hunk_of(
            "fn foo(total : u32) { ... }",
            "fn foo(total : u32 , extra : u32) { ... }",
        );
        assert_eq!(fn_param_binder_rename_from_hunk(&h), None);
    }

    #[test]
    fn fn_param_multi_diff_vetoes() {
        let h = hunk_of(
            "fn foo(a : u32 , total : u32) { ... }",
            "fn foo(b : u32 , sum : u32) { ... }",
        );
        assert_eq!(fn_param_binder_rename_from_hunk(&h), None);
    }

    #[test]
    fn fn_param_ellipsis_vetoes() {
        let h = hunk_of("fn foo(...) { ... }", "fn foo(...) { ... }");
        assert_eq!(fn_param_binder_rename_from_hunk(&h), None);
    }

    #[test]
    fn wi2d_expr_gate_covers_all_three() {
        let f = general_expr("for total in __RSMPL_E0 { ... }", "for sum in E { ... }");
        let c = CompiledPattern {
            kind: f,
            table: Default::default(),
            minus_bindings: Default::default(),
            plus_bindings: Default::default(),
            unbound_plus: Vec::new(),
            repeated: Vec::new(),
            root: None,
        };
        assert!(is_wi2d_expr_binder_rename(&c));
    }

    #[test]
    fn for_binder_single_hole_with_ellipsis_fires() {
        // Parity: with the real `__RSMPL_E` naming the `for ... { ... }`
        // shape must still compare equal (markers stay intact).
        let kind = general_expr_single(
            "for total in __RSMPL_E { __RSMPL_ELLIPSIS_0 }",
            "for sum in E { ... }",
        );
        assert_eq!(
            for_loop_binder_rename(&kind),
            Some(("total".to_string(), "sum".to_string()))
        );
    }

    fn general_expr_single(minus_sub: &str, plus_body: &str) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Expr,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders: vec![("E".to_string(), "__RSMPL_E".to_string())],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }
}
