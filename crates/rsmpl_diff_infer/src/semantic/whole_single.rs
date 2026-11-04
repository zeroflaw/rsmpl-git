//! Whole-section N=1 singles (WI-7 slice 1 / WI-22 split).
//!
//! Split from `chain.rs` under the 2000-line gate (D5): pure move with
//! the seven near-identical attempt/guard/push arms collapsed onto one
//! shared decision helper, no behavior change except the ordinary size
//! now prices spans (`normalized_section_len`, WI-2b residue) instead
//! of raw rendering. `chain.rs` keeps the caller (balance gate +
//! per-hunk fallback); this module owns the single-pair arms in their
//! original order: item-rename → definition-evolution → precision
//! single → impl-header → attr-replace → use-prefix → macro-rename.

use super::prov::{
    attempt, attempt_counted, declined_guard_with_numbers, push_section_prov, reheader_rename,
    HunkDeclined, HunkProvenance,
};
use super::{guard_for, Guard};

/// One shared guard/push decision for every N=1 arm below (the
/// module-header promise: one formula in `guard.rs`, never
/// per-detector). `ordinary` is the normalized section length, `allow`
/// the arm's precision allowance. Rename-prelude sections compare the
/// re-headered final bytes; the decline path truncates the reserved
/// rule and records numbered `guard:precision` provenance on every arm.
#[allow(clippy::too_many_arguments)]
fn emit_if_within(
    emission: &str,
    detector: &'static str,
    section: &str,
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    rename_prelude: &Option<String>,
    ordinary: usize,
    allow: usize,
    checkpoint: usize,
    used_rules: &mut Vec<String>,
    prov: &mut Vec<HunkProvenance>,
    declined: &mut Vec<HunkDeclined>,
) -> Option<String> {
    if let Some(prelude) = rename_prelude {
        if let Some(final_text) = reheader_rename(emission, prelude) {
            if final_text.len() <= ordinary + allow {
                push_section_prov(
                    prov,
                    &final_text,
                    section,
                    path,
                    detector,
                    changed,
                    file_old,
                    checkpoint,
                    used_rules,
                );
                return Some(final_text);
            }
            // WI-17 slice 2: veto carries the WI-7 numbers (losing bytes).
            declined_guard_with_numbers(
                declined,
                detector,
                "precision",
                final_text.len(),
                ordinary,
                allow,
            );
        } else {
            used_rules.truncate(checkpoint);
            return None;
        }
        used_rules.truncate(checkpoint);
        None
    } else if emission.len() <= ordinary + allow {
        push_section_prov(
            prov, emission, section, path, detector, changed, file_old, checkpoint, used_rules,
        );
        Some(emission.to_string())
    } else {
        // WI-17 slice 2: veto carries the WI-7 numbers (losing bytes).
        declined_guard_with_numbers(
            declined,
            detector,
            "precision",
            emission.len(),
            ordinary,
            allow,
        );
        used_rules.truncate(checkpoint);
        None
    }
}

