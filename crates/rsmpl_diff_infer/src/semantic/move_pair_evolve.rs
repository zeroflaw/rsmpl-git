//! Evolved-pair construction for the move pre-pass (WI-8 slice 9 split).
//!
//! Byte-identical extraction of this cascade out of `moves.rs` (D5: that
//! file sat at 2015/2000). When no byte-identical plus run exists for a
//! minus item, every plus hunk is probed with the nine header-evolution
//! finders (`move_evolve`) plus the signature-evolution finder
//! (`move_evolve_sig`); each hit becomes a `MovePair` with an
//! `EvolvedPair` (concrete template for signature-evolving `fn`, `None`
//! otherwise). Push order is preserved exactly (plus hunk outer, finder
//! order inner) — the downstream plus-run dedup keeps the first claim
//! per run, so order is load-bearing. Zero behavior change.

use super::move_parts::{EvolvedKind, EvolvedPair, MoveItem, MovePair, SectionParts};

/// All evolved pairs for one minus item with no byte-identical run.
pub(crate) fn evolved_pairs_for_item(
    item: &MoveItem,
    parsed: &[SectionParts],
    plus_hunks: &[(usize, usize)],
    minus_key: (usize, usize),
) -> Vec<MovePair> {
    let mut pairs = Vec::new();
    for (psi, phi) in plus_hunks {
        let pbody = &parsed[*psi].hunks[*phi].body;
        let plus_key = (*psi, *phi);
        push_abstract(
            &mut pairs,
            item,
            minus_key,
            plus_key,
            super::move_evolve::find_evolved_hits(&item.text, pbody),
            EvolvedKind::Fn,
        );
        // WI-6 slice 3b: body-evolving `fn` pairs (move+rename whose body
        // renames its self-references). The paste carries the `as`
        // template (`fn <old> as <new>(...) { ... }`); the engine renames
        // the captured body through the same scanner that proves the
        // pairing, so splice and probe agree by construction.
        for (first, last, prefix, old, name) in
            super::move_evolve::find_fn_body_evolved_hits(&item.text, pbody)
        {
            let template = super::move_evolve::body_evolution_template(&prefix, &old, &name);
            pairs.push(MovePair {
                minus_hunk_key: minus_key,
                plus_hunk_key: plus_key,
                item: item.clone(),
                plus_first: first,
                plus_last: last,
                evolved: Some(EvolvedPair {
                    kind: EvolvedKind::Fn,
                    prefix,
                    name,
                    static_mut: false,
                    template: Some(template),
                }),
            });
        }
        // WI-8 slice 9: signature-evolving `fn` pairs carry the concrete
        // plus-side header as the paste template (mutually exclusive
        // with the abstract finder above — sig-identical pairs return
        // `None` here, never double-pair).
        for (first, last, prefix, template, name) in
            super::move_evolve_sig::find_fn_sig_evolved_hits(&item.text, pbody)
        {
            pairs.push(MovePair {
                minus_hunk_key: minus_key,
                plus_hunk_key: plus_key,
                item: item.clone(),
                plus_first: first,
                plus_last: last,
                evolved: Some(EvolvedPair {
                    kind: EvolvedKind::Fn,
                    prefix,
                    name,
                    static_mut: false,
                    template: Some(template),
                }),
            });
        }
        push_abstract(
            &mut pairs,
            item,
            minus_key,
            plus_key,
            super::move_evolve::find_struct_evolved_hits(&item.text, pbody),
            EvolvedKind::Struct,
        );
        push_abstract(
            &mut pairs,
            item,
            minus_key,
            plus_key,
            super::move_evolve::find_enum_evolved_hits(&item.text, pbody),
            EvolvedKind::Enum,
        );
        push_abstract(
            &mut pairs,
            item,
            minus_key,
            plus_key,
            super::move_evolve::find_union_evolved_hits(&item.text, pbody),
            EvolvedKind::Union,
        );
        push_abstract(
            &mut pairs,
            item,
            minus_key,
            plus_key,
            super::move_evolve::find_type_evolved_hits(&item.text, pbody),
            EvolvedKind::Type,
        );
        push_abstract(
            &mut pairs,
            item,
            minus_key,
            plus_key,
            super::move_evolve::find_const_evolved_hits(&item.text, pbody),
            EvolvedKind::Const,
        );
        // `static` carries the plus-side `mut` verbatim.
        for (first, last, prefix, name, is_mut) in
            super::move_evolve::find_static_evolved_hits(&item.text, pbody)
        {
            pairs.push(MovePair {
                minus_hunk_key: minus_key,
                plus_hunk_key: plus_key,
                item: item.clone(),
                plus_first: first,
                plus_last: last,
                evolved: Some(EvolvedPair {
                    kind: EvolvedKind::Static,
                    prefix,
                    name,
                    static_mut: is_mut,
                    template: None,
                }),
            });
        }
        push_abstract(
            &mut pairs,
            item,
            minus_key,
            plus_key,
            super::move_evolve::find_mod_evolved_hits(&item.text, pbody),
            EvolvedKind::Mod,
        );
        push_abstract(
            &mut pairs,
            item,
            minus_key,
            plus_key,
            super::move_evolve::find_trait_evolved_hits(&item.text, pbody),
            EvolvedKind::Trait,
        );
    }
    pairs
}

