//! Precision-definition singles (WI-19b).
//!
//! Split from `semantic/mod.rs` under the 2000-line gate (D5): pure
//! move, no behavior change. `mod.rs` keeps the facade and re-exports
//! this module's name so `super::` paths keep working.

use super::{
    guard_for, item_rename_vis, prov::Counted, try_assoc_def_only, try_fn_slot_sweep,
    try_macro_rules_def_only, try_member_def_only, try_sig_evolution_sweep, Guard,
};

/// WI-19b precision singles beyond item-rename/definition-evolution:
/// slot, sig-evolution, bare-item vis, member def-only, macro-rules
/// def-only, assoc def-only — in that order (disjoint by construction:
/// item-rename owns plain `fn` renames, the slot sweep owns rename-only
/// headers it declines, sig owns evolving headers — including every N=1
/// name-preserving evolution, so the hetero-names header sweep stays
/// N≥2 — and def-only owns non-header definition lines). Each
/// detector keeps its own totality; callers apply the precision guard.
pub(crate) fn try_precision_definition_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    if let Counted::Fire(sweep) =
        try_fn_slot_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        debug_assert_eq!(guard_for("fn_slot_sweep"), Guard::Precision);
        return Some((sweep, "fn_slot_sweep"));
    }
    if let Counted::Fire(evo) =
        try_sig_evolution_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        debug_assert_eq!(guard_for("sig_evolution"), Guard::Precision);
        return Some((evo, "sig_evolution"));
    }
    // N=1 bare-item vis single (struct/enum vis singles): same-name
    // closed-set vis step as a terse ItemRename pair. Same machinery as
    // renames (provenance `item_rename`, precision guard) — the rename
    // edit replays the ident while the vis edit carries the change.
    if let Some(vis) =
        item_rename_vis::try_item_vis_single(minuses, pluses, path, file_old, changed, used_rules)
    {
        debug_assert_eq!(guard_for("item_rename"), Guard::Precision);
        return Some((vis, "item_rename"));
    }
    if let Some(out) = try_member_def_only(minuses, pluses, path, file_old, changed, used_rules) {
        return Some(out);
    }
    if let Some(out) =
        try_macro_rules_def_only(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some(out);
    }
    if let Some(out) = try_assoc_def_only(minuses, pluses, path, file_old, changed, used_rules) {
        return Some(out);
    }
    None
}
