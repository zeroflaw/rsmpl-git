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
