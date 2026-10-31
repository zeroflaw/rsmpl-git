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
//! - plus-instantiation gate (WI-1b, WI-1.2): every rewrite hunk's plus
//!   template, instantiated with its recorded per-site bindings, must
//!   appear in the post-image *at the reported spans* (SPEC 24.6).
//!   Rename kinds verify the new ident on the span lines (WI-1.1);
//!   `Expr`/`General`/`Macro` verify the instantiated plus inside the
//!   span windows — string literals compare as literals, code-level
//!   `...` tails verify as ordered fragments ("absorbed tail present
//!   verbatim") — and `ImplContext` verifies `fn {name}` span-locally.
//!   Occurrence elsewhere in the file never counts. A rewrite that did
//!   not land as authored fails loud as `RSMPL_MATCH_CONFLICT`.
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

/// WI-1b/WI-1.2: check the instantiated plus appears in the post-image
/// *at the reported spans*.
///
/// `spans` are post-image coordinates here: the caller translates the
/// reported original-source spans via `translate_spans_to_post` (a
/// line-changing rewrite shifts every later site in the file, so
/// original spans miss their output).
///
/// Rename kinds (`Fn`, `ItemRename`, `FieldRename`, `VariantRename`,
/// `UseRename`) verify span-locally (WI-1.1): at least `matches`
/// reported spans must carry the new ident on their post-image lines.
/// Occurrence elsewhere in the file never counts. The count (not
/// all-spans) formulation covers multi-edit sites (attr-strip deletions,
/// signature-evolution slot edits) where only the ident-bearing spans name
/// the new ident.
///
/// `Expr`/`General`/`Macro` verify span-locally too (WI-1.2): every
/// recorded binding's instantiated plus must appear inside at least one
/// reported span's adaptive window. String literals compare as literals
/// inside the instantiated text (no abstention); code-level `...` tails
/// verify as ordered fragments (the absorbed tail is present verbatim).
/// `ImplContext` counts spans whose lines name `fn {name}`.
///
/// Returns `None` when the plus shape cannot be verified (insertions,
/// cuts, pastes, `Raw`, unextractable impl-method names) — the caller
/// downgrades to `reparse-only` (honest, never over-claimed). Otherwise
/// `Some(true)` when every site's instantiated template is found at its
/// spans, `Some(false)` when any is missing.
pub(crate) fn plus_instantiated(
    compiled: &rsmpl_pattern::CompiledPattern,
    rh: &rsmpl_patch::RsmplHunk,
    bindings_list: &[HashMap<String, String>],
    post_text: &str,
    spans: &[(u32, u32)],
    matches: usize,
) -> Option<bool> {
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
        PatternKind::Expr { plus_raw, .. } => Some(verify_instantiated_at_spans(
            plus_raw,
            bindings_list,
            post_text,
            spans,
        )),
        PatternKind::General { plus_body, .. } => Some(verify_instantiated_at_spans(
            plus_body,
            bindings_list,
            post_text,
            spans,
        )),
        PatternKind::Macro { plus_raw, .. } => Some(verify_instantiated_at_spans(
            plus_raw,
            bindings_list,
            post_text,
            spans,
        )),
        PatternKind::ImplContext { .. } => {
            let name = plus_fn_name(rh)?;
            let needle = normalize_ws(&format!("fn {name}"));
            if spans.is_empty() {
                let post_norm = normalize_ws(post_text);
                return Some(post_norm.contains(&needle));
            }
            Some(spans_count_normalized(post_text, spans, &needle) >= matches.max(1))
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

/// WI-1.2 span-local check for `Expr`/`General`/`Macro` plus shapes.
///
/// Every non-empty recorded binding's instantiated plus (substitute +
/// backtick-strip, exactly as the engine emits) must appear inside at
/// least one reported span's adaptive window; all-empty bindings check
/// the concrete template. String literals compare as literals inside the
/// instantiated text; code-level `...` tails verify as ordered fragments.
/// Empty spans fall back to the whole post-image (callers without spans
/// keep the old posture, never a new veto).
fn verify_instantiated_at_spans(
    plus_raw: &str,
    bindings_list: &[HashMap<String, String>],
    post_text: &str,
    spans: &[(u32, u32)],
) -> bool {
    if spans.is_empty() {
        let hay = canonical_inst(post_text);
        let check = |m: &HashMap<String, String>| {
            let inst = strip_backticks(&substitute_plus(plus_raw, m));
            fragments_present(&hay, &inst)
        };
        if bindings_list.iter().all(|m| m.is_empty()) {
            return check(&HashMap::new());
        }
        return bindings_list.iter().filter(|m| !m.is_empty()).all(check);
    }
    let windows: Vec<String> = {
        let lines: Vec<&str> = post_text.lines().collect();
        spans
            .iter()
            .map(|(s, e)| {
                let inst_lines = plus_raw.lines().count().max(1);
                span_window_canon(&lines, *s, *e, inst_lines)
            })
            .collect()
    };
    let check = |m: &HashMap<String, String>| {
        let inst = strip_backticks(&substitute_plus(plus_raw, m));
        windows.iter().any(|w| fragments_present(w, &inst))
    };
    if bindings_list.iter().all(|m| m.is_empty()) {
        return check(&HashMap::new());
    }
    bindings_list.iter().filter(|m| !m.is_empty()).all(check)
}

/// Adaptive post-image window for one reported (post-image) span.
///
/// The window starts at `s` and covers `max(span_len, min_lines)` lines,
/// clamped to EOF. `min_lines` is the instantiated template's line count:
/// a wrapping rewrite's emitted output is at most template-shaped plus
/// the original span's own lines, so the site's output is always covered.
fn span_window_canon(lines: &[&str], s: u32, e: u32, min_lines: usize) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let start = (s as usize).saturating_sub(1).min(lines.len() - 1);
    let span_len = (e as usize).saturating_sub(s as usize) + 1;
    let len = span_len.max(min_lines);
    let end = (start + len).min(lines.len());
    canonical_inst(&lines[start..end].join("\n"))
}

/// Ordered-fragment check for an instantiated plus against one
/// canonical window: split the needle on code-level `...` (string-aware,
/// so `"a...b"` never splits), canonicalise each fragment, and require
/// every non-empty fragment present in order. A needle with no concrete
/// fragment is vacuously true (nothing concrete to verify; the reparse
/// gate still ran).
fn fragments_present(window_canon: &str, needle_raw: &str) -> bool {
    let mut rest = window_canon;
    for frag in split_code_ellipsis(needle_raw) {
        let frag = canonical_inst(&frag);
        if frag.is_empty() {
            continue;
        }
        if let Some(pos) = rest.find(&frag) {
            rest = &rest[pos + frag.len()..];
            continue;
        }
        // T157 replay rule: an empty `...` remainder swallows its
        // adjacent comma (`bar(1)` from `bar(E, ...)`), so a fragment
        // with a replay-adjacent comma also matches with one comma
        // removed (suffix first: the `...` sat on that side; prefix
        // covers leading `...`). A lone-`,` fragment is vacuous — both
        // remainders were empty and both commas legitimately vanished.
        let stripped = frag.strip_suffix(',').or_else(|| frag.strip_prefix(','));
        match stripped {
            None => return false,
            Some("") => continue,
            Some(s) => match rest.find(s) {
                Some(pos) => rest = &rest[pos + s.len()..],
                None => return false,
            },
        }
    }
    true
}

/// Split on `...` outside string literals (backslash escapes honoured).
fn split_code_ellipsis(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            cur.push(c);
            if c == '\\' && i + 1 < chars.len() {
                cur.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            cur.push(c);
            i += 1;
            continue;
        }
        if c == '.' && i + 2 < chars.len() && chars[i + 1] == '.' && chars[i + 2] == '.' {
            parts.push(std::mem::take(&mut cur));
            i += 3;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    parts.push(cur);
    parts
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

/// Whitespace-normalized variant for `ImplContext` needles (`fn name`
/// with single spaces).
fn spans_count_normalized(post_text: &str, spans: &[(u32, u32)], needle: &str) -> usize {
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
            normalize_ws(&lines[s..=e].join("\n")).contains(needle)
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

    fn compile_first_rh(patch: &str) -> (rsmpl_patch::RsmplHunk, rsmpl_pattern::CompiledPattern) {
        let parsed = rsmpl_patch::parse_patch(patch).unwrap();
        let rh = match &parsed.files[0].hunks[0] {
            rsmpl_patch::Hunk::Rsmpl(r) => r.clone(),
            _ => panic!("expected rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&rh).unwrap();
        (rh, compiled)
    }

    fn bindings1(pairs: &[(&str, &str)]) -> Vec<HashMap<String, String>> {
        vec![pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()]
    }

    #[test]
    fn string_plus_fires_at_span_and_fails_when_tampered() {
        // WI-1.2 (F1 probe): `+ bar(E, "hi")` verifies structurally at
        // its span — strings compare as literals, no abstention — and an
        // instantiation occurring elsewhere never satisfies the span.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E, \"hi\")\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "x")]);
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    bar(x, \"hi\");\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        // Span reverted; the same instantiation on another line does not
        // count.
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    foo(x);\n}\nfn g() {\n    bar(x, \"hi\");\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn ellipsis_tail_verifies_fragments_at_span() {
        // WI-1.2: `+ bar(E, ...)` verifies as ordered fragments (the
        // absorbed tail is present verbatim), span-locally.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E, ...)\n+ bar(E, ...)\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "x")]);
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    bar(x, y);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        // Reverted span; fragments complete on another line do not count.
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    foo(x, y);\n}\nfn g() {\n    bar(x, y);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn hetero_bindings_each_need_a_span() {
        // WI-1.2: every recorded binding's instantiation must appear at
        // the reported spans; one reverted site fails the run.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E, \"hi\")\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings: Vec<HashMap<String, String>> = vec![
            [("E".to_string(), "x".to_string())].into_iter().collect(),
            [("E".to_string(), "y".to_string())].into_iter().collect(),
        ];
        let post = "fn f() {\n    bar(x, \"hi\");\n}\nfn g() {\n    bar(y, \"hi\");\n}\n";
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, post, &[(2, 2), (5, 5)], 2),
            Some(true)
        );
        let reverted = "fn f() {\n    bar(x, \"hi\");\n}\nfn g() {\n    foo(y);\n}\n";
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, reverted, &[(2, 2), (5, 5)], 2),
            Some(false)
        );
    }

    #[test]
    fn empty_remainder_swallows_comma_in_fragments() {
        // WI-1.2 + T157: `+ bar(E, ...)` with an empty absorbed
        // remainder emits `bar(1)` (comma swallowed); the fragment
        // `bar(1,` still verifies.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E, ...)\n+ bar(E, ...)\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "1")]);
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn a() {\n    bar(1);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn a() {\n    bar(1, 2, 3);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
    }

    #[test]
    fn general_fires_at_span_and_fails_when_tampered() {
        // WI-1.3: General shapes verify span-locally like Expr — the
        // instantiated plus must sit inside a reported span's window.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# expr E\n- E + 1\n+ E + 2\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "x")]);
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    x + 2;\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        // Span reverted; the same instantiation elsewhere never counts.
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    x + 1;\n}\nfn g() {\n    x + 2;\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn macro_fires_at_span_and_fails_when_tampered() {
        // WI-1.3: Macro token-tree shapes verify span-locally; `...`
        // verifies as ordered fragments around the absorbed args.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# macro=token_tree\n- old_mac!(...)\n+ new_mac!(...)\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    new_mac!(1);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    old_mac!(1);\n}\nfn g() {\n    new_mac!(1);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn field_rename_elsewhere_veto() {
        // WI-1.3: the new field ident must sit at a reported span; a
        // same-name field elsewhere never satisfies the hunk.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=r\n  struct S {\n- old: u32\n+ new: u32\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "struct S {\n    new: u32,\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "struct S {\n    old: u32,\n}\nstruct T {\n    new: u32,\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn variant_rename_elsewhere_veto() {
        // WI-1.3: same span-local posture for enum variants.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=r\n  enum E {\n- Old\n+ New\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "enum E {\n    New,\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "enum E {\n    Old,\n}\nstruct New;\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn use_rename_elsewhere_veto() {
        // WI-1.3: the renamed use-path segment must sit at a reported
        // span; the same segment on another line never counts.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=u\n- use crate::old\n+ use crate::new\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "use crate::new::Thing;\n",
                &[(1, 1)],
                1,
            ),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "use crate::old::Thing;\nuse crate::new::Other;\n",
                &[(1, 1)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn impl_context_is_span_local() {
        // WI-1.2: `fn {name}` must appear at a reported span; a
        // same-name method elsewhere never satisfies the hunk.
        let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_impl\n# scope=file\n# allow=many\n# lang=rust\n  impl A {\n      ...\n-     fn old_name(...) {\n+     fn new_name(...) {\n          ...\n      }\n  }\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "impl A {\n    fn new_name(&self) -> i32 {\n        1\n    }\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "impl A {\n    fn old_name(&self) -> i32 {\n        1\n    }\n}\nfn new_name() {}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
    }
}
