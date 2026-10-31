//! WI-2c + WI-1b semantic postconditions (SPEC 24.6).
//!
//! Replaces the tautological `check_untouched` re-derivation (which could
//! only fail on engine non-determinism, never on real user input) with two
//! gates that fail loud on real patches:
//!
//! - sweep change gate: every non-`optional` file-scope `# allow=many`
//!   hunk that matched must have at least one byte-changing definition
//!   edit. A sweep claims every site in scope; a sweep that changes
//!   nothing (e.g. `foo(E) -> foo(E)`) is a loud `RSMPL_MATCH_CONFLICT`.
//! - plus-instantiation gate (WI-1b): every rewrite hunk's plus template,
//!   instantiated with its recorded per-site bindings, must appear in the
//!   post-image. A rewrite that did not land as authored fails loud as
//!   `RSMPL_MATCH_CONFLICT`.
//!
//! Deliberately NOT a "minus must be zero in the post-image" check:
//! widening rewrites legitimately re-match their own output (`let P: T =
//! E` -> `let P: T = E.wrapping_add(1)` still matches `let P: T = E` on
//! the wrapped init; `foo(E)` -> `foo(foo(E))` likewise). Re-matching
//! was probe-verified against the `24-pat-sweep` fixture and rejected
//! for exactly this reason. Ranged singles are skipped (other sites may
//! legitimately remain, and byte-identical fixed-point instantiations
//! per SPEC 18.6 R4 `P @` stay valid matches). `optional` zero-edit
//! hunks are skipped by design (no edits expected).

use std::collections::HashMap;

