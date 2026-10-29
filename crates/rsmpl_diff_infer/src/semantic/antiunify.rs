//! Anti-unification core for WI-4 (first slice: `let` statements).
//!
//! Given N ≥ 2 `(minus, plus)` native line pairs of the same `syn`
//! variant, compute the least general generalization — a template pair
//! where every position that is not token-equal across all sites stays
//! a fresh inferred metavariable (`P0` pat, `E0` expr, `T0` type, by
//! node kind) and every token-equal position stays literal. Guards
//! (IMPROVE.md WI-4): same `syn` variant at every generalized node;
//! generalization depth ≤ 1 below the root here (pattern, type,
//! initializer are direct children — deeper differences stay vetoed,
//! never a template that is mostly holes); ≥ 1 literal anchor token
//! must survive per side (`let`/`=`/`;` by construction); repeated
//! positions that differ at one site must differ identically at all
//! sites or stay independent (SPEC 15.7 — enforced by requiring
//! per-site token equality wherever a hole is reused).
//!
//! This slice generalizes one shape: **let type-addition** — minus
//! `let PAT = INIT;` (no ascription) → plus `let PAT: T = INIT;`
//! (uniform concrete `T`, per-site preserved `PAT`/`INIT`). A varying
//! type would need an unbound plus-side `T0` (T442-illegal), so it
//! vetoes; a per-site init/pattern delta is not this shape (verbatim
//! and the call sweeps own those). The `syn::Type`-root sweep (WI-4b)
//! and signature evolution (WI-4c) reuse this module.

use quote::ToTokens;

/// Uniform concrete type text for a let type-addition class, or `None`
/// when the pairs do not form one.
pub(crate) fn antiunify_let_type_add(minuses: &[&str], pluses: &[&str]) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    // All-identical classes stay for the tighter verbatim sweep (T422
    // precedent): a concrete literal hunk is always shorter than a
    // metavariable template over the same lines.
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut uniform_ty: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let minus = parse_untyped_let(m)?;
        let (plus_pat_inner, plus_ty, plus_init) = parse_typed_let(p)?;
        // Repeated-hole structural equality (SPEC 15.7): `P0` is reused
        // on the plus side, so the inner pattern must be token-equal
        // per site; same for `E0` and the initializer.
        if tok(&minus.pat) != tok(&plus_pat_inner) {
            return None;
        }
        if tok(&minus.init) != tok(&plus_init) {
            return None;
        }
        // Depth-1 engine gate: the bound init must be matchable (no
        // macros/lets/closures/unsupported nodes — the engine `Stmt`
        // path matches inits via structural `match_expr`, never the
        // call router, so there is no Call/MethodCall exception here).
        if !init_ok(&minus.init) {
            return None;
        }
        let ty = tok(&plus_ty);
        if uniform_ty.as_ref().is_some_and(|u| *u != ty) {
            return None;
        }
        uniform_ty = Some(ty);
    }
    let ty = uniform_ty?;
    // The uniform type must parse as a type on its own (it came from a
    // typed `let`, so this only rejects pathological renders).
    if syn::parse_str::<syn::Type>(&ty).is_err() {
        return None;
    }
    Some(ty)
}

struct UntypedLet {
    pat: syn::Pat,
    init: syn::Expr,
}

/// Parse a minus-side `let PAT = INIT;`: complete single-line `let`
/// statement, no type ascription, with init, no `let-else` diverge
/// (the engine never moves into diverge targets).
fn parse_untyped_let(line: &str) -> Option<UntypedLet> {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return None;
    };
    if matches!(local.pat, syn::Pat::Type(_)) {
        return None;
    }
    let init = local.init?;
    if init.diverge.is_some() {
        return None;
    }
    Some(UntypedLet {
        pat: local.pat,
        init: *init.expr,
    })
}

/// Parse a plus-side `let PAT: T = INIT;` into (inner pattern, type,
/// init). The ascription must be present; init and diverge rules match
/// the minus side.
fn parse_typed_let(line: &str) -> Option<(syn::Pat, syn::Type, syn::Expr)> {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return None;
    };
    let syn::Pat::Type(pt) = local.pat else {
        return None;
    };
    let init = local.init?;
    if init.diverge.is_some() {
        return None;
    }
    Some((*pt.pat, *pt.ty, *init.expr))
}

fn tok<T: ToTokens>(node: &T) -> String {
    node.to_token_stream().to_string()
}

