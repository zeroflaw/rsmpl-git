//! C21: general Stmt/Expr hunks must not rewrite bound binder idents
//! the binder machinery cannot track.
//!
//! `binder_rename` (correctly, conservatively) accepts only single-binder
//! diffs (bare-`x` plus Stmt single-leaf destructuring), so a
//! multi-leaf destructuring rename (`let (a, b) = E;` → `let (c, d) =
//! E;`, `if let Some(old) = E { ... }` → `if let Some(new) = E { ... }`)
//! falls through to the general-template path, which rewrites the
//! header, strands body/file uses, files no orphans, and still reports
//! `verified`. This gate closes that verdict.
//!
//! Veto invariant (keyed by binding, D11): let `m`/`p` be the hole-erased
//! concrete bound-ident sets of minus/plus `Pat` positions. The hunk is
//! binding-safe iff `m == p` (a pure permutation — same binders, same
//! scope), or every dropped ident (`m \ p`) is replayed through a
//! plus-side hole covering its position (`P @ Some(x)` → `P`,
//! `Some(x)` → `Some(P)` — the emitted text replays the target, so no
//! use strands). Anything else — a plus-side binder minus never bound
//! (renames, adds, hole concretions like `Some(P)` → `Some(x)`), or a
//! dropped binder with no hole replay (`Some(x)` → `None`) — vetoes with
//! `RSMPL_PATTERN_CONFLICT` C21 when no binder identity owns the hunk
//! (the minus *does* match, so a no-match error would mislead).
//!
//! Plain bare-ident renames and Stmt single-leaf destructuring renames
//! keep today's behavior through binder identity (completion +
//! orphans). Sequences are exempt at the call site except a
//! rename-once plain-`let` rename at any single position, which enters
//! binder identity (residue slices 1-2): the raiser owns other
//! binder-renaming hetero runs there (WI-5d single-block fusion,
//! completed by sibling hunks in the same patch — c48a6e6 pins the
//! shape), so a hunk-local veto would break the apply direction of a
//! proven cycle; sequence-contained destructuring and old-mentioning
//! windows stay declined (exempt window-only), owned by the next
//! slice, not this gate. Unparseable sides never veto (downstream
//! gates own them).

use std::collections::HashSet;

use syn::visit::Visit;

use rsmpl_pattern::GeneralShape;

/// True when the C21 gate fires (Stmt/Expr call-site scope): the template
/// rewrites bound idents the binder machinery cannot track, and no binder
/// identity owns the hunk.
pub fn unclaimed_binder_rewrite(
    shape: GeneralShape,
    minus_sub: &str,
    plus_body: &str,
    placeholders: &[(String, String)],
) -> bool {
    if shape == GeneralShape::Type {
        return false;
    }
    let holes: HashSet<String> = placeholders
        .iter()
        .flat_map(|(declared, marker)| [declared.clone(), marker.clone()])
        .collect();
    let minus = collect(shape, minus_sub, &holes);
    let plus = collect(shape, plus_body, &holes);
    let (Some(m_side), Some(p_side)) = (minus, plus) else {
        return false;
    };
    let m: HashSet<&str> = m_side.concrete.iter().map(String::as_str).collect();
    let p: HashSet<&str> = p_side.concrete.iter().map(String::as_str).collect();
    if m == p {
        return false;
    }
    // Invented binder (rename/add/concretion): nothing downstream can
    // account for it.
    if !p.is_subset(&m) {
        return true;
    }
    // Strict drop: safe only when every dropped ident is replayed through
    // a plus-side hole covering its position.
    !hole_replay_covers(&m_side.tree, &p_side.tree, &holes)
}

/// One template side: concrete bound idents plus the parsed `Pat` forest
/// for hole-replay coverage.
struct Side {
    concrete: Vec<String>,
    tree: Vec<syn::Pat>,
}