/// Whole-section single-pair arms (all seven, original order). Returns
/// the winning semantic section or `None` when every arm declines (the
/// caller falls through to the per-hunk path). `checkpoint` is the
/// `used_rules` length on entry (the item-rename reservation base);
/// each arm re-bases from the live length exactly as before.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_whole_single_section(
    section: &str,
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    minuses: &[&str],
    pluses: &[&str],
    rename_prelude: &Option<String>,
    checkpoint: usize,
    used_rules: &mut Vec<String>,
    prov: &mut Vec<HunkProvenance>,
    declined: &mut Vec<HunkDeclined>,
) -> Option<String> {
    // WI-2b residue fix: price the span (canonical explicit headers),
    // not the raw rendering. Budgets only grow to the explicit
    // convention — no firing emission can newly veto.
    let ordinary = super::guard::normalized_section_len(section);
    // Single pair: fn/item rename under the precision guard
    // (`guard_for("item_rename") == Precision`: the engine
    // re-verifies the plus at its span, so a bounded byte cost
    // prices the proof, not the win). A lone call rewrite stays
    // strict T375 below; a lone heterogeneous fn header likewise
    // (slot sweep covers the multi-site case).
    if let Some(rename) = attempt(
        declined,
        "item_rename",
        super::try_item_rename(minuses[0], pluses[0], path, file_old, changed, used_rules),
    ) {
        debug_assert_eq!(guard_for("item_rename"), Guard::Precision);
        // WI-7a: rename sections re-emit under the preserved
        // prelude; the precision guard compares the final bytes.
        if let Some(out) = emit_if_within(
            &rename,
            "item_rename",
            section,
            path,
            file_old,
            changed,
            rename_prelude,
            ordinary,
            super::guard::precision_allowance(1, 0),
            checkpoint,
            used_rules,
            prov,
            declined,
        ) {
            return Some(out);
        }
    }
    // WI-12 slice 1: single-definition evolution under the precision
    // guard (`semantic <= ordinary + 16`) — beside the item-rename
    // single above. Strict T375 would price every lone RHS edit
    // above its ordinary hunk; the guard prices the proof instead.
    let evo_checkpoint = used_rules.len();
    if let Some((evo, detector)) = attempt_counted(
        declined,
        "definition_evolution",
        super::try_definition_evolution_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        debug_assert_eq!(detector, "definition_evolution");
        if let Some(out) = emit_if_within(
            &evo,
            detector,
            section,
            path,
            file_old,
            changed,
            rename_prelude,
            ordinary,
            super::guard::precision_allowance(1, 0),
            evo_checkpoint,
            used_rules,
            prov,
            declined,
        ) {
            return Some(out);
        }
    }
    // WI-19b: slot/sig/header + def-only member/macro/assoc singles
    // under the precision guard (same `semantic <= ordinary + 16`
    // posture as the two shapes above). N=1 method `sig_evolution`
    // (indented vis/qual-only) rides a doubled allowance (32): the
    // WI-4a verbatim filter block (header + pre-`...` + nesting +
    // closers ≈ +30 over a free single on the stripped section the
    // guard actually sees) is a mandatory proof anchor (D10), not
    // bloat — the 16-byte budget was calibrated on filter-free
    // singles. A long enclosing `impl` header contributes exactly
    // its surplus over the measured baseline on top (`per-emission`
    // pricing, shared with the per-hunk and subhunk budgets), so the
    // allowance scales with the proof, never with a bumped guard.
    // The detector's own gates (vis/qual-only, engine-count
    // totality + D13 byte-compare) are unchanged; the allowance only
    // prices the filter. Mixed-section method singles ride the
    // per-hunk fallback (four-guard method-run price + per-emission
    // header excess); single mixed hunks ride the subhunk partition.
    let prec_checkpoint = used_rules.len();
    if let Some((prec, detector)) = attempt_counted(
        declined,
        "precision_definition_single",
        super::try_precision_definition_single(
            minuses, pluses, path, file_old, changed, used_rules,
        ),
    ) {
        debug_assert_eq!(guard_for(detector), Guard::Precision);
        let allow = if detector == "sig_evolution"
            && minuses.len() == 1
            && minuses[0].starts_with([' ', '\t'])
        {
            super::guard::precision_allowance(2, super::block_headers::filter_header_excess(&prec))
        } else {
            super::guard::precision_allowance(1, 0)
        };
        if let Some(out) = emit_if_within(
            &prec,
            detector,
            section,
            path,
            file_old,
            changed,
            rename_prelude,
            ordinary,
            allow,
            prec_checkpoint,
            used_rules,
            prov,
            declined,
        ) {
            return Some(out);
        }
    }
    // WI-19c: self-ty `impl` header single under the precision guard
    // (same posture as the three shapes above; the scratch-apply
    // probe inside is the totality, orphans at apply are evidence).
    let impl_checkpoint = used_rules.len();
    if let Some((imp, detector)) = attempt_counted(
        declined,
        "impl_header_single",
        super::try_impl_header_single(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        debug_assert_eq!(guard_for(detector), Guard::Precision);
        if let Some(out) = emit_if_within(
            &imp,
            detector,
            section,
            path,
            file_old,
            changed,
            rename_prelude,
            ordinary,
            super::guard::precision_allowance(1, 0),
            impl_checkpoint,
            used_rules,
            prov,
            declined,
        ) {
            return Some(out);
        }
    }
    // WI-14 slice 5: whole-balanced attr-replace single under the
    // precision guard (same posture as the four shapes above; the
    // scratch-apply probe at n=1 plus the T361 byte-compare is the
    // uniqueness proof). No `guard_for` assert: `attr_sweep` stays
    // Strict for the N ≥ 2 sweep — only this n=1 arm prices proof
    // over bytes (the call-single precedent). The N=1 emission is
    // the ranged owner-span diet (no directive: default `one` over
    // the `syn` owner span, which contains both the attr and the
    // header) — the sweep dummy's 8 bytes are what kept the live
    // short-hunk section 1 byte over the guard (fidelity fix); the
    // sweep-based probe still proves it. The full-section accounting
    // (prelude included) is what fits the concrete header inside
    // the guard.
    let attr_checkpoint = used_rules.len();
    if let Some((attr, detector)) = attempt_counted(
        declined,
        "attr_replace_single",
        super::try_attr_replace_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        debug_assert_eq!(detector, "attr_sweep");
        if let Some(out) = emit_if_within(
            &attr,
            detector,
            section,
            path,
            file_old,
            changed,
            rename_prelude,
            ordinary,
            super::guard::precision_allowance(1, 0),
            attr_checkpoint,
            used_rules,
            prov,
            declined,
        ) {
            return Some(out);
        }
    }
    // WI-3 slice 1: single-pair use-prefix single under the
    // precision guard (same posture as the five shapes above; the
    // `use_holds` n=1 totality plus the T361 byte-compare inside
    // is the uniqueness proof). `use_prefix` stays Strict for
    // the N >= 2 sweep — only this n=1 arm prices proof over
    // bytes (the call/type single precedent).
    let use_checkpoint = used_rules.len();
    if let Some(single) = attempt_counted(
        declined,
        "use_prefix",
        super::try_use_prefix_single(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        if let Some(out) = emit_if_within(
            &single,
            "use_prefix",
            section,
            path,
            file_old,
            changed,
            rename_prelude,
            ordinary,
            super::guard::precision_allowance(1, 0),
            use_checkpoint,
            used_rules,
            prov,
            declined,
        ) {
            return Some(out);
        }
    }
    // WI-3 slice 3: single-pair macro-rename single under the
    // precision guard (same posture as the use-prefix single
    // above; `macro_holds`/`qualified_macro_holds` at n=1 plus
    // the T361 byte-compare inside is the uniqueness proof).
    // `macro_rename` stays Strict for the N >= 2 sweep — only
    // this n=1 arm prices proof over bytes.
    let macro_checkpoint = used_rules.len();
    if let Some(single) = attempt_counted(
        declined,
        "macro_rename",
        super::try_macro_rename_single(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        if let Some(out) = emit_if_within(
            &single,
            "macro_rename",
            section,
            path,
            file_old,
            changed,
            rename_prelude,
            ordinary,
            super::guard::precision_allowance(1, 0),
            macro_checkpoint,
            used_rules,
            prov,
            declined,
        ) {
            return Some(out);
        }
    }
    None
}
