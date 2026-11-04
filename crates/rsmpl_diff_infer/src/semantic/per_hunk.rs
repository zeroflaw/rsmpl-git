//! Per-hunk fallback core: one balanced hunk through the shared
//! detector chain.
//!
//! Split from `lib.rs` under the 2000-line gate (WI-2b): pure move
//! with module paths adjusted, no behavior change.
//! `lib.rs` re-exports the entry so `crate::` call sites keep working.

/// One balanced hunk through the shared detector chain (the per-hunk
/// fallback's core): item rename for singles, else slot / method /
/// strip / use-prefix / call sweeps in order. Detectors return full
/// sections; only the hunk body is kept, and only when it beats the
/// ordinary hunk (the T375 per-hunk guard, prelude shared either way).
/// WI-2: the winning detector family travels with the body for per-hunk
/// provenance. `None` on any failure — reserved rule names are released,
/// so later hunks keep tight names. WI-15c: `declined` collects
/// chain-level decline evidence for wholly-ordinary sections (composite
/// drivers attribute at their own level; inner subhunk/verbatim retries
/// pass a throwaway).
#[allow(clippy::too_many_arguments)]
pub(crate) fn compress_balanced_pair_body(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    hunk_len: usize,
    used_rules: &mut Vec<String>,
    declined: &mut Vec<super::HunkDeclined>,
) -> Option<(String, &'static str)> {
    use super::{attempt, attempt_counted, declined_guard_with_numbers};
    if minuses.len() != pluses.len() || minuses.is_empty() {
        return None;
    }
    let checkpoint = used_rules.len();
    // WI-12 slice 1: single-definition evolution under the precision
    // guard (`body <= hunk + 16`). WI-19: single item renames ride the
    // same guard (`guard_for("item_rename") == Precision`). Other
    // singles stay ordinary (T375) except whole-arg permutation (WI-6
    // slice 2 posture).
    if minuses.len() == 1 {
        if let Some((evo, detector)) = attempt(
            declined,
            "definition_evolution",
            super::try_definition_evolution_sweep(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = evo.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), detector));
            }
            declined_guard_with_numbers(
                declined,
                detector,
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-1: whole-line bare-call single under the precision
        // guard (`body <= hunk + 16`; uniqueness proved by `call_verdict`
        // at n=1, `guard_for("call_sweep")` stays Strict for multis).
        // WI-17 slice 4: counted decline — an N=1 shape match with a twin
        // records `totality:found:want` (or `veto:opaque-hit`).
        if let Some(single) = attempt_counted(
            declined,
            "call_sweep",
            super::try_call_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "call_sweep"));
            }
            declined_guard_with_numbers(
                declined,
                "call_sweep",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-2: whole-line method-to-method single under the
        // precision guard (uniqueness proved by `method_holds` at n=1,
        // `guard_for("method_to_method")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_to_method",
            super::try_method_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_to_method"));
            }
            declined_guard_with_numbers(
                declined,
                "method_to_method",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-3: whole-line method-to-function single under the
        // precision guard (uniqueness proved by `method_holds` at n=1,
        // `guard_for("method_to_fn")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_to_fn",
            super::try_method_to_fn_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_to_fn"));
            }
            declined_guard_with_numbers(
                declined,
                "method_to_fn",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slices 4-5: whole-line (slice-4) and `let`-init
        // (slice-5) general-expression singles under the precision
        // guard (uniqueness proved by `template_pair_holds` at n=1,
        // `guard_for("general_expr")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "general_expr",
            super::try_general_expr_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "general_expr"));
            }
            declined_guard_with_numbers(
                declined,
                "general_expr",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-2 slice 1: bare-type literal single (one type subterm
        // delta in any line shape) under the precision guard
        // (uniqueness proved by `template_type_holds` at n=1 plus the
        // T361 byte-compare, `guard_for("type_sweep")` stays Strict for
        // multis). Sits right after the general-expr single: a bare
        // type is tighter than any holed wrap of the same site, and the
        // wrap arms below decline type positions by construction.
        if let Some(single) = attempt(
            declined,
            "type_sweep",
            super::try_type_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "type_sweep"));
            }
            declined_guard_with_numbers(
                declined,
                "type_sweep",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-3 slice 1: single-pair use-prefix single under the
        // precision guard (uniqueness proved by `use_holds` at n=1
        // plus the T361 byte-compare inside; `guard_for("use_prefix")`
        // stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "use_prefix",
            super::try_use_prefix_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "use_prefix"));
            }
            declined_guard_with_numbers(
                declined,
                "use_prefix",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-3 slice 3: single-pair macro-rename single under the
        // precision guard (uniqueness proved by `macro_holds` /
        // `qualified_macro_holds` at n=1 plus the T361 byte-compare
        // inside; `guard_for("macro_rename")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "macro_rename",
            super::try_macro_rename_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "macro_rename"));
            }
            declined_guard_with_numbers(
                declined,
                "macro_rename",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-6: bare-call arg-template wrap single (whole-line
        // + `let`-init via the shared T449 entry) under the precision
        // guard (uniqueness proved by `template_pair_holds` at n=1,
        // `guard_for("call_arg_template")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "call_arg_template",
            super::try_call_arg_template_single(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "call_arg_template"));
            }
            declined_guard_with_numbers(
                declined,
                "call_arg_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-7: method-arg-template wrap single (whole-line
        // + `let`-init via the shared T449 entry) under the precision
        // guard (uniqueness proved by `template_pair_holds` at n=1,
        // `guard_for("method_arg_template")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_arg_template",
            super::try_method_arg_template_single(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_arg_template"));
            }
            declined_guard_with_numbers(
                declined,
                "method_arg_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-10: whole-line bare-call perm single (whole-line
        // + `let`-init via the shared T449 entry) under the precision
        // guard (uniqueness proved by `template_pair_holds` at n=1,
        // `guard_for("call_arg_template")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "call_arg_template",
            super::try_call_perm_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "call_arg_template"));
            }
            declined_guard_with_numbers(
                declined,
                "call_arg_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-11: whole-line method perm single (whole-line +
        // `let`-init via the shared T449 entry) under the precision
        // guard (uniqueness proved by `template_pair_holds` at n=1,
        // `guard_for("method_arg_template")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_arg_template",
            super::try_method_perm_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_arg_template"));
            }
            declined_guard_with_numbers(
                declined,
                "method_arg_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-12: bare-call trailing-literal single (whole-line
        // + `let`-init via the shared T449 entry) under the precision
        // guard (uniqueness proved by `call_literal_holds` at n=1,
        // `guard_for("call_literal")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "call_literal",
            super::try_call_literal_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "call_literal"));
            }
            declined_guard_with_numbers(
                declined,
                "call_literal",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-13: method same-literal single (whole-line
        // + `let`-init via the shared `parse_method_line` entry) under
        // the precision guard (uniqueness proved by
        // `method_literal_holds` at n=1 over the exact old-args tuple,
        // `guard_for("method_same_literal")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_same_literal",
            super::try_method_same_literal_single(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_same_literal"));
            }
            declined_guard_with_numbers(
                declined,
                "method_same_literal",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-14: sub-line bare-call single under the precision
        // guard (whole-line pairs decline here — slice-1 owns them;
        // uniqueness proved by `template_expr_holds` at n=1 plus the
        // T361 byte-compare, `guard_for("call_sweep")` stays Strict for
        // multis).
        if let Some(single) = attempt(
            declined,
            "call_sweep",
            super::try_subline_call_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "call_sweep"));
            }
            declined_guard_with_numbers(
                declined,
                "call_sweep",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-15: sub-line method-to-method single under the
        // precision guard (whole-line pairs decline here — slice-2 owns
        // them; uniqueness proved by `template_expr_holds` at n=1 plus
        // the T361 byte-compare, `guard_for("method_to_method")` stays
        // Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_to_method",
            super::try_subline_method_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_to_method"));
            }
            declined_guard_with_numbers(
                declined,
                "method_to_method",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-16: sub-line method-to-function single under the
        // precision guard (whole-line pairs decline here — slice-3 owns
        // them; uniqueness proved by `template_expr_holds` at n=1 plus
        // the T361 byte-compare, `guard_for("method_to_fn")` stays
        // Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_to_fn",
            super::try_subline_method_fn_single(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_to_fn"));
            }
            declined_guard_with_numbers(
                declined,
                "method_to_fn",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-17: sub-line bare perm single under the precision
        // guard (whole-line pairs decline here - slice-10 owns them;
        // uniqueness proved by `template_pair_holds` at n=1 plus the
        // T361 byte-compare, `guard_for("call_arg_template")` stays
        // Strict for multis).
        if let Some(single) = attempt(
            declined,
            "call_arg_template",
            super::try_subline_perm_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "call_arg_template"));
            }
            declined_guard_with_numbers(
                declined,
                "call_arg_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-18: sub-line method perm single under the precision
        // guard (whole-line pairs decline here - slice-11 owns them;
        // uniqueness proved by `template_pair_holds` at n=1 plus the
        // T361 byte-compare, `guard_for("method_arg_template")` stays
        // Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_arg_template",
            super::try_subline_method_perm_single(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_arg_template"));
            }
            declined_guard_with_numbers(
                declined,
                "method_arg_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-19: sub-line general-expression single under the
        // precision guard (whole-line/let-init general pairs decline
        // here — slices 4-5 own them; uniqueness proved by
        // `template_pair_holds` at n=1 plus the T361 byte-compare,
        // `guard_for("general_expr")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "general_expr",
            super::try_subline_general_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "general_expr"));
            }
            declined_guard_with_numbers(
                declined,
                "general_expr",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-22: sub-line method-arg-template wrap single under
        // the precision guard (whole-line pairs decline here — slice-7
        // owns them; uniqueness proved by `template_pair_holds` at n=1
        // over the emitted holed pair plus the T361 byte-compare,
        // `guard_for("method_arg_template")` stays Strict for multis).
        // Sits BEFORE the literal arms: a holed wrap beats the concrete
        // literal spelling where both prove the same site set (D10); the
        // general arm above declines wraps by construction (Call /
        // MethodCall subterms are never general sites), and the literal
        // arms keep every pair the wrap core declines.
        if let Some(single) = attempt(
            declined,
            "method_arg_template",
            super::try_subline_method_wrap_single(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_arg_template"));
            }
            declined_guard_with_numbers(
                declined,
                "method_arg_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-23: sub-line bare-arg-template wrap single under
        // the precision guard (whole-line pairs decline here — slice-6
        // owns them; uniqueness proved by `template_pair_holds` at n=1
        // over the emitted holed pair plus the T361 byte-compare,
        // `guard_for("call_arg_template")` stays Strict for multis).
        // Sits with the slice-22 wrap arm before the literal arms: a
        // holed wrap beats the concrete literal spelling where both
        // prove the same site set (D10); the general arm above declines
        // wraps by construction (Call / MethodCall subterms are never
        // general sites), and the literal arms keep every pair the wrap
        // core declines.
        if let Some(single) = attempt(
            declined,
            "call_arg_template",
            super::try_subline_bare_wrap_single(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "call_arg_template"));
            }
            declined_guard_with_numbers(
                declined,
                "call_arg_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-20: sub-line bare trailing-literal single under
        // the precision guard (whole-line pairs with `prefix >= 1`
        // decline here — slice-12 owns them; uniqueness proved by
        // `call_literal_holds` at n=1 plus the T361 byte-compare,
        // `guard_for("call_literal")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "call_literal",
            super::try_subline_literal_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "call_literal"));
            }
            declined_guard_with_numbers(
                declined,
                "call_literal",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-21: sub-line method same-literal single under
        // the precision guard (whole-line pairs decline here —
        // slice-13 owns them; uniqueness proved by
        // `method_literal_holds` at n=1 over the exact old-args tuple
        // plus the T361 byte-compare,
        // `guard_for("method_same_literal")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "method_same_literal",
            super::try_subline_method_literal_single(
                minuses, pluses, path, file_old, changed, used_rules,
            ),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "method_same_literal"));
            }
            declined_guard_with_numbers(
                declined,
                "method_same_literal",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-1 (repeat family): single-pair full-strip fold
        // under the precision guard (uniqueness proved by
        // `template_repeat_holds` at n=1 plus the T361 byte-compare with
        // `# repeat`; `guard_for("repeat_fold")` stays Strict for multis).
        if let Some(single) = attempt(
            declined,
            "repeat_fold",
            super::try_repeat_fold_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "repeat_fold"));
            }
            declined_guard_with_numbers(
                declined,
                "repeat_fold",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        // WI-7 slice-2 (let-template family): single-pair method
        // wrap-adds (typed, untyped) and the tuple-position swap under
        // the precision guard (uniqueness proved by
        // `template_pair_holds` at n=1 plus the T361 byte-compare;
        // `guard_for("let_template")` stays Strict for multis). Sits
        // last in the singles block: holed templates beat the literal
        // verbatim floor where both prove the same site set (D10), and
        // every tighter holed winner above keeps priority.
        if let Some(single) = attempt(
            declined,
            "let_template",
            super::try_typed_wrap_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "let_template"));
            }
            declined_guard_with_numbers(
                declined,
                "let_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        if let Some(single) = attempt(
            declined,
            "let_template",
            super::try_untyped_wrap_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "let_template"));
            }
            declined_guard_with_numbers(
                declined,
                "let_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
        if let Some(single) = attempt(
            declined,
            "let_template",
            super::try_pat_swap_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "let_template"));
            }
            declined_guard_with_numbers(
                declined,
                "let_template",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
    }
    let full: Option<(String, &'static str)> = if minuses.len() == 1 {
        // Single pairs: definition shapes under the precision guard
        // (the engine re-verifies the plus at its span), whole-arg
        // permutation under strict T375 (subterm-counted multi-site
        // swaps owe canonical-losslessness). WI-19b extends the
        // precision family past item-rename/definition-evolution to
        // slot/sig/header + def-only member/macro/assoc singles; WI-19c
        // adds self-ty `impl` header singles (scratch-apply probe).
        attempt(
            declined,
            "item_rename",
            super::try_item_rename(minuses[0], pluses[0], path, file_old, changed, used_rules),
        )
        .map(|f| (f, "item_rename"))
        .or_else(|| {
            attempt(
                declined,
                "precision_definition_single",
                super::try_precision_definition_single(
                    minuses, pluses, path, file_old, changed, used_rules,
                ),
            )
        })
        .or_else(|| {
            attempt(
                declined,
                "impl_header_single",
                super::try_impl_header_single(minuses, pluses, path, file_old, changed, used_rules),
            )
        })
        .or_else(|| {
            attempt_counted(
                declined,
                "call_arg_template",
                super::try_call_perm_sweep(minuses, pluses, path, file_old, changed, used_rules),
            )
            .map(|f| (f, "call_arg_template"))
        })
    } else {
        super::chain::run_pair_chain(
            minuses, pluses, path, file_old, changed, used_rules, declined,
        )
    };
    // WI-6 slice 1: last-resort literal verbatim single under the
    // precision guard. Runs only when every holed arm above AND the
    // single-pair definition/perm shapes in `full` decline, so tighter
    // spellings always win (including the single-pair perm sweep, which
    // owns multi-subterm lines like `swap(x, y) + swap(y, x)`).
    // Uniqueness proved at n=1 plus the T361 byte-compare inside;
    // `guard_for("verbatim")` stays Strict for multis.
    if full.is_none() && minuses.len() == 1 {
        if let Some(single) = attempt(
            declined,
            "verbatim",
            super::try_verbatim_single(minuses, pluses, path, file_old, changed, used_rules),
        ) {
            let body = single.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if super::guard::within_precision(body.len(), hunk_len, 1, 0) {
                return Some((body.to_string(), "verbatim"));
            }
            declined_guard_with_numbers(
                declined,
                "verbatim",
                "precision",
                body.len(),
                hunk_len,
                super::guard::precision_allowance(1, 0),
            );
            used_rules.truncate(checkpoint);
        }
    }
    match full {
        Some((f, detector)) => {
            let body = f.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            // WI-19: singles consult `guard_for` (precision for
            // definition shapes, strict for sweeps); multi-pair
            // sections keep strict T375 exactly as before.
            // Method (indented) `sig_evolution` N=1 singles carry the
            // mandatory WI-4a filter block. Measured ~41 over the native
            // pair hunk (filter + rule tag; no `---`/`+++` prelude to drop
            // per-hunk, unlike the whole-section arm), so the per-hunk
            // budget is four guards (64) — the same method-run price the
            // whole-section arm (32 + ~37 dropped prelude) and the subhunk
            // scaled guard pay. WI-2 slice 2: lone indented `fn_slot_sweep`
            // singles carry the identical mandatory filter block (compact
            // inline-body emission) under the same proof, so they ride
            // the same budget. The filter header itself is priced per
            // emission (`filter_header_excess`): a long enclosing `impl`
            // header contributes exactly its surplus over the measured
            // baseline, so only the proof anchor scales, never the guard.
            let wins = if minuses.len() == 1 {
                match super::guard_for(detector) {
                    super::Guard::Precision => {
                        let filtered_single = minuses[0].starts_with([' ', '\t'])
                            && (detector == "sig_evolution" || detector == "fn_slot_sweep");
                        if filtered_single {
                            super::guard::within_precision(
                                body.len(),
                                hunk_len,
                                4,
                                super::filter_header_excess(body),
                            )
                        } else {
                            super::guard::within_precision(body.len(), hunk_len, 1, 0)
                        }
                    }
                    super::Guard::Strict => super::guard::within_strict(body.len(), hunk_len),
                }
            } else {
                super::guard::within_strict(body.len(), hunk_len)
            };
            if wins {
                Some((body.to_string(), detector))
            } else {
                // WI-15c: the candidate fired but the hunk guard priced
                // it out — the guard owns the veto, not the shape.
                // WI-17 slice 2: the veto carries the WI-7 numbers in
                // the closed `guard:` form (same branches as `wins`).
                let (kind, allow) = if minuses.len() == 1
                    && matches!(super::guard_for(detector), super::Guard::Precision)
                {
                    let filtered_single = minuses[0].starts_with([' ', '\t'])
                        && (detector == "sig_evolution" || detector == "fn_slot_sweep");
                    if filtered_single {
                        (
                            "precision",
                            super::guard::precision_allowance(4, super::filter_header_excess(body)),
                        )
                    } else {
                        ("precision", super::guard::precision_allowance(1, 0))
                    }
                } else {
                    ("byte", 0)
                };
                declined_guard_with_numbers(declined, detector, kind, body.len(), hunk_len, allow);
                used_rules.truncate(checkpoint);
                None
            }
        }
        None => {
            used_rules.truncate(checkpoint);
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::super::HunkDeclined;
    #[test]
    fn wi17_slice2_item_rename_veto_carries_verdict_numbers() {
        // WI-17 slice 2: the per-hunk composed-guard veto records the
        // WI-7 numbers in the closed `guard:` form — never a bare
        // `precision-guard`. `hunk_len=0` forces the lossy branch on a
        // real `item_rename` candidate (const rename needs no other
        // arm: def/call/method/type/use/macro/template/let/verbatim
        // singles all decline the const shape first).
        let pre = "const OLD_A: usize = 1001;\n";
        let mut used_rules = Vec::new();
        let mut declined: Vec<HunkDeclined> = Vec::new();
        let out = super::compress_balanced_pair_body(
            &["const OLD_A: usize = 1001;"],
            &["const NEW_A: usize = 1001;"],
            "f.rs",
            Some(pre),
            &[1],
            0,
            &mut used_rules,
            &mut declined,
        );
        assert!(out.is_none(), "forced loss must veto, got: {out:?}");
        assert!(
            !declined.is_empty(),
            "the chain must record its trial, got none"
        );
        for d in &declined {
            assert!(
                d.reason != "precision-guard" && d.reason != "byte-guard",
                "bare guard veto emitted: {d:?}"
            );
        }
        let veto = declined
            .iter()
            .find(|d| d.detector == "item_rename" && d.reason.starts_with("guard:precision:"))
            .expect("item_rename must veto with numbered guard, got: {declined:?}");
        let body = veto.reason.clone();
        let sem: usize = body
            .split("sem=")
            .nth(1)
            .unwrap()
            .split(':')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let allow: usize = body.split("allow=").nth(1).unwrap().parse().unwrap();
        assert_eq!(
            allow,
            super::super::guard::precision_allowance(1, 0),
            "got: {veto:?}"
        );
        assert!(
            sem > 16,
            "veto must name its overflow (sem={sem}), got: {veto:?}"
        );
        assert!(
            body.contains("ord=0"),
            "forced hunk_len=0 prices ord=0, got: {veto:?}"
        );
    }
}