/// Hole-erased concrete bound idents and top-level `Pat` nodes of every
/// `Pat` position, or `None` when the template does not parse.
fn collect(shape: GeneralShape, template: &str, holes: &HashSet<String>) -> Option<Side> {
    struct Bound<'a> {
        holes: &'a HashSet<String>,
        concrete: Vec<String>,
        tree: Vec<syn::Pat>,
    }
    impl<'ast> Visit<'ast> for Bound<'_> {
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            let name = node.ident.to_string();
            if !name.starts_with("__RSMPL_") && !self.holes.contains(&name) {
                self.concrete.push(name);
            }
            // Manual subpat recursion only (`x @ sub` binds both): the
            // default walk would visit the same node twice.
            if let Some((_, sub)) = &node.subpat {
                self.visit_pat(sub);
            }
        }
    }
    // Top-level patterns: the `Pat` the position binds (whole-pattern
    // holes replay it verbatim — the coverage base case).
    fn top_pats(node: &syn::Pat, out: &mut Vec<syn::Pat>) {
        out.push(node.clone());
    }
    let mut collector = Bound {
        holes,
        concrete: Vec::new(),
        tree: Vec::new(),
    };
    match shape {
        GeneralShape::Stmt => {
            let stmt: syn::Stmt = syn::parse_str(template).ok()?;
            if let syn::Stmt::Local(local) = &stmt {
                top_pats(&local.pat, &mut collector.tree);
            }
            collector.visit_stmt(&stmt);
        }
        GeneralShape::Expr => {
            let expr = parse_expr_forgiving(template)?;
            for pat in expr_top_pats(&expr) {
                collector.tree.push(pat);
            }
            collector.visit_expr(&expr);
        }
        GeneralShape::Sequence => {
            // Newline-joined single-statement templates (SPEC 16.4.2).
            for line in template.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let stmt: syn::Stmt = syn::parse_str(line).ok()?;
                if let syn::Stmt::Local(local) = &stmt {
                    top_pats(&local.pat, &mut collector.tree);
                }
                collector.visit_stmt(&stmt);
            }
        }
        GeneralShape::Type => return None,
    }
    Some(Side {
        concrete: collector.concrete,
        tree: collector.tree,
    })
}

/// Top-level bound patterns of a general expression: the positions whose
/// whole text a hole replays. Nested lets inside blocks are separate
/// statements (their own coverage domain when the block replays them —
/// conservatively uncovered here, safe direction).
fn expr_top_pats(expr: &syn::Expr) -> Vec<syn::Pat> {
    match expr {
        syn::Expr::If(e) => match &*e.cond {
            syn::Expr::Let(l) => vec![(*l.pat).clone()],
            _ => vec![],
        },
        syn::Expr::ForLoop(e) => vec![(*e.pat).clone()],
        syn::Expr::While(e) => match &*e.cond {
            syn::Expr::Let(l) => vec![(*l.pat).clone()],
            _ => vec![],
        },
        syn::Expr::Match(e) => e.arms.iter().map(|a| a.pat.clone()).collect(),
        syn::Expr::Closure(e) => e.inputs.iter().cloned().collect(),
        syn::Expr::Let(e) => vec![(*e.pat).clone()],
        _ => vec![],
    }
}

/// True when every minus-side concrete binder is preserved in, or replayed
/// through a plus-side hole covering its position. A plus-side hole
/// replays its whole matched fragment, so the subtree beneath it is
/// covered whatever minus spells there.
fn hole_replay_covers(minus: &[syn::Pat], plus: &[syn::Pat], holes: &HashSet<String>) -> bool {
    minus.len() == plus.len()
        && minus
            .iter()
            .zip(plus.iter())
            .all(|(m, p)| pat_covers(m, p, holes))
}

