//! WI-6 follow-span probe (SPEC 24.6), WI-6b byte-tight.
//!
//! Follow-only `RuleReport` entries earn `instantiated` iff every rewritten
//! use span verifies in the post-image *at that byte span*. Follow edits are
//! single-ident replacements at recorded spans by construction, so the check
//! is byte-exact replacement equality at the translated post offset:
//! the post slice must equal the recorded replacement (`new`, or
//! `S { new: old }` wider form) with ident boundaries. Occurrence elsewhere
//! in the file — including elsewhere on the same line — never counts.
//!
//! Unparseable post-images abstain (`None` -> `reparse-only`).

use std::collections::{HashMap, HashSet};

use rsmpl_patch::StructuredError;
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::{translate_spans_to_post, TextEdit};
use crate::lines::FileState;

/// WI-6 pending follow-only probe (position, rule, compiled, file, spans,
/// optional new-name override for `ImplContext`, whose compiled kind
/// carries no names).
/// WI-6b: `bytes` carries the original-coordinate follow edits
/// `(start, end, replacement)` parallel to `spans`; the probe verifies
/// the post-image slice at the translated byte offset (exact replacement
/// equality), so a sibling `new` elsewhere on the same line never satisfies it.
pub(crate) type FollowJob = (
    (usize, usize),
    String,
    CompiledPattern,
    String,
    Vec<(u32, u32)>,
    Option<String>,
    Vec<(usize, usize, String)>,
);

