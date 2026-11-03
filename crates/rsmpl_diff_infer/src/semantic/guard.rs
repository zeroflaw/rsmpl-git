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
        | "item_intent"
        | "trait_method_rename"
        | "inherent_method_rename" => Guard::Precision,
        _ => Guard::Strict,
    }
}

/// WI-7: one precision-allowance law (SPEC D.4). A proved site set
/// (scratch-apply totality over the exact sites) prices
/// `semantic <= ordinary + 16 * claimed_sites + filter_block_bytes`,
/// where `filter_block_bytes` is the byte count of anchor lines the
/// emission is required to carry (enclosing `impl`/`trait` headers).
/// `n_claimed == 0` prices one site (the single-site floor); strict
/// T375 (`semantic < ordinary`) remains for sweeps with no proof.
pub(crate) fn precision_allowance(n_claimed: usize, filter_extra: usize) -> usize {
    super::PRECISION_GUARD
        .saturating_mul(n_claimed.max(1))
        .saturating_add(filter_extra)
}

/// WI-7 slice 2: one decision point for byte guards. Proved site sets
/// price `semantic <= ordinary + precision_allowance(n, extra)`; sweeps
/// with no proof keep strict T375 (`semantic < ordinary`).
pub(crate) fn within_precision(
    body_len: usize,
    ordinary_len: usize,
    n_claimed: usize,
    filter_extra: usize,
) -> bool {
    body_len <= ordinary_len.saturating_add(precision_allowance(n_claimed, filter_extra))
}

/// Strict T375 for sweeps with no proof (`semantic < ordinary`).
pub(crate) fn within_strict(body_len: usize, ordinary_len: usize) -> bool {
    body_len < ordinary_len
}

/// WI-9 slice 1: one decision point for whole-section pair-chain
/// emissions (SPEC D.4). Precision detectors prove their exact site
/// set (block probe + full-set scratch-apply), so they price the WI-7
/// allowance over the claimed pair count plus the per-emission
/// filter-header excess measured from the emission itself; Strict
/// detectors keep T375. WI-17 slice 1 records the veto with the
/// verdict numbers via `prov::declined_guard_with_numbers`
/// (`guard:<kind>:sem=X:ord=Y:allow=Z`, SPEC D.2); the bare
/// `precision-guard`/`byte-guard` spellings are retired on this path;
/// WI-17 slice 2 retired them on every remaining veto path too.
pub(crate) fn pair_section_gate(
    detector: &str,
    emission: &str,
    ordinary: usize,
    n_claimed: usize,
) -> bool {
    if guard_for(detector) == Guard::Precision {
        within_precision(
            emission.len(),
            ordinary,
            n_claimed,
            super::block_headers::filter_header_excess(emission),
        )
    } else {
        within_strict(emission.len(), ordinary)
    }
}

/// WI-7 (WI-2b residue): rendering-normalized whole-section length.
///
/// Real `git diff -U0` elides `,1` per hunk header while unit
/// convention and synthetic builders write it explicitly — the same
/// 4-bytes-per-hunk class `normalized_hunk_len` fixes per hunk. Whole
/// sections sum raw `section.len()` today, so the same rendering flip
/// moves multi-hunk verdicts. Guards must price the hunk *spans*, so
/// this measures the canonical explicit-length headers plus the
/// verbatim prelude/body bytes. Unparseable input falls back to raw
/// length — normalization only ever grows to the explicit convention.
pub(crate) fn normalized_section_len(section: &str) -> usize {
    let mut prelude_len = 0usize;
    let mut hunk_lens = 0usize;
    let mut current: Option<String> = None;
    let mut prelude_done = false;
    for line in section.split_inclusive('\n') {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunk_lens += normalized_hunk_len(&h);
            }
            prelude_done = true;
            current = Some(String::new());
        }
        match current.as_mut() {
            Some(h) => h.push_str(line),
            None => prelude_len += line.len(),
        }
    }
    if let Some(h) = current.take() {
        hunk_lens += normalized_hunk_len(&h);
    }
    // No `@@` at all (or trailing text without newline): `prelude_len`
    // already holds every byte; keep the fallback exact.
    let _ = prelude_done;
    if hunk_lens == 0 {
        section.len()
    } else {
        prelude_len + hunk_lens
    }
}