/// Position-wise coverage: `true` iff no minus-side binder under `m`
/// disappears from the emitted text.
fn pat_covers(m: &syn::Pat, p: &syn::Pat, holes: &HashSet<String>) -> bool {
    if is_hole(p, holes) {
        return true;
    }
    match (m, p) {
        (syn::Pat::Ident(mm), syn::Pat::Ident(pp)) => {
            // Bare-vs-bare: covered iff same binder (`mut`/`ref` ride
            // along — the name is what strands). Subpatterns (`x @ sub`)
            // never cover structurally: different binders below veto.
            mm.subpat.is_none() && pp.subpat.is_none() && mm.ident == pp.ident
        }
        (syn::Pat::Type(mm), syn::Pat::Type(pp)) => pat_covers(&mm.pat, &pp.pat, holes),
        (syn::Pat::Paren(mm), syn::Pat::Paren(pp)) => pat_covers(&mm.pat, &pp.pat, holes),
        (syn::Pat::Reference(mm), syn::Pat::Reference(pp)) => {
            mm.mutability.is_some() == pp.mutability.is_some()
                && pat_covers(&mm.pat, &pp.pat, holes)
        }
        (syn::Pat::TupleStruct(mm), syn::Pat::TupleStruct(pp)) => {
            pat_path_eq(&mm.path, &pp.path)
                && mm.elems.len() == pp.elems.len()
                && mm
                    .elems
                    .iter()
                    .zip(pp.elems.iter())
                    .all(|(a, b)| pat_covers(a, b, holes))
        }
        (syn::Pat::Tuple(mm), syn::Pat::Tuple(pp)) => {
            mm.elems.len() == pp.elems.len()
                && mm
                    .elems
                    .iter()
                    .zip(pp.elems.iter())
                    .all(|(a, b)| pat_covers(a, b, holes))
        }
        (syn::Pat::Struct(mm), syn::Pat::Struct(pp)) => {
            pat_path_eq(&mm.path, &pp.path)
                && mm.fields.len() == pp.fields.len()
                && mm.fields.iter().zip(pp.fields.iter()).all(|(a, b)| {
                    render_member(&a.member) == render_member(&b.member)
                        && a.colon_token.is_some() == b.colon_token.is_some()
                        && pat_covers(&a.pat, &b.pat, holes)
                })
        }
        (syn::Pat::Slice(mm), syn::Pat::Slice(pp)) => {
            mm.elems.len() == pp.elems.len()
                && mm
                    .elems
                    .iter()
                    .zip(pp.elems.iter())
                    .all(|(a, b)| pat_covers(a, b, holes))
        }
        // Binder-free positions: nothing to strand, token-equal only.
        (syn::Pat::Wild(_), syn::Pat::Wild(_)) => true,
        (syn::Pat::Path(mm), syn::Pat::Path(pp)) => render(mm) == render(pp),
        (syn::Pat::Lit(mm), syn::Pat::Lit(pp)) => render(mm) == render(pp),
        (syn::Pat::Range(mm), syn::Pat::Range(pp)) => render(mm) == render(pp),
        (syn::Pat::Rest(_), syn::Pat::Rest(_)) => true,
        (syn::Pat::Const(mm), syn::Pat::Const(pp)) => render(mm) == render(pp),
        (syn::Pat::Macro(mm), syn::Pat::Macro(pp)) => render(mm) == render(pp),
        (syn::Pat::Verbatim(mm), syn::Pat::Verbatim(pp)) => {
            quote::ToTokens::to_token_stream(mm).to_string()
                == quote::ToTokens::to_token_stream(pp).to_string()
        }
        _ => false,
    }
}

/// A whole-pattern hole: replays its matched fragment verbatim. Declared
/// hole names erase; `__RSMPL_*` markers never bind.
fn is_hole(p: &syn::Pat, holes: &HashSet<String>) -> bool {
    match p {
        syn::Pat::Ident(pid) if pid.subpat.is_none() => {
            let name = pid.ident.to_string();
            name.starts_with("__RSMPL_") || holes.contains(&name)
        }
        syn::Pat::Wild(_) => true,
        _ => false,
    }
}

fn pat_path_eq(a: &syn::Path, b: &syn::Path) -> bool {
    render(a) == render(b)
}