/// Engine gate for a bound let initializer (mirror of
/// `verbatim::verbatim_init_ok`, local copy since that fn is private
/// to `verbatim.rs`): the `Stmt` path matches inits structurally, so a
/// pattern containing macros, lets, closures or unsupported nodes
/// matches nothing and would fail loud at apply.
fn init_ok(init: &syn::Expr) -> bool {
    let mut probe = ShapeProbe::default();
    probe.visit_expr(init);
    if probe.macros > 0 || probe.lets > 0 || probe.closures > 0 || probe.unsupported > 0 {
        return false;
    }
    // The try+closure combination matches nothing (a2fc9b3 precedent).
    if probe.trys > 0 && probe.closures > 0 {
        return false;
    }
    true
}

/// One AST walk collecting engine-relevant shape flags (same bounded
/// variant set as `verbatim.rs`: anything outside `is_supported`
/// falls to `_ => false` in engine `match_expr` and never matches).
#[derive(Default)]
struct ShapeProbe {
    macros: usize,
    lets: usize,
    closures: usize,
    trys: usize,
    unsupported: usize,
}

impl<'ast> syn::visit::Visit<'ast> for ShapeProbe {
    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        if !is_supported(node) {
            self.unsupported += 1;
        }
        syn::visit::visit_expr(self, node);
    }
    fn visit_expr_macro(&mut self, _node: &'ast syn::ExprMacro) {
        self.macros += 1;
    }
    fn visit_expr_let(&mut self, node: &'ast syn::ExprLet) {
        self.lets += 1;
        syn::visit::visit_expr_let(self, node);
    }
    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.closures += 1;
        syn::visit::visit_expr_closure(self, node);
    }
    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        self.trys += 1;
        syn::visit::visit_expr_try(self, node);
    }
}

use syn::visit::Visit as _;

/// The engine general matcher's bounded variant set (`match_expr` in
/// `matcher_general.rs` handles exactly these — kept in sync with
/// `verbatim.rs`; divergence is a bug).
fn is_supported(e: &syn::Expr) -> bool {
    matches!(
        e,
        syn::Expr::Path(_)
            | syn::Expr::Lit(_)
            | syn::Expr::Binary(_)
            | syn::Expr::Unary(_)
            | syn::Expr::Cast(_)
            | syn::Expr::Reference(_)
            | syn::Expr::Paren(_)
            | syn::Expr::Group(_)
            | syn::Expr::Call(_)
            | syn::Expr::MethodCall(_)
            | syn::Expr::Assign(_)
            | syn::Expr::Field(_)
            | syn::Expr::Index(_)
            | syn::Expr::Array(_)
            | syn::Expr::Tuple(_)
            | syn::Expr::Struct(_)
            | syn::Expr::Range(_)
            | syn::Expr::Await(_)
            | syn::Expr::Try(_)
            | syn::Expr::Return(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_add_hetero_patterns_and_inits_generalizes() {
        let minuses = [
            "    let alpha_value = compute_first(input);",
            "    let beta_value = compute_second(other);",
        ];
        let pluses = [
            "    let alpha_value: u32 = compute_first(input);",
            "    let beta_value: u32 = compute_second(other);",
        ];
        assert_eq!(
            antiunify_let_type_add(&minuses, &pluses).as_deref(),
            Some("u32")
        );
    }

    #[test]
    fn type_add_varying_type_vetoes() {
        // A varying type would need an unbound plus-side T0 (T442).
        let minuses = ["    let a = f(x);", "    let b = f(y);"];
        let pluses = ["    let a: u32 = f(x);", "    let b: u64 = f(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_per_site_init_delta_vetoes() {
        let minuses = ["    let a = f(x);", "    let b = f(y);"];
        let pluses = ["    let a: u32 = f(x);", "    let b: u32 = g(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_per_site_pattern_delta_vetoes() {
        let minuses = ["    let a = f(x);", "    let b = f(y);"];
        let pluses = ["    let a2: u32 = f(x);", "    let b: u32 = f(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_all_identical_declines_for_verbatim() {
        let minuses = ["    let a = f(x);", "    let a = f(x);"];
        let pluses = ["    let a: u32 = f(x);", "    let a: u32 = f(x);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_minus_with_type_vetoes() {
        let minuses = ["    let a: u8 = f(x);", "    let b: u8 = f(y);"];
        let pluses = ["    let a: u32 = f(x);", "    let b: u32 = f(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_plus_without_type_vetoes() {
        let minuses = ["    let a = f(x);", "    let b = f(y);"];
        let pluses = ["    let a = f(x);", "    let b = f(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_macro_init_vetoes() {
        let minuses = ["    let a = m!(x);", "    let b = m!(y);"];
        let pluses = ["    let a: u32 = m!(x);", "    let b: u32 = m!(y);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }

    #[test]
    fn type_add_single_vetoes() {
        let minuses = ["    let a = f(x);"];
        let pluses = ["    let a: u32 = f(x);"];
        assert_eq!(antiunify_let_type_add(&minuses, &pluses), None);
    }
}
