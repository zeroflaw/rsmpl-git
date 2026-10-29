//! Shared engine-shape gates for inference detectors (WI-4b debt).
//!
//! Single source of truth for the [`supported_expr`] variant list and
//! the [`init_ok`] strict gate, used by both `verbatim.rs`
//! (`verbatim_shape_ok`, `verbatim_init_ok`) and `antiunify.rs`
//! (`init_ok`). The two copies were byte-identical in logic but
//! textually duplicated (WI-4a review gap 2): a gate that admits what
//! the engine rejects fails loud at apply, while a gate stricter than
//! the engine only loses compression — silent divergence either way is
//! a soundness-adjacent bug, so both detectors funnel through this one
//! module (agreement by construction, pinned by the matrix below).
//!
//! The variant list mirrors the engine general matcher's bounded set
//! (`match_expr` in `rsmpl_engine/src/matcher_general.rs` handles
//! exactly these — everything else falls to `_ => false` and never
//! matches, T412). When the engine grows an arm, this list grows with
//! it or the new shape stays ordinary by detector veto (never by
//! over-match: the totality counters re-check every emission).

use syn::visit::Visit as _;

/// Engine-relevant shape flags for one expression subtree.
#[derive(Default)]
pub(crate) struct ShapeProbe {
    pub(crate) macros: usize,
    pub(crate) lets: usize,
    pub(crate) closures: usize,
    pub(crate) trys: usize,
    /// Nodes outside [`supported_expr`]: anything here falls to
    /// `_ => false` in engine `match_expr` and never matches (T412).
    pub(crate) unsupported: usize,
}

impl<'ast> syn::visit::Visit<'ast> for ShapeProbe {
    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        if !supported_expr(node) {
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

/// Collect [`ShapeProbe`] flags for one expression subtree.
pub(crate) fn probe_expr(e: &syn::Expr) -> ShapeProbe {
    let mut probe = ShapeProbe::default();
    probe.visit_expr(e);
    probe
}

/// The engine general matcher's bounded `syn::Expr` variant set.
pub(crate) fn supported_expr(e: &syn::Expr) -> bool {
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
            // WI-5a: `if` (condition + then-block + else branch) and
            // blocks (concrete-length statement sequences) match
            // structurally in the engine.
            | syn::Expr::If(_)
            | syn::Expr::Block(_)
    )
}

/// Strict single-expression gate shared by the `let`-init paths: the
/// engine `Stmt` path matches inits structurally via `match_expr`, so a
/// pattern containing macros, lets, closures or unsupported nodes
/// matches nothing and the sweep would fail loud. The try+closure
/// combination matches nothing either (a2fc9b3 precedent).
pub(crate) fn init_ok(e: &syn::Expr) -> bool {
    let probe = probe_expr(e);
    if probe.macros > 0 || probe.lets > 0 || probe.closures > 0 || probe.unsupported > 0 {
        return false;
    }
    // The try+closure combination matches nothing (a2fc9b3 precedent).
    if probe.trys > 0 && probe.closures > 0 {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(s: &str) -> syn::Expr {
        syn::parse_str::<syn::Expr>(s).expect("test expr must parse")
    }

    #[test]
    fn supported_matrix_pins_engine_variant_list() {
        // Accepted: every arm the engine `match_expr` handles (T412).
        for ok in [
            "x",
            "1",
            "a + b",
            "!a",
            "x as u32",
            "&x",
            "(x)",
            "f(x)",
            "r.method(x)",
            "a = b",
            "s.field",
            "v[0]",
            "[a, b]",
            "(a, b)",
            "S { x }",
            "0..n",
            "fut.await",
            "x?",
            "return x",
            // WI-5a: `if` and blocks match structurally now.
            "if a { b }",
            "if a { b } else { c }",
            "{ x }",
        ] {
            assert!(supported_expr(&expr(ok)), "{ok} must be supported");
        }
        // Rejected: match/loops/closures/let/macro shapes fall to
        // `_ => false` in the engine and never match.
        for bad in [
            "m!(x)",
            "let Some(x) = y",
            "|| x",
            "match a { _ => b }",
            "loop {}",
            "async { x }",
        ] {
            assert!(!supported_expr(&expr(bad)), "{bad} must be unsupported");
        }
    }

    #[test]
    fn init_ok_pins_strict_gate() {
        assert!(init_ok(&expr("compute_first(input)")));
        assert!(init_ok(&expr("x + 1")));
        // Macros, lets, closures, unsupported nodes each veto.
        assert!(!init_ok(&expr("m!(x)")));
        assert!(!init_ok(&expr("match a { _ => b }")));
        assert!(!init_ok(&expr("|| x")));
        // Try alone is fine; try+closure matches nothing (a2fc9b3).
        assert!(init_ok(&expr("x?")));
        assert!(!init_ok(&expr("(|| x)?")));
    }
}