fn render_member(m: &syn::Member) -> String {
    quote::quote!(#m).to_string()
}

fn render<T: quote::ToTokens>(node: &T) -> String {
    quote::quote!(#node).to_string()
}

/// Parse a general-expression template forgiving the block-tail `...`
/// ellipsis (not valid `syn` on its own), mirroring
/// `rsmpl_pattern::binder` so both sides compare what the matcher sees.
fn parse_expr_forgiving(template: &str) -> Option<syn::Expr> {
    if let Ok(expr) = syn::parse_str::<syn::Expr>(template) {
        return Some(expr);
    }
    let fixed = template.replace("...", "__RSMPL_ELLIPSIS_0");
    syn::parse_str::<syn::Expr>(&fixed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holes() -> Vec<(String, String)> {
        vec![
            ("E".to_string(), "__RSMPL_E0".to_string()),
            ("F".to_string(), "__RSMPL_F0".to_string()),
            ("P".to_string(), "__RSMPL_P0".to_string()),
            ("Q".to_string(), "__RSMPL_Q0".to_string()),
        ]
    }

    fn stmt_vetoes(minus: &str, plus: &str) -> bool {
        unclaimed_binder_rewrite(GeneralShape::Stmt, minus, plus, &holes())
    }

    fn expr_vetoes(minus: &str, plus: &str) -> bool {
        unclaimed_binder_rewrite(GeneralShape::Expr, minus, plus, &holes())
    }

    #[test]
    fn stmt_tuple_destructuring_vetoes() {
        assert!(stmt_vetoes(
            "let (a , old) = __RSMPL_E0 ;",
            "let (a , new) = E ;"
        ));
    }

    #[test]
    fn stmt_struct_pattern_vetoes() {
        assert!(stmt_vetoes(
            "let S { f : old , g } = __RSMPL_E0 ;",
            "let S { f : new , g } = E ;"
        ));
    }

    #[test]
    fn stmt_shorthand_field_vetoes() {
        assert!(stmt_vetoes(
            "let S { old } = __RSMPL_E0 ;",
            "let S { new } = E ;"
        ));
    }

    #[test]
    fn stmt_slice_ref_and_at_veto() {
        assert!(stmt_vetoes(
            "let [a , old] = __RSMPL_E0 ;",
            "let [a , new] = E ;"
        ));
        assert!(stmt_vetoes("let &old = __RSMPL_E0 ;", "let &new = E ;"));
        assert!(stmt_vetoes(
            "let x @ Some (old) = __RSMPL_E0 ;",
            "let x @ Some (new) = E ;"
        ));
    }

    #[test]
    fn stmt_binder_drop_and_add_veto() {
        assert!(stmt_vetoes("let Some (x) = __RSMPL_E0 ;", "let None = E ;"));
        assert!(stmt_vetoes("let None = __RSMPL_E0 ;", "let Some (x) = E ;"));
    }

    #[test]
    fn stmt_hole_concretion_vetoes() {
        // `Some(P)` → `Some(x)` concretizes the replayed fragment: the
        // emitted binder is one minus never bound.
        assert!(stmt_vetoes(
            "let Some (__RSMPL_P0) = __RSMPL_E0 ;",
            "let Some (x) = E ;"
        ));
    }

    #[test]
    fn plain_rename_without_identity_vetoes() {
        // Bare-ident renames are flagged here too; the call site exempts
        // proven single-binder renames via `is_local_binder_rename`
        // (completion + orphans), so these never reach the gate there.
        assert!(stmt_vetoes("let old = __RSMPL_E0 ;", "let new = E ;"));
        assert!(expr_vetoes(
            "if let old = __RSMPL_E0 { ... }",
            "if let new = E { ... }"
        ));
    }

    #[test]
    fn at_pattern_replay_applies() {
        // `P` replays the whole matched pattern text: the emitted
        // statement is byte-identical, no binder is rewritten.
        assert!(!stmt_vetoes(
            "let __RSMPL_P0 @ Some (x) = __RSMPL_E0 ;",
            "let P = E ;"
        ));
        assert!(!stmt_vetoes(
            "let __RSMPL_P0 @ __RSMPL_Q0 = __RSMPL_E0 ;",
            "let P = E ;"
        ));
    }

    #[test]
    fn hole_abstraction_applies() {
        // `Some(x)` → `Some(P)`: the hole replays the target fragment.
        assert!(!stmt_vetoes(
            "let Some (x) = __RSMPL_E0 ;",
            "let Some (P) = E ;"
        ));
        assert!(!stmt_vetoes("let x = __RSMPL_E0 ;", "let P = E ;"));
    }

    #[test]
    fn expr_if_let_tuple_struct_vetoes() {
        assert!(expr_vetoes(
            "if let Some (old) = __RSMPL_E0 { ... }",
            "if let Some (new) = E { ... }"
        ));
    }

    #[test]
    fn expr_for_tuple_vetoes() {
        assert!(expr_vetoes(
            "for (a , old) in __RSMPL_E0 { ... }",
            "for (a , new) in E { ... }"
        ));
    }

    #[test]
    fn expr_while_let_struct_vetoes() {
        assert!(expr_vetoes(
            "while let S { f : old } = __RSMPL_E0 { ... }",
            "while let S { f : new } = E { ... }"
        ));
    }

    #[test]
    fn expr_match_arm_destructuring_vetoes() {
        assert!(expr_vetoes(
            "match __RSMPL_E0 { Some (old) => __RSMPL_F0 , _ => 0 }",
            "match E { Some (new) => F , _ => 0 }"
        ));
    }

    #[test]
    fn expr_closure_param_destructuring_vetoes() {
        assert!(expr_vetoes("| (a , old) | __RSMPL_E0", "| (a , new) | E"));
    }

    #[test]
    fn expr_or_branch_vetoes() {
        assert!(expr_vetoes(
            "match __RSMPL_E0 { Some (old) | None => 1 }",
            "match E { Some (new) | None => 1 }"
        ));
    }

    #[test]
    fn sequence_flags_for_follow_up() {
        // Sequences report through the same pure function but the C21
        // call site exempts them: hetero fusion owns binder-renaming runs
        // there, and a final-position plain-`let` rename completes via
        // binder identity instead of vetoing. Broader sequencing
        // completion is follow-up work.
        assert!(unclaimed_binder_rewrite(
            GeneralShape::Sequence,
            "let a = __RSMPL_E0 ;\nlet (x , old) = __RSMPL_F0 ;",
            "let a = E ;\nlet (x , new) = F ;",
            &holes()
        ));
        // The final-position plain rename flags here too (the pure
        // predicate is binder-identity-blind); it applies because the
        // call site exempts sequences and completes via `sequence_binder`.
        assert!(unclaimed_binder_rewrite(
            GeneralShape::Sequence,
            "let a = __RSMPL_E0 ;\nlet old = __RSMPL_F0 ;",
            "let a = E ;\nlet new = F ;",
            &holes()
        ));
    }

    #[test]
    fn transparent_wrappers_cover_plainly() {
        // Ascription and parens are transparent to coverage; a rename
        // through them still invents a binder and vetoes.
        assert!(stmt_vetoes(
            "let ((x)) : u32 = __RSMPL_E0 ;",
            "let ((y)) : u32 = E ;"
        ));
        assert!(!stmt_vetoes(
            "let ((x)) : u32 = __RSMPL_E0 ;",
            "let ((x)) : u64 = E ;"
        ));
    }

    #[test]
    fn init_only_change_applies() {
        assert!(!stmt_vetoes(
            "let (a , b) = __RSMPL_E0 ;",
            "let (a , b) = E + 1 ;"
        ));
        assert!(!expr_vetoes(
            "if let Some (x) = __RSMPL_E0 { ... }",
            "if let Some (x) = E + 1 { ... }"
        ));
    }

    #[test]
    fn annotation_and_mut_only_changes_apply() {
        assert!(!stmt_vetoes(
            "let x : u32 = __RSMPL_E0 ;",
            "let x : u64 = E ;"
        ));
        assert!(!stmt_vetoes("let x = __RSMPL_E0 ;", "let mut x = E ;"));
    }

    #[test]
    fn constructor_only_change_applies() {
        assert!(!stmt_vetoes(
            "let Some (x) = __RSMPL_E0 ;",
            "let Ok (x) = E ;"
        ));
    }

    #[test]
    fn hole_only_diffs_apply() {
        assert!(!stmt_vetoes(
            "let S { f : __RSMPL_P0 , g : __RSMPL_Q0 } = __RSMPL_E0 ;",
            "let S { f : P , g : Q } = E ;"
        ));
        assert!(!stmt_vetoes("let __RSMPL_P0 = __RSMPL_E0 ;", "let Q = E ;"));
        assert!(!expr_vetoes(
            "match __RSMPL_E0 { Some (__RSMPL_P0) => 1 , _ => 2 }",
            "match E { Some (P) => 9 , _ => 2 }"
        ));
    }

    #[test]
    fn swap_and_reorder_apply() {
        // Equal concrete sets: a pure permutation binds the same idents.
        assert!(!stmt_vetoes(
            "let (a , b) = __RSMPL_E0 ;",
            "let (b , a) = E ;"
        ));
        assert!(!expr_vetoes(
            "match __RSMPL_E0 { Some (__RSMPL_P0) => 1 , None => 2 }",
            "match E { None => 2 , Some (P) => 1 }"
        ));
    }

    #[test]
    fn unparseable_sides_never_veto() {
        assert!(!stmt_vetoes("let (a , old) = ", "let (a , new) = E ;"));
        assert!(!expr_vetoes("if let (", "if let Some (new) = E { ... }"));
        assert!(!unclaimed_binder_rewrite(
            GeneralShape::Sequence,
            "let a = ;",
            "let a = E ;",
            &holes()
        ));
    }

    #[test]
    fn type_shape_never_vetoes() {
        assert!(!unclaimed_binder_rewrite(
            GeneralShape::Type,
            "Vec < __RSMPL_T0 >",
            "Box < [T] >",
            &holes()
        ));
    }
}