/// WI-2b: rendering-normalized native hunk length for guard budgets.
///
/// Real `git diff -U0` elides `,1` (`@@ -1 +1 @@`) while unit
/// convention and the synthetic builders write it explicitly
/// (`@@ -1,1 +1,1 @@`) — 4 bytes that flipped a per-hunk precision
/// verdict by 1 (vis+ret single: body 89 vs elided 72+16=88 veto vs
/// explicit 76+16=92 fire). Guards must price the hunk *span*, not
/// its rendering, so every guard measures the canonical
/// explicit-length header plus the verbatim body/title bytes.
/// Unparseable headers (or a header-only string) fall back to
/// `hunk.len()` — normalization never vetoes. Canonical is the
/// explicit form, so budgets only grow to the unit-test convention
/// and no firing emission can newly veto.
pub(crate) fn normalized_hunk_len(hunk: &str) -> usize {
    let Some((header, rest)) = hunk.split_once('\n') else {
        return hunk.len();
    };
    if !header.starts_with("@@") {
        return hunk.len();
    }
    let Some((old_start, old_len, new_start, new_len, title)) = parse_hunk_header(header) else {
        return hunk.len();
    };
    let canonical = format!("@@ -{old_start},{old_len} +{new_start},{new_len} @@{title}\n");
    canonical.len() + rest.len()
}

/// `@@ -o[,ol] +n[,nl] @@{title}` shapes the differ emits; elided
/// lengths default to 1 (git's `,1` elision). `None` on anything else
/// (the caller falls back to the raw length).
fn parse_hunk_header(header: &str) -> Option<(usize, usize, usize, usize, &str)> {
    let inner = header.strip_prefix("@@")?.split("@@").next()?;
    let mut parts = inner.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (old_start, old_len) = parse_range(old)?;
    let (new_start, new_len) = parse_range(new)?;
    let title = header.split("@@").nth(2).unwrap_or("");
    Some((old_start, old_len, new_start, new_len, title))
}

