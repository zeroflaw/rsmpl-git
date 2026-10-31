//! WI-6 follow-span probe (SPEC 24.6).
//!
//! Follow-only `RuleReport` entries earn `instantiated` iff every rewritten
//! use span verifies in the post-image *at that span*. Follow edits are
//! single-ident replacements at recorded spans by construction, so the check
//! is span-local ident equality over the post-image lines: the post span
//! must name the new ident as an exact ident token (`New` never matches
//! `New2`). Occurrence elsewhere in the file never counts.
//!
//! Shorthand-safe: `S { old }` -> `S { new: old }` contains `new` as an
//! ident, so the same check covers it; a reverted `S { old }` lacks `new`
//! and fails. Unparseable post-images abstain (`None` -> `reparse-only`).

use std::collections::{HashMap, HashSet};

use rsmpl_patch::StructuredError;
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::{translate_spans_to_post, TextEdit};
use crate::lines::FileState;

/// WI-6 pending follow-only probe (position, rule, compiled, file, spans,
/// optional new-name override for `ImplContext`, whose compiled kind
/// carries no names).
pub(crate) type FollowJob = (
    (usize, usize),
    String,
    CompiledPattern,
    String,
    Vec<(u32, u32)>,
    Option<String>,
);

/// New ident this follow proves, or `None` when the shape carries no
/// rename to verify.
pub(crate) fn follow_new_name(compiled: &CompiledPattern) -> Option<String> {
    match &compiled.kind {
        PatternKind::Fn { plus_name, .. } => Some(plus_name.clone()),
        PatternKind::ItemRename { plus_name, .. } => Some(plus_name.clone()),
        PatternKind::FieldRename { new, .. } => Some(new.clone()),
        PatternKind::VariantRename { new, .. } => Some(new.clone()),
        PatternKind::AssocRename { plus_name, .. } => Some(plus_name.clone()),
        PatternKind::ImplContext { .. } => None,
        _ => None,
    }
}