/// New ident this follow proves, or `None` when the shape carries no
/// rename to verify.
pub(crate) fn follow_new_name(compiled: &CompiledPattern) -> Option<String> {
    match &compiled.kind {
        PatternKind::Fn { plus_name, .. } => Some(plus_name.clone()),
        PatternKind::ItemRename { plus_name, .. } => Some(plus_name.clone()),
        PatternKind::StructRenameFieldEvo { new_container, .. } => Some(new_container.clone()),
        PatternKind::EnumRenameVariantEvo { new_container, .. } => Some(new_container.clone()),
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

/// WI-6b: translate original-coordinate follow edits to post-image offsets.
///
/// `orig` holds `(start, end, replacement)` in original coordinates;
/// `all` holds every `TextEdit` for the file (original coordinates, any
/// order). Post offset for an edit = `start + shift_before`, where
/// `shift_before` is the net byte delta of all edits strictly before it
/// (sorted ascending by `(start, end)`, stable so same-point insertions
/// keep document order). Returns `(post_start, replacement)` pairs in the
/// input order; edits missing from `all` are skipped (caller treats a
/// length mismatch as a loud failure).
pub(crate) fn post_starts_for(
    orig: &[(usize, usize, String)],
    all: &[TextEdit],
) -> Vec<(usize, String)> {
    let mut sorted: Vec<&TextEdit> = all.iter().collect();
    sorted.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    let mut out: Vec<(usize, String)> = Vec::with_capacity(orig.len());
    for (os, oe, repl) in orig {
        let mut shift: i64 = 0;
        let mut found = false;
        for e in &sorted {
            if e.start == *os && e.end == *oe && e.replacement == *repl {
                found = true;
                break;
            }
            if e.end <= *os {
                shift += e.replacement.len() as i64 - (e.end as i64 - e.start as i64);
            } else if e.start < *os {
                // Overlapping predecessor (should not happen: overlaps are
                // rejected at plan time); count its delta conservatively.
                shift += e.replacement.len() as i64 - (e.end as i64 - e.start as i64);
            }
        }
        if found {
            let ps = (*os as i64 + shift).max(0) as usize;
            out.push((ps, repl.clone()));
        }
    }
    out
}

/// WI-6b: per-(rule, file) original-coordinate `follow` edits backing the
/// byte-exact same-file probe. The report keeps line spans; this map keeps
/// the bytes the probe verifies.
pub(crate) type DefFollowMap = HashMap<(String, String), Vec<(usize, usize, String)>>;

/// WI-6b: collect original-coordinate `(start, end, replacement)` triples
/// for `follow`-labeled edits in a hunk's new-edit slice.
pub(crate) fn collect_def_follow_bytes(new_edits: &[TextEdit]) -> Vec<(usize, usize, String)> {
    new_edits
        .iter()
        .filter(|e| e.label == Some("follow"))
        .map(|e| (e.start, e.end, e.replacement.clone()))
        .collect()
}

/// WI-6b: record a hunk's `follow`-labeled bytes under `(rule, file)`.
pub(crate) fn record_def_follow(
    map: &mut DefFollowMap,
    rule: &str,
    file: &str,
    new_edits: &[TextEdit],
) {
    let bytes = collect_def_follow_bytes(new_edits);
    if !bytes.is_empty() {
        map.entry((rule.to_string(), file.to_string()))
            .or_default()
            .extend(bytes);
    }
}

/// WI-6b: fetch recorded `follow` bytes for `(rule, file)`.
pub(crate) fn def_follow_for(
    map: &DefFollowMap,
    rule: &str,
    file: &str,
) -> Vec<(usize, usize, String)> {
    map.get(&(rule.to_string(), file.to_string()))
        .cloned()
        .unwrap_or_default()
}

/// WI-6b byte-exact follow check: every `(post_start, replacement)` pair
/// must match the post-image slice exactly, with ident boundaries.
/// `None` on unparseable inputs (abstain -> `reparse-only`).
pub(crate) fn verify_follow_bytes(
    post_text: &str,
    post_parsed: Option<&syn::File>,
    post_items: &[(usize, String)],
) -> Option<bool> {
    let _ = post_parsed?;
    if post_items.is_empty() {
        return None;
    }
    for (ps, repl) in post_items {
        let pe = ps.saturating_add(repl.len());
        if pe > post_text.len() {
            return Some(false);
        }
        if post_text.get(*ps..pe) != Some(repl.as_str()) {
            return Some(false);
        }
        // Ident boundaries: replacement starts/ends with ident chars in the
        // plain case (`new`); the shorthand form (`new: old`) starts/ends
        // with ident chars too, so the outer-edge check covers both.
        let before_ok = if *ps == 0 {
            true
        } else {
            post_text[..*ps]
                .chars()
                .next_back()
                .is_none_or(|c| !is_ident_char(c))
        };
        let after_ok = if pe == post_text.len() {
            true
        } else {
            post_text[pe..]
                .chars()
                .next()
                .is_none_or(|c| !is_ident_char(c))
        };
        if repl.chars().next().is_some_and(is_ident_char) && !before_ok {
            return Some(false);
        }
        if repl.chars().next_back().is_some_and(is_ident_char) && !after_ok {
            return Some(false);
        }
    }
    Some(true)
}

/// Verify every post-image span names `new_name` as an exact ident.
///
/// `post_spans` are post-image coordinates; `None` (unparseable inputs)
/// downgrades to `reparse-only`. Otherwise `Some(true)` when every span
/// verifies, `Some(false)` when any is missing (caller fails loud).
/// WI-6b legacy fallback: used when byte triples are absent and by unit
/// tests; production paths prefer `verify_follow_bytes`.
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
/// WI-6b: byte-exact when `bytes` are present, line-span fallback otherwise.
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
        .map(|(_, rule, _, file, _, _, _)| (rule.clone(), file.clone()))
        .collect();
    for (_, rule, compiled, file, spans, override_name, bytes) in jobs {
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
        // WI-6b byte path: translate original edit bytes to post offsets.
        if !bytes.is_empty() {
            let all = full_edits.get(file).map(Vec::as_slice).unwrap_or(&[]);
            let post_items = post_starts_for(bytes, all);
            if post_items.len() != bytes.len() {
                let range = spans
                    .first()
                    .map(|(s, e)| format!("{s}..{e}"))
                    .unwrap_or_else(|| "1..1".to_string());
                return Err(crate::postcheck::instantiation_error(rule, file, &range));
            }
            match verify_follow_bytes(&post_text, post_parsed.get(file), &post_items) {
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
            continue;
        }
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
/// WI-6b: when `follow_bytes` (original-coordinate follow edits) are
/// present the check is byte-exact; otherwise the legacy line-span probe
/// runs (kept for callers without byte attribution).
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
    follow_bytes: &[(usize, usize, String)],
) -> Result<(), StructuredError> {
    if follow_bytes.is_empty() {
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
        return match verify_follow_spans(post_text, post_parsed, &follow_post, &new_name) {
            Some(true) | None => Ok(()),
            Some(false) => Err(crate::postcheck::instantiation_error(
                rule, file, range_desc,
            )),
        };
    }
    let all = full_edits.get(file).map(Vec::as_slice).unwrap_or(&[]);
    let post_items = post_starts_for(follow_bytes, all);
    if post_items.len() != follow_bytes.len() {
        return Err(crate::postcheck::instantiation_error(
            rule, file, range_desc,
        ));
    }
    match verify_follow_bytes(post_text, post_parsed, &post_items) {
        Some(true) | None => Ok(()),
        Some(false) => Err(crate::postcheck::instantiation_error(
            rule, file, range_desc,
        )),
    }
}

/// WI-6b: plan.rs entry point for the same-file follow probe (keeps
/// `plan.rs` under the file-size gate). Resolves the `ImplContext`
/// override, fetches recorded `follow` bytes for `(rule, file)`, and runs
/// the byte-exact check. `rename`-labeled impl-def claims stay under WI-1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_definition_follows_recorded(
    rh: &rsmpl_patch::RsmplHunk,
    rule: &str,
    file: &str,
    spans: &[(u32, u32)],
    span_labels: &[Option<String>],
    compiled: &CompiledPattern,
    states: &HashMap<String, FileState>,
    full_edits: &HashMap<String, Vec<TextEdit>>,
    post_text: &str,
    post_parsed: Option<&syn::File>,
    range_desc: &str,
    def_map: &DefFollowMap,
) -> Result<(), StructuredError> {
    let method_new = match &compiled.kind {
        PatternKind::ImplContext { .. } => {
            crate::use_graph_method::method_follow_new_name(rh, compiled)
        }
        _ => None,
    };
    let def_bytes = def_follow_for(def_map, rule, file);
    verify_definition_follows(
        rule,
        file,
        spans,
        span_labels,
        compiled,
        method_new.as_deref(),
        states,
        full_edits,
        post_text,
        post_parsed,
        range_desc,
        &def_bytes,
    )
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

    #[test]
    fn wi6b_sibling_new_on_same_line_does_not_satisfy_byte_probe() {
        // Line-span `contains_ident` would pass (line holds `new` at the
        // sibling call); the byte probe must fail because the follow span
        // itself still names `old`.
        // Post line: `old(1); new(2)` with the follow edit at bytes 0..3.
        let post = "fn f() { old(1); new(2); }\n";
        let parsed = syn::parse_str::<syn::File>(post).ok();
        assert_eq!(
            verify_follow_spans(post, parsed.as_ref(), &[(1, 1)], "new"),
            Some(true),
            "line probe passes (the WI-6b hole)"
        );
        let start = post.find("old(1)").expect("old site");
        let all = vec![TextEdit::new(start, start + 3, "new".to_string())];
        // Simulate a reverted post-image: the edit did NOT land, so the
        // byte at `start` still names `old` — verification fails.
        let reverted_items = vec![(start, "new".to_string())];
        assert_eq!(
            verify_follow_bytes(
                "fn f() { old(1); new(2); }\n",
                parsed.as_ref(),
                &reverted_items
            ),
            Some(false)
        );
        // And the fixed post-image verifies.
        let fixed = "fn f() { new(1); new(2); }\n";
        let parsed_fixed = syn::parse_str::<syn::File>(fixed).ok();
        let _ = &all;
        assert_eq!(
            verify_follow_bytes(fixed, parsed_fixed.as_ref(), &reverted_items),
            Some(true)
        );
    }

    #[test]
    fn wi6b_post_offsets_shift_past_earlier_edits() {
        // Two follow edits; the second's post offset shifts by the first's
        // delta (here both same length, shift 0, plus a growing first edit).
        let orig = "fn f() { old(1); old(2); }\n";
        let first = orig.find("old(1)").expect("first");
        let second = orig.find("old(2)").expect("second");
        let all = vec![
            TextEdit::new(first, first + 3, "new_name(1)".to_string()),
            TextEdit::new(second, second + 3, "new".to_string()),
        ];
        // Apply to get the true post text, then verify offsets land.
        let post = crate::edits::apply_edits(
            orig,
            vec![
                TextEdit::new(first, first + 3, "new_name(1)".to_string()),
                TextEdit::new(second, second + 3, "new".to_string()),
            ],
        )
        .unwrap();
        let parsed = syn::parse_str::<syn::File>(&post).ok();
        let triples = vec![
            (first, first + 3, "new_name(1)".to_string()),
            (second, second + 3, "new".to_string()),
        ];
        let items = post_starts_for(&triples, &all);
        assert_eq!(items.len(), 2);
        assert_eq!(
            verify_follow_bytes(&post, parsed.as_ref(), &items),
            Some(true)
        );
    }

    #[test]
    fn wi6b_same_point_insertions_keep_document_order() {
        // Same-point inserts share (start, end); the stable sort keeps
        // document order so the second insert shifts past the first.
        let triples = vec![(1, 1, "X;".to_string()), (1, 1, "Y;".to_string())];
        let all = vec![
            TextEdit::new(1, 1, "X;".to_string()),
            TextEdit::new(1, 1, "Y;".to_string()),
        ];
        assert_eq!(
            post_starts_for(&triples, &all),
            vec![(1, "X;".to_string()), (3, "Y;".to_string())]
        );
        // Order-dependence: reversed document order reverses the offsets.
        let all_rev = vec![
            TextEdit::new(1, 1, "Y;".to_string()),
            TextEdit::new(1, 1, "X;".to_string()),
        ];
        assert_eq!(
            post_starts_for(&triples, &all_rev),
            vec![(3, "X;".to_string()), (1, "Y;".to_string())]
        );
        // Single-insert control: no shift.
        let single = vec![(1, 1, "X;".to_string())];
        let single_all = vec![TextEdit::new(1, 1, "X;".to_string())];
        assert_eq!(
            post_starts_for(&single, &single_all),
            vec![(1, "X;".to_string())]
        );
    }

    #[test]
    fn wi6b_overlapping_predecessor_counts_conservatively() {
        // Plan time rejects overlaps; the probe still counts an overlapping
        // predecessor's delta (e.start < os < e.end) instead of ignoring it.
        let target = vec![(5, 8, "NN".to_string())];
        let overlap_all = vec![
            TextEdit::new(3, 7, "XXXXXX".to_string()),
            TextEdit::new(5, 8, "NN".to_string()),
        ];
        assert_eq!(
            post_starts_for(&target, &overlap_all),
            vec![(7, "NN".to_string())]
        );
        // Control without the overlap: no shift.
        let alone = vec![TextEdit::new(5, 8, "NN".to_string())];
        assert_eq!(
            post_starts_for(&target, &alone),
            vec![(5, "NN".to_string())]
        );
    }
}