/// `start[,len]` with git's elided-`,1` default.
fn parse_range(range: &str) -> Option<(usize, usize)> {
    let mut it = range.split(',');
    let start: usize = it.next()?.parse().ok()?;
    let len: usize = match it.next() {
        Some(l) => l.parse().ok()?,
        None => 1,
    };
    if it.next().is_some() {
        return None;
    }
    Some((start, len))
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
            "trait_method_rename",
            "inherent_method_rename",
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
            "attr_sweep",
        ] {
            assert_eq!(guard_for(d), Guard::Strict, "{d} must be strict");
        }
        // Unknown detectors (future work, `prov.rs` lag) default strict.
        assert_eq!(guard_for("not_a_detector"), Guard::Strict);
    }

    #[test]
    fn normalized_hunk_len_elided_equals_explicit() {
        // WI-2b live case: the 113-shape native hunk prices 72 elided,
        // 76 explicit — the precision verdict must not depend on it.
        let body = "-fn helper(x: i32) -> i32 {\n+pub fn helper(x: i32) -> i64 {\n";
        let elided = format!("@@ -1 +1 @@\n{body}");
        let explicit = format!("@@ -1,1 +1,1 @@\n{body}");
        assert_eq!(normalized_hunk_len(&elided), explicit.len());
        assert_eq!(normalized_hunk_len(&explicit), explicit.len());
    }

    #[test]
    fn normalized_hunk_len_keeps_title_bytes() {
        // Titles (function context) ride verbatim on both sides of the
        // comparison — only the range rendering canonicalizes.
        let body = "-    fn alpha(x: i32) {\n+    pub fn alpha(x: i32) {\n";
        let elided = format!("@@ -2 +2 @@ impl Svc {{\n{body}");
        let explicit = format!("@@ -2,1 +2,1 @@ impl Svc {{\n{body}");
        assert_eq!(normalized_hunk_len(&elided), normalized_hunk_len(&explicit));
        assert_eq!(normalized_hunk_len(&elided), explicit.len());
        assert!(
            normalized_hunk_len(&elided) > normalized_hunk_len(&format!("@@ -2 +2 @@\n{body}"))
        );
    }

    #[test]
    fn normalized_hunk_len_zero_sides_and_fallback() {
        // Pure-insertion `,0` sides parse (already explicit, identity);
        // garbage and header-only input fall back to the raw length.
        let ins = "@@ -1,0 +1,2 @@\n+a\n+b\n";
        assert_eq!(normalized_hunk_len(ins), ins.len());
        let garbage = "not a hunk at all\n-x\n";
        assert_eq!(normalized_hunk_len(garbage), garbage.len());
        let header_only = "@@ -1 +1 @@";
        assert_eq!(normalized_hunk_len(header_only), header_only.len());
    }

    #[test]
    fn precision_allowance_scales_per_site_plus_filter() {
        // WI-7 law: `16 * max(1, n) + filter_extra` from the one
        // `PRECISION_GUARD` constant — never a per-detector literal.
        assert_eq!(precision_allowance(0, 0), super::super::PRECISION_GUARD);
        assert_eq!(precision_allowance(1, 0), super::super::PRECISION_GUARD);
        assert_eq!(precision_allowance(2, 0), super::super::PRECISION_GUARD * 2);
        assert_eq!(precision_allowance(3, 0), super::super::PRECISION_GUARD * 3);
        // Method-filtered singles ride `16 * 2 + excess` (same value
        // the whole-section arm open-coded as `32 + excess`).
        assert_eq!(
            precision_allowance(2, 7),
            super::super::PRECISION_GUARD * 2 + 7
        );
    }

    #[test]
    fn within_precision_boundary_and_strict() {
        // WI-7 slice 2: the one decision point — boundary equality wins
        // under precision, loses under strict; scaling rides the same
        // `precision_allowance` law (no per-detector formula).
        assert!(within_precision(116, 100, 1, 0));
        assert!(!within_precision(117, 100, 1, 0));
        assert!(within_precision(100 + 32 + 7, 100, 2, 7));
        assert!(!within_precision(100 + 32 + 8, 100, 2, 7));
        assert!(!within_strict(100, 100));
        assert!(within_strict(99, 100));
    }

    #[test]
    fn pair_section_gate_admits_proved_method_sizes() {
        // WI-9 slice 1: method intents prove their site set, so a
        // 2-pair emission 20B over ordinary still gates Precision
        // while a Strict detector vetoes the identical bytes.
        assert!(pair_section_gate(
            "inherent_method_rename",
            &"x".repeat(120),
            100,
            2
        ));
        assert!(pair_section_gate(
            "trait_method_rename",
            &"x".repeat(120),
            100,
            2
        ));
        assert!(!pair_section_gate(
            "method_to_method",
            &"x".repeat(120),
            100,
            2
        ));
        // WI-17 slice 2: the retired `pair_gate_reason` helper is gone —
        // the kind rides `guard_for` (the veto text carries it as
        // `guard:<kind>:` with the WI-7 numbers at the call site).
        assert_eq!(guard_for("inherent_method_rename"), Guard::Precision);
        assert_eq!(guard_for("call_sweep"), Guard::Strict);
    }

    #[test]
    fn normalized_section_len_elided_equals_explicit() {
        // WI-2b residue: whole-section guards must price spans, not
        // rendering — elided and explicit forms of the same section
        // measure equal, at the explicit length.
        let prelude = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n";
        let body = "-fn foo() {\n+fn bar() {\n";
        let elided = format!("{prelude}@@ -1 +1 @@\n{body}");
        let explicit = format!("{prelude}@@ -1,1 +1,1 @@\n{body}");
        assert_eq!(normalized_section_len(&elided), explicit.len());
        assert_eq!(normalized_section_len(&explicit), explicit.len());
        // Two hunks: the 4B delta applies per hunk (8 total).
        let two_elided = format!("{prelude}@@ -1 +1 @@\n{body}@@ -5 +5 @@\n{body}");
        let two_explicit = format!("{prelude}@@ -1,1 +1,1 @@\n{body}@@ -5,1 +5,1 @@\n{body}");
        assert_eq!(normalized_section_len(&two_elided), two_explicit.len());
        assert!(normalized_section_len(&two_elided) > two_elided.len());
        // Hunk-free input falls back to the raw length.
        assert_eq!(normalized_section_len(prelude), prelude.len());
    }
}