/// Resolve the probe name: explicit override (WI-3 `ImplContext`, whose
/// compiled kind carries no names) wins; otherwise the compiled rename
/// name.
fn probe_new_name(compiled: &CompiledPattern, override_name: Option<&str>) -> Option<String> {
    if let Some(n) = override_name {
        if !n.trim().is_empty() {
            return Some(n.to_string());
        }
    }
    follow_new_name(compiled)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Span-local exact-ident check: `needle` occurs in `hay` with non-ident
/// chars (or edges) on both sides. Empty needles never match.
fn contains_ident(hay: &str, needle: &str) -> bool {
    let n = needle.trim();
    if n.is_empty() {
        return false;
    }
    let hb: Vec<char> = hay.chars().collect();
    let nb: Vec<char> = n.chars().collect();
    if nb.len() > hb.len() {
        return false;
    }
    let mut i = 0;
    while i + nb.len() <= hb.len() {
        if hb[i..i + nb.len()] == nb[..]
            && (i == 0 || !is_ident_char(hb[i - 1]))
            && (i + nb.len() == hb.len() || !is_ident_char(hb[i + nb.len()]))
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Verify every post-image span names `new_name` as an exact ident.
///
/// `post_spans` are post-image coordinates; `None` (unparseable inputs)
/// downgrades to `reparse-only`. Otherwise `Some(true)` when every span
/// verifies, `Some(false)` when any is missing (caller fails loud).
pub(crate) fn verify_follow_spans(
    post_text: &str,
    post_parsed: Option<&syn::File>,
    post_spans: &[(u32, u32)],
    new_name: &str,
) -> Option<bool> {
    let _ = post_parsed?;
    let needle = new_name.trim();
    if needle.is_empty() || post_spans.is_empty() {
        return None;
    }
    let lines: Vec<&str> = post_text.lines().collect();
    if lines.is_empty() {
        return Some(false);
    }
    for (s, e) in post_spans {
        let s = (*s as usize).saturating_sub(1);
        let e = (*e as usize).saturating_sub(1);
        if s >= lines.len() {
            return Some(false);
        }
        let e = e.min(lines.len() - 1);
        if s > e {
            return Some(false);
        }
        if !contains_ident(&lines[s..=e].join("\n"), needle) {
            return Some(false);
        }
    }
    Some(true)
}

/// WI-6 follow-only upgrade (SPEC 24.6): every follow-only entry whose
/// spans all verify earns `instantiated`; any missing span fails loud;
/// abstentions stay `reparse-only`. Matched by (rule, file); returns the
/// key set so the definition loop can skip these entries.
#[allow(clippy::too_many_arguments)]
pub(crate) fn upgrade_follow_only_entries(
    rules: &mut [crate::RuleReport],
    jobs: &[FollowJob],
    states: &HashMap<String, FileState>,
    full_edits: &HashMap<String, Vec<TextEdit>>,
    files: &HashMap<String, String>,
    post_parsed: &HashMap<String, syn::File>,
) -> Result<HashSet<(String, String)>, StructuredError> {
    let keys: HashSet<(String, String)> = jobs
        .iter()
        .map(|(_, rule, _, file, _, _)| (rule.clone(), file.clone()))
        .collect();
    for (_, rule, compiled, file, spans, override_name) in jobs {
        let Some(r) = rules
            .iter_mut()
            .find(|r| &r.rule == rule && &r.file == file)
        else {
            continue;
        };
        let Some(new_name) = probe_new_name(compiled, override_name.as_deref()) else {
            continue;
        };
        let post_text = files.get(file).cloned().unwrap_or_default();
        let post_spans: Vec<(u32, u32)> = states
            .get(file)
            .map(|st| {
                translate_spans_to_post(
                    &st.original,
                    &st.index,
                    full_edits.get(file).map(Vec::as_slice).unwrap_or(&[]),
                    spans,
                )
            })
            .unwrap_or_else(|| spans.clone());
        match verify_follow_spans(&post_text, post_parsed.get(file), &post_spans, &new_name) {
            Some(true) => {
                r.semantics = crate::RuleSemantics::Instantiated;
            }
            Some(false) => {
                let range = spans
                    .first()
                    .map(|(s, e)| format!("{s}..{e}"))
                    .unwrap_or_else(|| "1..1".to_string());
                return Err(crate::postcheck::instantiation_error(rule, file, &range));
            }
            None => {}
        }
    }
    Ok(keys)
}

/// WI-6 same-file follow check: the `follow`-labeled subset of a
/// definition entry verifies identically. `rename`-labeled impl-def
/// claims stay under the definition probe (WI-1). Loud on mismatch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_definition_follows(
    rule: &str,
    file: &str,
    spans: &[(u32, u32)],
    span_labels: &[Option<String>],
    compiled: &CompiledPattern,
    override_name: Option<&str>,
    states: &HashMap<String, FileState>,
    full_edits: &HashMap<String, Vec<TextEdit>>,
    post_text: &str,
    post_parsed: Option<&syn::File>,
    range_desc: &str,
) -> Result<(), StructuredError> {
    let follow_orig: Vec<(u32, u32)> = spans
        .iter()
        .zip(span_labels.iter())
        .filter(|(_, l)| l.as_deref() == Some("follow"))
        .map(|(s, _)| *s)
        .collect();
    if follow_orig.is_empty() {
        return Ok(());
    }
    let Some(new_name) = probe_new_name(compiled, override_name) else {
        return Ok(());
    };
    let follow_post: Vec<(u32, u32)> = states
        .get(file)
        .map(|st| {
            translate_spans_to_post(
                &st.original,
                &st.index,
                full_edits.get(file).map(Vec::as_slice).unwrap_or(&[]),
                &follow_orig,
            )
        })
        .unwrap_or_else(|| follow_orig.clone());
    match verify_follow_spans(post_text, post_parsed, &follow_post, &new_name) {
        Some(true) | None => Ok(()),
        Some(false) => Err(crate::postcheck::instantiation_error(
            rule, file, range_desc,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_use_verifies_and_revert_fails() {
        let parsed = syn::parse_str::<syn::File>("fn f() { new(1); }\n").ok();
        assert_eq!(
            verify_follow_spans("fn f() { new(1); }\n", parsed.as_ref(), &[(1, 1)], "new"),
            Some(true)
        );
        let parsed = syn::parse_str::<syn::File>("fn f() { old(1); }\n").ok();
        assert_eq!(
            verify_follow_spans("fn f() { old(1); }\n", parsed.as_ref(), &[(1, 1)], "new"),
            Some(false)
        );
    }

    #[test]
    fn prefix_hazard_never_counts() {
        let parsed = syn::parse_str::<syn::File>("fn f() { new2(1); }\n").ok();
        assert_eq!(
            verify_follow_spans("fn f() { new2(1); }\n", parsed.as_ref(), &[(1, 1)], "new"),
            Some(false)
        );
    }

    #[test]
    fn shorthand_contains_new_and_revert_fails() {
        let parsed = syn::parse_str::<syn::File>("fn f() { let x = S { new: old }; }\n").ok();
        assert_eq!(
            verify_follow_spans(
                "fn f() { let x = S { new: old }; }\n",
                parsed.as_ref(),
                &[(1, 1)],
                "new"
            ),
            Some(true)
        );
        let parsed = syn::parse_str::<syn::File>("fn f() { let x = S { old }; }\n").ok();
        assert_eq!(
            verify_follow_spans(
                "fn f() { let x = S { old }; }\n",
                parsed.as_ref(),
                &[(1, 1)],
                "new"
            ),
            Some(false)
        );
    }

    #[test]
    fn elsewhere_does_not_satisfy_span() {
        let parsed = syn::parse_str::<syn::File>("fn f() { old(1); }\nfn g() { new(1); }\n").ok();
        assert_eq!(
            verify_follow_spans(
                "fn f() { old(1); }\nfn g() { new(1); }\n",
                parsed.as_ref(),
                &[(1, 1)],
                "new"
            ),
            Some(false)
        );
    }

    #[test]
    fn unparseable_abstains() {
        assert_eq!(verify_follow_spans("fn f( {", None, &[(1, 1)], "new"), None);
    }
}
