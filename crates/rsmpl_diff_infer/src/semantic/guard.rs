//! Byte-guard discipline (WI-19, SPEC D.4): one decision point for
//! which detectors ride precision vs strict T375.
//!
//! Split from `semantic/mod.rs` under the 2000-line gate: pure move
//! of the WI-19 guard vocabulary, no behavior change. `mod.rs` keeps
//! the facade (`infer_semantic*`) and re-exports this module's names
//! so `super::`/`semantic::` paths keep working.

/// Byte-win guard discipline (WI-19, SPEC D.4): definition shapes are
/// proved by construction (the engine re-verifies the plus at the
/// reported spans), so they fire under the bounded precision guard
/// (`semantic <= ordinary + PRECISION_GUARD`); sweep shapes whose
/// minus pattern could match wider than the diff keep strict T375
/// (`semantic < ordinary`), where bytes remain the honest proxy for
/// over-match risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Guard {
    Precision,
    Strict,
}

/// One decision point for the guard discipline above (WI-19): every
/// single/pair caller consults this instead of open-coding a byte
/// comparison, so a detector's guard can only change here. Unknown
/// names (future detectors, `prov.rs` lag) default to [`Guard::Strict`].
pub(crate) fn guard_for(detector: &str) -> Guard {
    match detector {
        "item_rename"
        | "member_rename"
        | "assoc_rename"
        | "macro_rules_rename"
        | "fn_slot_sweep"
        | "sig_evolution"
        | "header_evolution"
        | "definition_evolution"
        | "definition_addition"
        | "definition_addition_follow"
        | "variant_addition_follow"
        | "delete"
        | "move"
        | "mod_rename"
        | "item_intent" => Guard::Precision,
        _ => Guard::Strict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_for_precision_set_matches_spec() {
        // WI-19: definition shapes (proved by construction via the WI-1
        // plus re-match) ride the precision guard; everything else
        // keeps strict T375. This list is the SPEC D.4 vocabulary —
        // adding a detector without classifying it here fails CI.
        for d in [
            "item_rename",
            "member_rename",
            "assoc_rename",
            "macro_rules_rename",
            "fn_slot_sweep",
            "sig_evolution",
            "header_evolution",
            "definition_evolution",
            "definition_addition",
            "definition_addition_follow",
            "variant_addition_follow",
            "delete",
            "move",
            "mod_rename",
            "item_intent",
        ] {
            assert_eq!(guard_for(d), Guard::Precision, "{d} must be precision");
        }
        for d in [
            "call_sweep",
            "call_arg_template",
            "method_arg_template",
            "call_arg_add",
            "call_literal",
            "call_peel",
            "control_template",
            "general_expr",
            "literal_frame",
            "let_template",
            "let_type_add",
            "type_sweep",
            "sequence",
            "repeat_fold",
            "partial_strip",
            "single_block",
            "use_prefix",
            "verbatim",
            "macro_rename",
            "macro_interior",
            "macro_rule_body",
            "macro_enclosing",
            "qualified_call",
            "turbofish",
            "ref_strip",
            "method_same_literal",
            "method_to_fn",
            "method_to_method",
            "trait_method_rename",
            "attr_sweep",
        ] {
            assert_eq!(guard_for(d), Guard::Strict, "{d} must be strict");
        }
        // Unknown detectors (future work, `prov.rs` lag) default strict.
        assert_eq!(guard_for("not_a_detector"), Guard::Strict);
    }
}