use rsmpl_patch::{Allow, ErrorCode, Scope, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::edits::TextEdit;

/// Whether this hunk owes the sweep change gate: non-`optional`,
/// file-scope `# allow=many`, countable rewrite kind (insertions, cuts
/// and `Raw` shapes never participate).
pub(crate) fn sweep_hunk_needs_postcheck(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> bool {
    if rh.is_optional() {
        return false;
    }
    if rh.allow() != Allow::Many || rh.scope() != Scope::File {
        return false;
    }
    !matches!(
        &compiled.kind,
        PatternKind::Insertion { .. } | PatternKind::ItemCapture { .. } | PatternKind::Raw { .. }
    )
}

/// True when the hunk's definition edits changed no bytes (every
/// replacement is byte-identical to the original slice it covers).
pub(crate) fn sweep_edits_are_noop(original: &str, def_edits: &[TextEdit]) -> bool {
    if def_edits.is_empty() {
        return false;
    }
    def_edits.iter().all(|e| {
        original
            .get(e.start..e.end)
            .is_some_and(|slice| slice == e.replacement)
    })
}

/// Postcondition failure for a sweep whose edits change nothing
/// (SPEC 24.6, code `RSMPL_MATCH_CONFLICT` so agents reuse the
/// match-conflict repair path).
pub(crate) fn noop_sweep_error(rule: &str, file: &str, range_desc: &str) -> StructuredError {
    StructuredError::new(
        ErrorCode::MatchConflict,
        "postcondition: sweep matched but changed no bytes (plus still matches minus)",
    )
    .with_rule(rule)
    .with_file(file)
    .with_range(range_desc)
    .with_suggestion(
        "change the plus side so the rewrite alters the matched text, or restrict the hunk range",
    )
}

/// WI-1b: true for rewrite kinds that owe the plus-instantiation
/// postcondition (SPEC 24.6). Insertions, item captures (cuts), and `Raw`
/// shapes never participate (reparse-only).
pub(crate) fn is_rewrite_kind(compiled: &rsmpl_pattern::CompiledPattern) -> bool {
    !matches!(
        &compiled.kind,
        PatternKind::Insertion { .. } | PatternKind::ItemCapture { .. } | PatternKind::Raw { .. }
    )
}

/// WI-1b plus-instantiation failure (SPEC 24.6): the instantiated plus
/// did not appear in the post-image. Fails loud as `RSMPL_MATCH_CONFLICT`
/// so agents reuse the match-conflict repair path.
pub(crate) fn instantiation_error(rule: &str, file: &str, range_desc: &str) -> StructuredError {
    StructuredError::new(
        ErrorCode::MatchConflict,
        "postcondition: plus pattern did not re-match post-image at a reported span (rewrite did not land as authored)",
    )
    .with_rule(rule)
    .with_file(file)
    .with_range(range_desc)
    .with_suggestion(
        "read the reported spans in the post-image file; fix the plus side so it matches the intended output, or restrict the hunk range",
    )
}

/// WI-1b: check the instantiated plus appears in the post-image.
///
/// Rename kinds (`Fn`, `ItemRename`, `FieldRename`, `VariantRename`,
/// `UseRename`) verify span-locally (WI-1.1, SPEC 24.6): at least `matches`
/// reported spans must carry the new ident on their post-image lines.
/// Occurrence elsewhere in the file never counts. The count (not
/// all-spans) formulation covers multi-edit sites (attr-strip deletions,
/// signature-evolution slot edits) where only the ident-bearing spans name
/// the new ident.
///
/// Returns `None` when the plus shape cannot be verified by instantiation
/// (code-level `...` replay, string-bearing templates where metavars stay
/// concrete, unextractable impl-method names) — the caller downgrades to
/// `reparse-only` (honest, never over-claimed). Otherwise `Some(true)`
/// when every site's instantiated template is found (whitespace
/// normalized), `Some(false)` when any is missing.
pub(crate) fn plus_instantiated(
    compiled: &rsmpl_pattern::CompiledPattern,
    rh: &rsmpl_patch::RsmplHunk,
    bindings_list: &[HashMap<String, String>],
    post_text: &str,
    spans: &[(u32, u32)],
    matches: usize,
) -> Option<bool> {
    let post_norm = normalize_ws(post_text);
    let post_canon = canonical_inst(post_text);
    match &compiled.kind {
        PatternKind::Fn { plus_name, .. } => {
            let needle = canonical_inst(&format!("fn{plus_name}"));
            if spans.is_empty() {
                let post_canon = canonical_inst(post_text);
                return Some(post_canon.contains(&needle));
            }
            Some(spans_count_canon(post_text, spans, &needle) >= matches.max(1))
        }
        PatternKind::ItemRename { plus_name, .. } => Some(
            spans_count(post_text, spans, plus_name, |h| post_text.contains(h)) >= matches.max(1),
        ),
        // WI-9a: member renames carry the same span-local posture as item
        // renames (the new ident must appear at the reported spans).
        PatternKind::FieldRename { new, .. } => {
            Some(spans_count(post_text, spans, new, |h| post_text.contains(h)) >= matches.max(1))
        }
        PatternKind::VariantRename { new, .. } => {
            Some(spans_count(post_text, spans, new, |h| post_text.contains(h)) >= matches.max(1))
        }
        PatternKind::UseRename { plus_path, .. } => {
            // Grouped tails (`crate::{new::Deep}`) never contain the full
            // `crate::new` prefix as a substring; the final segment is the
            // renamed anchor the post-image must name.
            let last = plus_path.rsplit("::").next().unwrap_or(plus_path);
            Some(spans_count(post_text, spans, last, |h| post_text.contains(h)) >= matches.max(1))
        }
        PatternKind::Expr { plus_raw, .. } => {
            if has_code_ellipsis(plus_raw) || plus_raw.contains('"') {
                return None;
            }
            // Concrete plus (no bindings): the template itself must appear
            // (backticks stripped: `` `E` `` emits concrete `E`).
            if bindings_list.iter().all(|m| m.is_empty()) {
                return Some(post_canon.contains(&canonical_inst(&strip_backticks(plus_raw))));
            }
            for m in bindings_list {
                if m.is_empty() {
                    continue;
                }
                let inst = strip_backticks(&substitute_plus(plus_raw, m));
                let ci = canonical_inst(&inst);
                if !post_canon.contains(&ci) {
                    return Some(false);
                }
            }
            Some(true)
        }
        PatternKind::General { plus_body, .. } => {
            if has_code_ellipsis(plus_body) || plus_body.contains('"') {
                return None;
            }
            if bindings_list.iter().all(|m| m.is_empty()) {
                return Some(post_canon.contains(&canonical_inst(&strip_backticks(plus_body))));
            }
            for m in bindings_list {
                if m.is_empty() {
                    continue;
                }
                let inst = strip_backticks(&substitute_plus(plus_body, m));
                if !post_canon.contains(&canonical_inst(&inst)) {
                    return Some(false);
                }
            }
            Some(true)
        }
        PatternKind::Macro { plus_raw, .. } => {
            if has_code_ellipsis(plus_raw) || plus_raw.contains('"') {
                return None;
            }
            if bindings_list.iter().all(|m| m.is_empty()) {
                return Some(post_canon.contains(&canonical_inst(&strip_backticks(plus_raw))));
            }
            for m in bindings_list {
                if m.is_empty() {
                    continue;
                }
                let inst = strip_backticks(&substitute_plus(plus_raw, m));
                if !post_canon.contains(&canonical_inst(&inst)) {
                    return Some(false);
                }
            }
            Some(true)
        }
        PatternKind::ImplContext { .. } => {
            let name = plus_fn_name(rh)?;
            let needle = normalize_ws(&format!("fn {name}"));
            Some(post_norm.contains(&needle))
        }
        PatternKind::Insertion { .. }
        | PatternKind::ItemCapture { .. }
        | PatternKind::Raw { .. } => None,
    }
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// WI-1.1 span-local check: count of reported spans whose post-image lines
/// contain `needle`. Empty spans fall back to the whole-file `global`
/// closure (callers without spans keep the old posture, never a new veto).
fn spans_count(
    post_text: &str,
    spans: &[(u32, u32)],
    needle: &str,
    global: impl Fn(&str) -> bool,
) -> usize {
    if spans.is_empty() {
        return usize::from(global(needle));
    }
    let lines: Vec<&str> = post_text.lines().collect();
    spans
        .iter()
        .filter(|(s, e)| {
            let s = (*s as usize).saturating_sub(1);
            let e = (*e as usize).saturating_sub(1);
            if lines.is_empty() {
                return false;
            }
            let e = e.min(lines.len() - 1);
            if s >= lines.len() || s > e {
                return false;
            }
            lines[s..=e].join("\n").contains(needle)
        })
        .count()
}

/// Canonical (whitespace-free) variant for `Fn` needles.
fn spans_count_canon(post_text: &str, spans: &[(u32, u32)], needle: &str) -> usize {
    let lines: Vec<&str> = post_text.lines().collect();
    spans
        .iter()
        .filter(|(s, e)| {
            let s = (*s as usize).saturating_sub(1);
            let e = (*e as usize).saturating_sub(1);
            if lines.is_empty() {
                return false;
            }
            let e = e.min(lines.len() - 1);
            if s >= lines.len() || s > e {
                return false;
            }
            canonical_inst(&lines[s..=e].join("\n")).contains(needle)
        })
        .count()
}

/// Canonical form for instantiation comparison: all whitespace removed
/// with trailing commas before closers stripped (`(2,)` → `(2)`), so
/// engine-preserved separators and template-vs-output line wrapping do
/// not fail the check.
fn canonical_inst(s: &str) -> String {
    let n: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    n.replace(",)", ")").replace(",]", "]").replace(",}", "}")
}

/// True when `...` appears outside string literals (code-level replay).
fn has_code_ellipsis(s: &str) -> bool {
    strip_strings(s).contains("...")
}

fn strip_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            if c == '\\' {
                chars.next();
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            continue;
        }
        out.push(c);
    }
    out
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn strip_backticks(s: &str) -> String {
    s.chars().filter(|c| *c != '`').collect()
}

/// Substitute whole-ident metavars in `template` with bound values,
/// skipping double-quoted string literals and backtick-literal regions
/// (metavars stay concrete there, SPEC 15.4 / WI-5b).
fn substitute_plus(template: &str, bindings: &HashMap<String, String>) -> String {
    if bindings.is_empty() {
        return template.to_string();
    }
    let mut keys: Vec<&String> = bindings.keys().collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    let mut out = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    let mut in_tick = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if in_tick {
            out.push(c);
            if c == '`' {
                in_tick = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '`' {
            in_tick = true;
            out.push(c);
            i += 1;
            continue;
        }
        // WI-1b sigiled support: `$E` references the `E` binding (the
        // engine emits the value without the `$` sigil).
        if c == '$' {
            let mut j = i + 1;
            while j < chars.len() && is_ident_char(chars[j]) {
                j += 1;
            }
            if j > i + 1 {
                let ident: String = chars[i + 1..j].iter().collect();
                if let Some(v) = bindings.get(&ident) {
                    out.push_str(v);
                    i = j;
                    continue;
                }
            }
            out.push(c);
            i += 1;
            continue;
        }
        if is_ident_char(c) {
            let mut j = i;
            while j < chars.len() && is_ident_char(chars[j]) {
                j += 1;
            }
            let ident: String = chars[i..j].iter().collect();
            if let Some(v) = bindings.get(&ident) {
                out.push_str(v);
            } else {
                out.push_str(&ident);
            }
            i = j;
            continue;
        }
        out.push(c);
        i += 1;
    }
    // Second pass for any remaining keys that abut non-ident chars
    // already handled above (single pass suffices; kept for clarity).
    let _ = keys;
    out
}

/// Extract the plus-side `fn` name from an impl-context hunk body.
fn plus_fn_name(rh: &rsmpl_patch::RsmplHunk) -> Option<String> {
    for l in &rh.lines {
        if l.prefix != '+' {
            continue;
        }
        let t = l.text.trim().trim_start_matches("pub ").trim_start();
        let rest = t.strip_prefix("fn ")?;
        let name: String = rest.chars().take_while(|c| is_ident_char(*c)).collect();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(start: usize, end: usize, replacement: &str) -> TextEdit {
        TextEdit::new(start, end, replacement.to_string())
    }

    #[test]
    fn noop_when_every_replacement_is_identical() {
        let original = "fn a() {\n    foo(1);\n}\n";
        // `foo(1)` at bytes 13..19 replaced with itself.
        assert!(sweep_edits_are_noop(original, &[edit(13, 19, "foo(1)")]));
    }

    #[test]
    fn change_when_any_replacement_differs() {
        let original = "fn a() {\n    foo(1);\n}\n";
        assert!(!sweep_edits_are_noop(original, &[edit(13, 19, "bar(1)")]));
        assert!(!sweep_edits_are_noop(
            original,
            &[edit(13, 19, "foo(1)"), edit(13, 19, "bar(1)")]
        ));
    }

    #[test]
    fn empty_edits_are_not_noop() {
        assert!(!sweep_edits_are_noop("fn a() {}\n", &[]));
    }

    #[test]
    fn deletions_change_bytes() {
        let original = "fn a() {}\nfn b() {}\n";
        assert!(!sweep_edits_are_noop(original, &[edit(0, 10, "")]));
    }

    #[test]
    fn tampered_post_image_fails_instantiation() {
        // WI-1b: the instantiation check can fail on real input — a
        // plus template that cannot re-appear in the post-image returns
        // `Some(false)` (the planner turns this into a loud
        // `RSMPL_MATCH_CONFLICT`, never a silent `verified`).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- fn foo\n+ fn bar\n";
        let parsed = rsmpl_patch::parse_patch(patch).unwrap();
        let rh = match &parsed.files[0].hunks[0] {
            rsmpl_patch::Hunk::Rsmpl(r) => r.clone(),
            _ => panic!("expected rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&rh).unwrap();
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        // Clean post-image: `fn bar` present.
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, "fn bar() {}\n", &[(1, 1)], 1),
            Some(true)
        );
        // Tampered post-image (rewrite reverted): `fn bar` missing.
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, "fn foo() {}\n", &[(1, 1)], 1),
            Some(false)
        );
    }

    #[test]
    fn rename_elsewhere_does_not_satisfy_span() {
        // WI-1.1: a pre-existing same-name item elsewhere never satisfies
        // a rename whose own span still carries the old name.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- struct Old\n+ struct New\n";
        let parsed = rsmpl_patch::parse_patch(patch).unwrap();
        let rh = match &parsed.files[0].hunks[0] {
            rsmpl_patch::Hunk::Rsmpl(r) => r.clone(),
            _ => panic!("expected rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&rh).unwrap();
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        // `New` exists on line 2, but the reported span (line 1) was not
        // rewritten: must fail.
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "struct Old {}\nstruct New {}\n",
                &[(1, 1)],
                1,
            ),
            Some(false)
        );
        // Span line carries the new name: passes.
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "struct New {}\nstruct New {}\n",
                &[(1, 1)],
                1,
            ),
            Some(true)
        );
    }

    #[test]
    fn rename_multi_span_counts_matches() {
        // WI-1.1 count formulation: an attr-strip rename emits a deletion
        // span plus the ident span; one ident-bearing span satisfies one
        // match.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- struct Old\n+ struct New\n";
        let parsed = rsmpl_patch::parse_patch(patch).unwrap();
        let rh = match &parsed.files[0].hunks[0] {
            rsmpl_patch::Hunk::Rsmpl(r) => r.clone(),
            _ => panic!("expected rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&rh).unwrap();
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        // Two spans (attr deletion + ident), only the second names `New`.
        let post = "/// doc\nstruct New {}\n";
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, post, &[(1, 1), (2, 2)], 1),
            Some(true)
        );
        // Two matches need two ident-bearing spans.
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, post, &[(1, 1), (2, 2)], 2),
            Some(false)
        );
    }
}
