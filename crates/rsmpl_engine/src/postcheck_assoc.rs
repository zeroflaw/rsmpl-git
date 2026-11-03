//! Assoc-evolution postcondition helper (WI-26, SPEC 24.6).
//!
//! Split from `postcheck` per D5 (the parent sits over the 2000-line
//! gate). One small helper: build the `header + pair` plus-lines for an
//! assoc evolution kind and run the shared structural re-match.

use std::collections::HashMap;

use rsmpl_pattern::PatternKind;

use crate::lines::LineIndex;

/// Span-local ident-equality check for an assoc rename rule (the
/// WI-1.4 posture shared with item renames). Returns `None` when the
/// kind is not an assoc rename.
pub(crate) fn verify_assoc_rename(
    kind: &PatternKind,
    rh: &rsmpl_patch::RsmplHunk,
    post_text: &str,
    post_parsed: Option<&syn::File>,
    spans: &[(u32, u32)],
    matches: usize,
) -> Option<Option<bool>> {
    let (plus_name, minus_attrs, plus_attrs) = match kind {
        PatternKind::AssocRename {
            plus_name,
            minus_attrs,
            plus_attrs,
            ..
        } => (plus_name, minus_attrs, plus_attrs),
        _ => return None,
    };
    Some(match post_parsed {
        Some(parsed) => {
            let preserve = rh.attrs_policy() == rsmpl_patch::AttrsPolicy::Preserve;
            let (m, p): (&[String], &[String]) = if preserve {
                (minus_attrs, plus_attrs)
            } else {
                (&[], &[])
            };
            Some(
                crate::postcheck_attr::count_spans_named_with_attrs(
                    parsed,
                    crate::postcheck_defs::DefKind::Item,
                    Some(plus_name.trim()),
                    m,
                    p,
                    spans,
                ) >= matches.max(1),
            )
        }
        None => Some(
            crate::postcheck::spans_count(post_text, spans, plus_name, |h| post_text.contains(h))
                >= matches.max(1),
        ),
    })
}

/// Build the plus-lines for an assoc evolution rule and verify them at
/// the reported spans. Returns `None` when the kind is not an assoc
/// evolution (caller falls through) or when verification abstains.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_assoc_evolution(
    kind: &PatternKind,
    rh: &rsmpl_patch::RsmplHunk,
    bindings_list: &[HashMap<String, String>],
    post_text: &str,
    post_parsed: Option<&syn::File>,
    post_index: Option<&LineIndex>,
    spans: &[(u32, u32)],
    matches: usize,
) -> Option<Option<bool>> {
    let (impl_header, name, ty, plus_raw, is_type) = match kind {
        PatternKind::AssocEvolution {
            impl_header,
            name,
            ty,
            plus_raw,
            is_type,
            ..
        } => (impl_header, name, ty, plus_raw, *is_type),
        _ => return None,
    };
    let (parsed, index) = match (post_parsed, post_index) {
        (Some(p), Some(i)) => (p, i),
        _ => return Some(None),
    };
    let pair = if is_type {
        format!("type {name} = {plus_raw};")
    } else {
        format!("const {name}: {ty} = {plus_raw};")
    };
    Some(crate::postcheck::verify_plus_pattern_at_spans(
        rh,
        &[impl_header.clone(), pair],
        bindings_list,
        post_text,
        parsed,
        index,
        spans,
        matches,
    ))
}
