//! Whole-patch pre-pass chain for `infer_semantic_with_provenance`
//! (D5 split: this chain lived in `mod.rs`, which was at 1993/2000).
//! Order is dependency order — move → item-intent → joint-`impl` →
//! fn-joint → member → mod → cross — and each stage runs on the previous
//! stage's output (candidate sets are disjoint by construction, so stages
//! never double-claim). Mirrors the `cross_sweep::try_cross_intents`
//! owned-vec chaining precedent, but stays borrowed (`Cow`) while no
//! stage fires so the common no-pre-pass path pays no clone.
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use super::prov::{HunkDeclined, HunkProvenance};
use super::{
    cross_sweep, fn_joint_driver, item_intent, item_joint_impl, member, mod_rename, moves, prov,
};

/// WI-4 slice 1: pre-pass attribution — each stage that declines records
/// one patch-level `{detector, "no-match"}` entry (silence removed).
/// Firing stages record nothing. Finer counted reasons per pre-pass are
/// slice 2; this skeleton proves the plumbing (decline is attributed).
pub(crate) fn run_pre_passes<'a>(
    sections: &'a [String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
    prov: &mut Vec<HunkProvenance>,
    pre_pass_declined: &mut Vec<HunkDeclined>,
    compressed_any: &mut bool,
) -> (Cow<'a, [String]>, HashSet<(String, String)>) {
    fn decline(out: &mut Vec<HunkDeclined>, detector: &str) {
        out.push(HunkDeclined {
            detector: detector.to_string(),
            reason: prov::reason_no_match(),
        });
    }
    // T430 whole-patch move pre-pass (cross-section bind/use pairs);
    // per-section detectors run on the rewritten sections below.
    let move_rules_before = used_rules.len();
    let mut current: Cow<'a, [String]> = match moves::try_move_sweep(sections, pre, used_rules) {
        Some(rewritten) => {
            *compressed_any = true;
            prov::push_move_prov(
                prov,
                sections,
                &rewritten,
                pre,
                move_rules_before,
                used_rules,
            );
            Cow::Owned(rewritten)
        }
        None => {
            decline(pre_pass_declined, "move");
            Cow::Borrowed(sections)
        }
    };
    // WI-15a whole-patch item-intent pre-pass (single definition +
    // same-delta uses joined into one follow hunk); per-section
    // detectors run on the rewritten sections below.
    let intent_rules_before = used_rules.len();
    if let Some((rewritten, def_path, _rule)) =
        item_intent::try_item_intent_sweep(current.as_ref(), pre, used_rules)
    {
        current = Cow::Owned(prov::commit_pre_pass(
            prov,
            current.as_ref(),
            rewritten,
            &def_path,
            "item_intent",
            intent_rules_before,
            used_rules.as_slice(),
            pre,
            compressed_any,
        ));
    } else {
        decline(pre_pass_declined, "item_intent");
    }
    // WI-3 whole-patch joint-`impl` pre-pass (trait + self-type renames
    // composed in one `impl` header collapse to one shared-rule intent);
    // runs on the item-intent output (disjoint: two-def triples never
    // candidates there, never suppressed).
    let joint_impl_rules_before = used_rules.len();
    if let Some((rewritten, def_path, _rule)) =
        item_joint_impl::try_joint_impl_rename(current.as_ref(), pre, used_rules)
    {
        current = Cow::Owned(prov::commit_pre_pass(
            prov,
            current.as_ref(),
            rewritten,
            &def_path,
            "item_joint_impl",
            joint_impl_rules_before,
            used_rules.as_slice(),
            pre,
            compressed_any,
        ));
    } else {
        decline(pre_pass_declined, "item_joint_impl");
    }
    // WI-4a whole-patch fn-joint pre-pass (single `fn` rename+slot
    // definition + callee-only call uses in one file collapse to one
    // follow hunk; vetoed joints suppress their old callee name so the
    // per-hunk fallback stays wholly ordinary). Runs on the item-intent
    // output (disjoint: pure renames never candidates, never suppressed).
    let joint_rules_before = used_rules.len();
    let mut joint_suppressed: HashSet<(String, String)> = HashSet::new();
    if let Some((rewritten, def_path, rule, suppressed)) =
        fn_joint_driver::try_fn_joint_intent(current.as_ref(), pre, used_rules)
    {
        joint_suppressed = suppressed;
        if rule.is_empty() {
            current = Cow::Owned(rewritten);
        } else {
            *compressed_any = true;
            let emitted = rewritten
                .iter()
                .find(|s| {
                    s.lines().next().is_some_and(|h| {
                        h.starts_with("diff --git ")
                            && crate::b_side_path(&h["diff --git ".len()..])
                                .is_some_and(|p| p == def_path)
                    })
                })
                .cloned()
                .unwrap_or_default();
            let original: String = current
                .as_ref()
                .iter()
                .filter(|s| !rewritten.iter().any(|r| r == *s))
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("");
            let changed: Vec<usize> = current
                .as_ref()
                .iter()
                .filter(|s| !rewritten.iter().any(|r| r == *s))
                .flat_map(|s| crate::totality::changed_old_lines(s.as_str()))
                .collect();
            if !emitted.is_empty() && !original.is_empty() {
                let detector_of: Vec<(String, &'static str)> = used_rules[joint_rules_before..]
                    .iter()
                    .map(|r| (r.clone(), "fn_joint"))
                    .collect();
                prov::commit_prov(
                    prov,
                    &emitted,
                    &original,
                    &def_path,
                    &detector_of,
                    &changed,
                    pre.get(&def_path).map(String::as_str),
                );
            }
            current = Cow::Owned(rewritten);
        }
    } else {
        decline(pre_pass_declined, "fn_joint");
    }
    // WI-15b whole-patch member-intent pre-pass (single member definition
    // + same-delta uses across files, file/section-pure partial); runs on
    // the fn-joint output (disjoint def shapes, no double-claim).
    let member_rules_before = used_rules.len();
    if let Some((rewritten, def_path, _rule)) =
        member::try_member_intent_sweep(current.as_ref(), pre, used_rules)
    {
        current = Cow::Owned(prov::commit_pre_pass(
            prov,
            current.as_ref(),
            rewritten,
            &def_path,
            "member_rename",
            member_rules_before,
            used_rules.as_slice(),
            pre,
            compressed_any,
        ));
    } else {
        decline(pre_pass_declined, "member_rename");
    }
    // WI-15c whole-patch mod-intent pre-pass (single `mod old;` def +
    // same-delta `old::` path uses across files + optional headers-only
    // file rename, file/section-pure partial); runs on the member-intent
    // output (disjoint def shapes: `mod`-only this claims).
    let mod_rules_before = used_rules.len();
    if let Some((rewritten, def_path, _rule)) =
        mod_rename::try_mod_rename_intent_sweep(current.as_ref(), pre, used_rules)
    {
        current = Cow::Owned(prov::commit_pre_pass(
            prov,
            current.as_ref(),
            rewritten,
            &def_path,
            "mod_rename",
            mod_rules_before,
            used_rules.as_slice(),
            pre,
            compressed_any,
        ));
    } else {
        decline(pre_pass_declined, "mod_rename");
    }
    // WI-12 cross-file assoc/trait-method intents (commits inside).
    if let Some(rewritten) =
        cross_sweep::try_cross_intents(current.as_ref(), pre, used_rules, prov, compressed_any)
    {
        current = Cow::Owned(rewritten);
    } else {
        decline(pre_pass_declined, "cross");
    }
    (current, joint_suppressed)
}