/// One abstract-finder hit list into pairs (no template override, no
/// `mut` — the signature-evolving `fn` and `static mut` shapes above
/// construct their pairs inline).
fn push_abstract(
    out: &mut Vec<MovePair>,
    item: &MoveItem,
    minus_key: (usize, usize),
    plus_key: (usize, usize),
    hits: Vec<(usize, usize, String, String)>,
    kind: EvolvedKind,
) {
    for (first, last, prefix, name) in hits {
        out.push(MovePair {
            minus_hunk_key: minus_key,
            plus_hunk_key: plus_key,
            item: item.clone(),
            plus_first: first,
            plus_last: last,
            evolved: Some(EvolvedPair {
                kind,
                prefix,
                name,
                static_mut: false,
                template: None,
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    #[test]
    fn body_evolve_124_fires_with_as_paste() {
        // WI-6 slice 3b: the 124-shape — move+rename whose body renames
        // its recursive call — fires as one evolving cut/paste with the
        // `as` template carrying the body delta.
        let minus = "fn fact(n: u64) -> u64 {\n    if n <= 1 {\n        return 1;\n    }\n    n * fact(n - 1)\n}\n";
        let plus = "fn factorial(n: u64) -> u64 {\n    if n <= 1 {\n        return 1;\n    }\n    n * factorial(n - 1)\n}\n";
        let a_minus: String = minus.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,6 +2,0 @@\n{a_minus}");
        let b_plus: String = plus.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,6 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{minus}")),
            ("b.rs", "fn keep2() {}\n"),
        ]);
        let mut used = Vec::new();
        let out =
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).expect("124 must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("+ fn fact as factorial(...) { ... }"),
            "paste must carry the `as` template, got:\n{text}"
        );
        assert!(
            !text.contains("-fn fact"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn factorial"),
            "plus lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn stale_self_ref_still_fires_verbatim() {
        // WI-6 slice 3b regression pin: a move+rename whose plus body
        // keeps the stale self-reference still fires the OLD verbatim
        // spelling (the `as` form never claims token-equal bodies).
        let minus = "fn fact(n: u64) -> u64 {\n    n * fact(n - 1)\n}\n";
        let plus = "fn factorial(n: u64) -> u64 {\n    n * fact(n - 1)\n}\n";
        let a_minus: String = minus.lines().map(|l| format!("-{l}\n")).collect();
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,3 +2,0 @@\n{a_minus}");
        let b_plus: String = plus.lines().map(|l| format!("+{l}\n")).collect();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3,3 @@\n{b_plus}");
        let pre = pre_map(&[
            ("a.rs", &format!("fn keep() {{}}\n{minus}")),
            ("b.rs", "fn keep2() {}\n"),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("stale shape must still fire verbatim");
        let text = out.join("");
        assert!(
            text.contains("+ fn factorial(...) { ... }"),
            "paste must keep the verbatim spelling, got:\n{text}"
        );
        assert!(!text.contains(" as "), "no `as` template, got:\n{text}");
        validated_live(&text, &pre);
    }
}
