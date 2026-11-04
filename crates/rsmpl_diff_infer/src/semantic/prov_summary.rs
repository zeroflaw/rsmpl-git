//! WI-16 provenance token diet: weight ordering + top-3 summary.
//!
//! Split from `prov.rs` under the D5 1600-line rule (`prov.rs` sits at
//! ~1593 lines): this sibling owns the display-side ordering only.
//! Stored `declined` lists keep chain order (full mode prints them
//! verbatim, byte-identical to HEAD); the summary view and the JSON
//! report both read through [`sorted_declined`], so text and JSON name
//! the same blockers first. No detector writes through this module.

use super::prov::HunkDeclined;

/// WI-16 display weight (SPEC D.2 actionability): `guard:` /
/// `totality:` / `veto:` (0, what nearly fired and why) before
/// `precondition:` (1, the shape premise that failed) before
/// `no-match` (2, the shape never engaged) before `not-tried`
/// (3, absence — least actionable). Legacy bare `byte-guard` /
/// `precision-guard` weigh as guard (0); bare gate spellings weigh by
/// their closed form (`crlf-content` → 1, the rest → 0).
pub fn declined_weight(reason: &str) -> u8 {
    if reason == "no-match" {
        return 2;
    }
    if reason == "not-tried" {
        return 3;
    }
    if let Some(rest) = reason.strip_prefix("guard:") {
        let _ = rest;
        return 0;
    }
    if reason.starts_with("totality:") || reason.starts_with("veto:") {
        return 0;
    }
    if reason.starts_with("precondition:") {
        return 1;
    }
    if reason == "byte-guard" || reason == "precision-guard" {
        return 0;
    }
    if reason == "crlf-content" {
        return 1;
    }
    if reason == "stranded-joint" || reason == "cross-kind-member" || reason == "unbalanced-hunks" {
        return 0;
    }
    // Closed-taxonomy reasons all matched above; anything else sorts
    // with `no-match` (never hides — the taxonomy pin rejects it).
    2
}

/// WI-16 stable weight sort: actionable blockers first, ties keep the
/// stored chain order (sort is stable). Both the default text summary
/// (top-3 of this) and the JSON report (full array in this order) read
/// through here.
pub fn sorted_declined(declined: &[HunkDeclined]) -> Vec<HunkDeclined> {
    let mut out = declined.to_vec();
    out.sort_by_key(|d| declined_weight(&d.reason));
    out
}

/// WI-16 per-kind totals across ordinary `declined` entries
/// (pre-truncation truths): `(guard, totality, veto, precondition,
/// no-match, not-tried)`. Legacy bare spellings count under their
/// closed kind.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DeclinedTotals {
    pub guard: usize,
    pub totality: usize,
    pub veto: usize,
    pub precondition: usize,
    pub no_match: usize,
    pub not_tried: usize,
}

/// WI-16 totals over the full (pre-truncation) ordinary trial.
pub fn summary_counts(lists: &[&[HunkDeclined]]) -> DeclinedTotals {
    let mut t = DeclinedTotals::default();
    for list in lists {
        for d in *list {
            let r = d.reason.as_str();
            if r.starts_with("guard:") || r == "byte-guard" || r == "precision-guard" {
                t.guard += 1;
            } else if r.starts_with("totality:") {
                t.totality += 1;
            } else if r.starts_with("veto:")
                || r == "stranded-joint"
                || r == "cross-kind-member"
                || r == "unbalanced-hunks"
            {
                t.veto += 1;
            } else if r.starts_with("precondition:") || r == "crlf-content" {
                t.precondition += 1;
            } else if r == "no-match" {
                t.no_match += 1;
            } else if r == "not-tried" {
                t.not_tried += 1;
            } else {
                t.no_match += 1;
            }
        }
    }
    t
}

/// WI-16 totals line (single stderr line, no `declined: ` prefix so the
/// `text == JSON trial-set` pins keep reading only entry lines).
pub fn totals_line(t: &DeclinedTotals) -> String {
    format!(
        "declined totals: guard={} totality={} veto={} precondition={} no-match={} not-tried={}",
        t.guard, t.totality, t.veto, t.precondition, t.no_match, t.not_tried
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declined(names: &[(&str, &str)]) -> Vec<HunkDeclined> {
        names
            .iter()
            .map(|(d, r)| HunkDeclined {
                detector: d.to_string(),
                reason: r.to_string(),
            })
            .collect()
    }

    #[test]
    fn weight_orders_actionability_before_absence() {
        // guard/totality/veto (0) < precondition (1) < no-match (2) <
        // not-tried (3).
        assert_eq!(declined_weight("guard:byte:sem=10:ord=5:allow=0"), 0);
        assert_eq!(declined_weight("totality:5:2"), 0);
        assert_eq!(declined_weight("veto:opaque-hit"), 0);
        assert_eq!(declined_weight("precondition:crlf-content"), 1);
        assert_eq!(declined_weight("no-match"), 2);
        assert_eq!(declined_weight("not-tried"), 3);
        // Legacy bare spellings weigh by their closed form.
        assert_eq!(declined_weight("byte-guard"), 0);
        assert_eq!(declined_weight("precision-guard"), 0);
        assert_eq!(declined_weight("crlf-content"), 1);
    }

    #[test]
    fn stable_sort_keeps_chain_order_within_weight() {
        // Ties keep stored chain order (stable): the two vetoes stay
        // b-before-a, and the counted totality leads the group.
        let v = declined(&[
            ("b", "veto:opaque-hit"),
            ("a", "veto:reprobe-failed"),
            ("c", "no-match"),
            ("d", "not-tried"),
            ("e", "totality:2:1"),
        ]);
        let s = sorted_declined(&v);
        let names: Vec<&str> = s.iter().map(|d| d.detector.as_str()).collect();
        assert_eq!(names, vec!["b", "a", "e", "c", "d"]);
    }

    #[test]
    fn top_three_names_same_blockers_first() {
        // The default text summary (top-3 of the sorted clone) names
        // the counted decline before the shape declines.
        let v = declined(&[
            ("call_sweep", "no-match"),
            ("general_expr", "totality:6:2"),
            ("verbatim", "no-match"),
            ("delete", "no-match"),
        ]);
        let s = sorted_declined(&v);
        assert_eq!(s[0].detector, "general_expr");
        assert_eq!(s[0].reason, "totality:6:2");
    }

    #[test]
    fn totals_count_pre_truncation_kinds() {
        let a = declined(&[
            ("x", "guard:byte:sem=9:ord=5:allow=0"),
            ("y", "totality:2:1"),
            ("z", "no-match"),
        ]);
        let b = declined(&[("w", "not-tried"), ("v", "precondition:crlf-content")]);
        let t = summary_counts(&[&a, &b]);
        assert_eq!(
            t,
            DeclinedTotals {
                guard: 1,
                totality: 1,
                veto: 0,
                precondition: 1,
                no_match: 1,
                not_tried: 1,
            }
        );
        let line = totals_line(&t);
        assert!(line.starts_with("declined totals: "), "got: {line}");
        assert!(!line.starts_with("declined: "), "got: {line}");
    }
}
