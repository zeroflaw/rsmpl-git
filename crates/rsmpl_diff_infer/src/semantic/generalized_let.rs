//! Let-template sweeps: typed, untyped, and tuple-position-swap N-site joints.
//! split off verbatim under the WI-19 1900-line warning gate (D5): pure move,
//! no behavior change. Tests stay inline in the origin file per WI-22.
use super::antiunify::{antiunify_let_template, antiunify_pat_swap, antiunify_untyped_let};
use super::prov::Counted;
use super::unique_rule;

/// N-site (N ≥ 2) joint let-template sweep (WI-5 slice 1): typed→typed
/// `let` pairs with per-site preserved pattern and type plus one
/// uniform initializer delta (e.g. `let P0: T0 = E0;` →
/// `let P0: T0 = E0.wrapping_add(1);` with hetero `P0`/`T0`/`E0`
/// across sites). Strictly generalizes the type-add-only slice (which
/// owns minus-untyped classes) and the type slice (which owns
/// per-site-preserved-init classes) — disjoint from both by
/// construction: this needs ascriptions on both sides plus a per-site
/// init delta. Totality is the real-pair engine-count check
/// (`template_pair_holds`): the emitted minus must match exactly N
/// typed lets in the pre-image — an untouched twin vetoes, exactly
/// like the real sweep rewrites. Detector name `let_template`.
pub(crate) fn try_let_template_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return Counted::Miss;
    }
    let Some((minus_tpl, plus_tpl)) = antiunify_let_template(minuses, pluses) else {
        return Counted::Miss;
    };
    match crate::totality_counted::engine_verdict(
        file_old,
        crate::totality_counted::template_pair_counts(path, file_old, &minus_tpl, &plus_tpl),
        minuses.len(),
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Counted::Fire(hunk)
}

/// N-site (N ≥ 2) untyped joint let-template sweep (WI-8 slice 1):
/// untyped→untyped `let` pairs with per-site preserved patterns plus
/// one uniform initializer delta (`let Some(P0) = E0;` →
/// `let Some(P0) = E0.unwrap_or(0);`, hetero `P0`/`E0` across
/// sites). The typed core's sibling without ascriptions (disjoint by
/// construction: this declines any ascribed side; the typed detector
/// above owns those classes). Uniform concrete deltas
/// (`let v = const { 1 + 2 };` → `… { 1 + 3 };`,
/// `let v = _;` → `let v = 0;`) ride the same core — WI-8's
/// const/infer init support is the `shape.rs` parity gate, not a
/// second template. Totality is the real-pair engine-count check
/// (`template_pair_holds`): the emitted minus must match exactly N
/// untyped lets in the pre-image — an untouched twin vetoes.
/// Detector name `let_template` (same family as the typed core).
pub(crate) fn try_untyped_let_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return Counted::Miss;
    }
    let Some((minus_tpl, plus_tpl)) = antiunify_untyped_let(minuses, pluses) else {
        return Counted::Miss;
    };
    match crate::totality_counted::engine_verdict(
        file_old,
        crate::totality_counted::template_pair_counts(path, file_old, &minus_tpl, &plus_tpl),
        minuses.len(),
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Counted::Fire(hunk)
}

/// N-site (N ≥ 2) tuple-position swap sweep (WI-8 slice 1):
/// `let (P0, P1) = E0;` → `let (P1, P0) = E0;` with per-site
/// preserved inits (a per-site init delta belongs to the untyped
/// core above, never guessed here). Totality is the same real-pair
/// engine-count check. Detector name `let_template` (same family).
pub(crate) fn try_pat_swap_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return Counted::Miss;
    }
    let Some((minus_tpl, plus_tpl)) = antiunify_pat_swap(minuses, pluses) else {
        return Counted::Miss;
    };
    match crate::totality_counted::engine_verdict(
        file_old,
        crate::totality_counted::template_pair_counts(path, file_old, &minus_tpl, &plus_tpl),
        minuses.len(),
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Counted::Fire(hunk)
}
