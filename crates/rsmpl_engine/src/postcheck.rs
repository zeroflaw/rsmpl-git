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
//! - plus-pattern gate (WI-1b, WI-1.2, WI-1.4, WI-1.5): every rewrite
//!   hunk's plus template, compiled to a pattern with the same compiler,
//!   must re-match in the post-image *at the reported spans* (SPEC 24.6)
//!   with SPEC-15.7-equal bindings. Rename kinds verify the new ident by
//!   syn ident equality at overlapping spans (WI-1.4 — exact ident match,
//!   never a substring, so `New` never matches `New2`); `Expr`/`General`/
//!   `Macro` re-run the existing matchers over a synthetic plus-as-minus
//!   hunk (WI-1.5 — no instantiation, no text windows; overlap comes
//!   from matcher byte offsets); code-level `...` tails re-match through
//!   the same matchers — and `ImplContext` verifies `fn {name}`
//!   span-locally.
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
use crate::lines::LineIndex;

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

/// WI-1b/WI-1.2/WI-1.4: check the instantiated plus verifies in the
/// post-image *at the reported spans*.
///
/// `spans` are post-image coordinates here: the caller translates the
/// reported original-source spans via `translate_spans_to_post` (a
/// line-changing rewrite shifts every later site in the file, so
/// original spans miss their output).
///
/// Rename kinds (`Fn`, `ItemRename`, `FieldRename`, `VariantRename`,
/// `UseRename`) verify span-locally by syn ident equality (WI-1.4): at
/// least `matches` reported spans must overlap a definition ident equal
/// to the new name. Occurrence elsewhere in the file never counts, and a
/// longer ident merely containing the needle (`New2`) never counts. The
/// count (not all-spans) formulation covers multi-edit sites (attr-strip
/// deletions, signature-evolution slot edits) where only the ident-bearing
/// spans name the new ident.
///
/// `Expr`/`General`/`Macro` verify by plus-pattern re-match (WI-1.5):
/// the plus side is compiled to a pattern with the same compiler and
/// re-matched in the post-image with the existing matchers; every
/// reported span must overlap a plus match whose bindings are
/// SPEC-15.7-equal to the recorded ones (pairwise when one span per
/// binding holds, per-span otherwise). No instantiation, no text
/// windows — overlap is computed from byte offsets via the post-image
/// `LineIndex`. `ImplContext` counts spans overlapping a
/// `fn {name}` definition (syn ident equality).
///
/// `post_parsed`/`post_index` are the once-per-file post-image parse
/// and line index owned by the caller; `None` (unparseable inputs)
/// downgrades to `reparse-only` (honest, never a new veto).
///
/// Returns `None` when the plus shape cannot be verified (insertions,
/// cuts, pastes, `Raw`, unextractable impl-method names, plus-side
/// compile/match failures) — the caller downgrades to `reparse-only`
/// (honest, never over-claimed). Otherwise `Some(true)` when every
/// reported span verifies, `Some(false)` when any is missing.
/// WI-1.5 parse-once entry point: structural plus verification over a
/// caller-owned post-image parse. Production callers ([`crate::plan`])
/// pass the per-file parse here so N rules over one file share a single
/// `syn::File`; `post_parsed = None` (or `post_index = None`) yields
/// `reparse-only` for the pattern kinds (unparseable inputs keep no
/// veto, never a failure).
#[allow(clippy::too_many_arguments)]
pub(crate) fn plus_instantiated_parsed(
    compiled: &rsmpl_pattern::CompiledPattern,
    rh: &rsmpl_patch::RsmplHunk,
    bindings_list: &[HashMap<String, String>],
    post_text: &str,
    post_parsed: Option<&syn::File>,
    post_index: Option<&LineIndex>,
    spans: &[(u32, u32)],
    matches: usize,
) -> Option<bool> {
    match &compiled.kind {
        PatternKind::Fn { plus_name, .. } => {
            let needle = plus_name.trim();
            match post_parsed {
                Some(parsed) => {
                    let defs = collect_def_idents(parsed);
                    Some(syn_spans_count(&defs, DefKind::Fn, spans, needle) >= matches.max(1))
                }
                None => {
                    let needle = canonical_inst(&format!("fn{plus_name}"));
                    if spans.is_empty() {
                        let post_canon = canonical_inst(post_text);
                        return Some(post_canon.contains(&needle));
                    }
                    Some(spans_count_canon(post_text, spans, &needle) >= matches.max(1))
                }
            }
        }
        PatternKind::ItemRename { plus_name, .. } => match post_parsed {
            Some(parsed) => {
                let defs = collect_def_idents(parsed);
                Some(
                    syn_spans_count(&defs, DefKind::Item, spans, plus_name.trim())
                        >= matches.max(1),
                )
            }
            None => Some(
                spans_count(post_text, spans, plus_name, |h| post_text.contains(h))
                    >= matches.max(1),
            ),
        },
        // WI-9a: member renames carry the same span-local posture as item
        // renames (the new ident must verify at the reported spans).
        PatternKind::FieldRename { new, .. } => match post_parsed {
            Some(parsed) => {
                let defs = collect_def_idents(parsed);
                Some(syn_spans_count(&defs, DefKind::Field, spans, new.trim()) >= matches.max(1))
            }
            None => Some(
                spans_count(post_text, spans, new, |h| post_text.contains(h)) >= matches.max(1),
            ),
        },
        PatternKind::VariantRename { new, .. } => match post_parsed {
            Some(parsed) => {
                let defs = collect_def_idents(parsed);
                Some(syn_spans_count(&defs, DefKind::Variant, spans, new.trim()) >= matches.max(1))
            }
            None => Some(
                spans_count(post_text, spans, new, |h| post_text.contains(h)) >= matches.max(1),
            ),
        },
        PatternKind::UseRename { plus_path, .. } => {
            // Grouped tails (`crate::{new::Deep}`) never contain the full
            // `crate::new` prefix as a substring; the final segment is the
            // renamed anchor the post-image must name.
            let last = plus_path.rsplit("::").next().unwrap_or(plus_path);
            match post_parsed {
                Some(parsed) => {
                    let defs = collect_def_idents(parsed);
                    Some(
                        syn_spans_count(&defs, DefKind::UseSeg, spans, last.trim())
                            >= matches.max(1),
                    )
                }
                None => Some(
                    spans_count(post_text, spans, last, |h| post_text.contains(h))
                        >= matches.max(1),
                ),
            }
        }
        PatternKind::Expr { plus_raw, .. } => {
            let (parsed, index) = match (post_parsed, post_index) {
                (Some(p), Some(i)) => (p, i),
                _ => return None,
            };
            // T126 doctrine: a trailing `;` on an expression pattern
            // denotes statement position (the candidate span excludes
            // it), not expression content. Compiling it into the
            // synthetic minus gloms it onto a binding (observed:
            // `E=(a.wrapping_add(1);)`), so strip one terminator.
            let trimmed = plus_raw.trim_end();
            let line = trimmed.strip_suffix(';').unwrap_or(trimmed).to_string();
            verify_plus_pattern_at_spans(
                rh,
                std::slice::from_ref(&line),
                bindings_list,
                post_text,
                parsed,
                index,
                spans,
                matches,
            )
        }
        PatternKind::General {
            plus_body,
            had_semi,
            ..
        } => {
            let (parsed, index) = match (post_parsed, post_index) {
                (Some(p), Some(i)) => (p, i),
                _ => return None,
            };
            // T126 doctrine (mirrors the Expr arm): with `had_semi` the
            // trailing `;` denotes statement position (the candidate
            // span excludes it), not pattern content. A true `let`
            // statement keeps its terminator; only the terminator
            // artifact is stripped.
            let mut lines = plus_body_lines(plus_body);
            if *had_semi {
                if let Some(last) = lines.last_mut() {
                    let t = last.trim_end();
                    *last = t.strip_suffix(';').unwrap_or(t).to_string();
                }
            }
            verify_plus_pattern_at_spans(
                rh,
                &lines,
                bindings_list,
                post_text,
                parsed,
                index,
                spans,
                matches,
            )
        }
        PatternKind::Macro { plus_raw, .. } => {
            let (parsed, index) = match (post_parsed, post_index) {
                (Some(p), Some(i)) => (p, i),
                _ => return None,
            };
            verify_plus_pattern_at_spans(
                rh,
                std::slice::from_ref(plus_raw),
                bindings_list,
                post_text,
                parsed,
                index,
                spans,
                matches,
            )
        }
        PatternKind::ImplContext { .. } => {
            let name = plus_fn_name(rh)?;
            match post_parsed {
                Some(parsed) => {
                    let defs = collect_def_idents(parsed);
                    Some(syn_spans_count(&defs, DefKind::Fn, spans, name.trim()) >= matches.max(1))
                }
                None => {
                    let needle = normalize_ws(&format!("fn {name}"));
                    if spans.is_empty() {
                        let post_norm = normalize_ws(post_text);
                        return Some(post_norm.contains(&needle));
                    }
                    Some(spans_count_normalized(post_text, spans, &needle) >= matches.max(1))
                }
            }
        }
        PatternKind::Insertion { .. }
        | PatternKind::ItemCapture { .. }
        | PatternKind::Raw { .. } => None,
    }
}

/// Split a `General` plus body into synthetic-hunk lines (one body
/// line per text line, empty lines dropped — the parser skips blanks).
fn plus_body_lines(plus_body: &str) -> Vec<String> {
    plus_body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect()
}

/// WI-1.5 synthetic plus-as-minus hunk: the recorded plus side,
/// compiled with the same compiler that compiled the minus. Metavar
/// declarations (`# expr E`, `# lang=rust`, `# repeat`, `# fallback`,
/// …) are preserved; the discovery posture is forced to file-scope
/// `# allow=many` so every plus occurrence in the post-image is found.
/// `optional`/ranged postures never carry over (a missing plus match
/// must fail, never pass by abstention).
fn plus_pattern_hunk(rh: &rsmpl_patch::RsmplHunk, plus_lines: &[String]) -> rsmpl_patch::RsmplHunk {
    let mut directives = Vec::new();
    for d in &rh.directives {
        match d {
            rsmpl_patch::Directive::Allow(_)
            | rsmpl_patch::Directive::Scope(_)
            | rsmpl_patch::Directive::Optional(_) => {}
            _ => directives.push(d.clone()),
        }
    }
    directives.push(rsmpl_patch::Directive::Allow(Allow::Many));
    directives.push(rsmpl_patch::Directive::Scope(Scope::File));
    let mut lines = Vec::with_capacity(plus_lines.len() * 2);
    for t in plus_lines {
        lines.push(rsmpl_patch::BodyLine {
            prefix: '-',
            text: t.clone(),
        });
        lines.push(rsmpl_patch::BodyLine {
            prefix: '+',
            text: t.clone(),
        });
    }
    let header = rsmpl_patch::HunkHeader {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
        new_elided: true,
        rule: rh.header.rule.clone(),
        raw_tail: String::new(),
    };
    rsmpl_patch::RsmplHunk {
        header,
        directives,
        raw_directives: Vec::new(),
        lines,
    }
}

/// Line-range intersection (1-based inclusive on both sides).
fn spans_overlap(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}

/// SPEC 15.7 binding equality for the postcondition: every variable the
/// fresh plus-match binds must equal the recorded value under the
/// string-aware canonical form (whitespace-insensitive outside string
/// literals, trailing-comma-insensitive). Minus-only variables the plus
/// side drops (e.g. an arg-drop `foo(E, F)` → `bar(E)`) have no fresh
/// counterpart and are ignored — there is nothing to re-match.
///
/// Plus-side concrete idents that diet inference hole-ifies when the
/// plus side is compiled as a pattern (e.g. the `String` in
/// `Box<dyn Fn(T) -> String>`, which was concrete on the plus side but
/// infers as a hole as a minus) are recovered here: a fresh key absent
/// from the recorded map must be self-bound (value equals the name, so
/// the hole matched the literal text). A reverted site binding that
/// hole to anything else fails — exactly the concrete semantics.
fn plus_bindings_equal(
    recorded: &HashMap<String, String>,
    fresh: &HashMap<String, String>,
) -> bool {
    for (k, v) in fresh {
        match recorded.get(k) {
            Some(rv) => {
                if canonical_struct(rv) != canonical_struct(v) {
                    return false;
                }
            }
            None => {
                if canonical_struct(v) != canonical_struct(k) {
                    return false;
                }
            }
        }
    }
    true
}

/// WI-1.5 plus-pattern re-match at the reported spans.
///
/// Compiles the recorded plus side as a pattern (minus = plus) and runs
/// the existing matchers over the post-image in check mode. Each
/// reported span (post-image coordinates) must overlap a plus match;
/// binding equality follows the pairing rule: one span per recorded
/// binding pairs binding `i` with span `i` (a swapped or reverted site
/// fails even when its text occurs at a sibling span); otherwise each
/// span must overlap a plus match equal to some recorded map (the
/// single-map multi-edit case checks every span against the one map).
/// All-empty bindings check overlap only (the concrete template must
/// match at each span). Empty spans (byte-identical self-rewrites, no
/// edits) require at least `matches` plus matches file-wide.
///
/// Any synthetic-compile or matcher failure returns `None` (honest
/// `reparse-only`, never a new veto). No instantiation, no text
/// windows: overlap comes from matcher byte offsets via `post_index`.
/// Line-range pair for one plus-match edit with its binding map.
type SpanBinding<'a> = ((u32, u32), &'a HashMap<String, String>);

#[allow(clippy::too_many_arguments)]
fn verify_plus_pattern_at_spans(
    rh: &rsmpl_patch::RsmplHunk,
    plus_lines: &[String],
    bindings_list: &[HashMap<String, String>],
    post_text: &str,
    post_parsed: &syn::File,
    post_index: &LineIndex,
    spans: &[(u32, u32)],
    matches: usize,
) -> Option<bool> {
    if plus_lines.is_empty() || plus_lines.iter().all(|l| l.trim().is_empty()) {
        return None;
    }
    let synth = plus_pattern_hunk(rh, plus_lines);
    let plus_compiled = rsmpl_pattern::compile(&synth).ok()?;
    let mut throwaway: Vec<TextEdit> = Vec::new();
    let (_matched, fresh_bindings) = match &plus_compiled.kind {
        PatternKind::Expr { .. } => crate::matcher_expr::apply_expr_hunk(
            post_parsed,
            post_text,
            post_index,
            &synth,
            &plus_compiled,
            &mut throwaway,
        )
        .ok()?,
        PatternKind::General { .. } => crate::matcher_general_apply::apply_general_hunk(
            post_parsed,
            post_text,
            post_index,
            &synth,
            &plus_compiled,
            &mut throwaway,
        )
        .ok()?,
        PatternKind::Macro { .. } => crate::matcher_macro::apply_macro_hunk(
            post_parsed,
            post_text,
            post_index,
            &synth,
            &plus_compiled,
            &mut throwaway,
        )
        .ok()?,
        _ => return None,
    };
    if spans.is_empty() {
        return Some(fresh_bindings.len() >= matches.max(1));
    }
    // Pair each plus-match edit span with its binding map. The matchers
    // emit one edit per match in document order, parallel to the
    // bindings vec; a single-map multi-edit run broadcasts its map.
    let pm_spans = crate::edits::edit_spans(post_index, &throwaway);
    let paired: Vec<SpanBinding<'_>> = if pm_spans.len() == fresh_bindings.len() {
        pm_spans.into_iter().zip(fresh_bindings.iter()).collect()
    } else if fresh_bindings.len() == 1 {
        pm_spans
            .into_iter()
            .zip(fresh_bindings.iter().cycle())
            .collect()
    } else {
        return None;
    };
    if bindings_list.iter().all(|m| m.is_empty()) {
        // Concrete template: the plus pattern must match at each span.
        // Diet-inference artifacts (plus-side concrete idents compiled
        // as holes) are admitted only when self-bound — `plus_bindings_equal`
        // against the empty map is exactly the concrete semantics.
        let empty = HashMap::new();
        return Some(spans.iter().all(|sp| {
            paired
                .iter()
                .any(|(pm, fresh)| spans_overlap(*sp, *pm) && plus_bindings_equal(&empty, fresh))
        }));
    }
    let nonempty: Vec<&HashMap<String, String>> =
        bindings_list.iter().filter(|m| !m.is_empty()).collect();
    if nonempty.len() == spans.len() {
        Some(nonempty.iter().zip(spans.iter()).all(|(rec, sp)| {
            paired
                .iter()
                .any(|(pm, fresh)| spans_overlap(*sp, *pm) && plus_bindings_equal(rec, fresh))
        }))
    } else {
        Some(spans.iter().all(|sp| {
            paired.iter().any(|(pm, fresh)| {
                spans_overlap(*sp, *pm)
                    && nonempty.iter().any(|rec| plus_bindings_equal(rec, fresh))
            })
        }))
    }
}

/// WI-1.4 definition-ident classes the syn collector distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefKind {
    /// Free functions and impl/trait methods (`fn` idents).
    Fn,
    /// Item names (struct/enum/trait/union/type/const/static/mod) plus
    /// `impl` self-ty and trait-path idents.
    Item,
    /// Struct field idents.
    Field,
    /// Enum variant idents.
    Variant,
    /// `use`-item path-segment idents.
    UseSeg,
}

/// One collected definition ident with its 1-based inclusive line range.
#[derive(Debug, Clone)]
struct DefIdent {
    kind: DefKind,
    name: String,
    start_line: usize,
    end_line: usize,
}

fn span_lines<S: syn::spanned::Spanned>(node: &S) -> (usize, usize) {
    let s = node.span();
    let start = s.start().line.max(1);
    let end = s.end().line.max(start);
    (start, end)
}

fn ranges_overlap(def: (usize, usize), span: (u32, u32)) -> bool {
    (def.0 as u32) <= span.1 && span.0 <= (def.1 as u32)
}

/// WI-1.4 syn definition collector: every definition ident in the
/// post-image file with exact text and line range. Exact ident equality
/// (never substring) is what removes the `New`/`New2` prefix hazard.
#[derive(Default)]
struct DefCollector {
    defs: Vec<DefIdent>,
}

impl DefCollector {
    /// Record one ident by its own token lines (not the enclosing
    /// item's): a reported span must overlap the ident itself, so an
    /// attr-deletion span above the item never counts as ident-bearing.
    fn push(&mut self, kind: DefKind, name: &syn::Ident) {
        let (start_line, end_line) = span_lines(name);
        self.defs.push(DefIdent {
            kind,
            name: name.to_string(),
            start_line,
            end_line,
        });
    }

    fn push_use(&mut self, tree: &syn::UseTree) {
        use syn::UseTree;
        match tree {
            UseTree::Path(p) => {
                self.push(DefKind::UseSeg, &p.ident);
                self.push_use(&p.tree);
            }
            UseTree::Name(n) => {
                self.push(DefKind::UseSeg, &n.ident);
            }
            UseTree::Rename(r) => {
                self.push(DefKind::UseSeg, &r.rename);
            }
            UseTree::Glob(_) => {}
            UseTree::Group(g) => {
                for t in &g.items {
                    self.push_use(t);
                }
            }
        }
    }

    fn push_path_last(&mut self, path: &syn::Path) {
        if let Some(seg) = path.segments.last() {
            self.push(DefKind::Item, &seg.ident);
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for DefCollector {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.push(DefKind::Fn, &node.sig.ident);
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.push(DefKind::Fn, &node.sig.ident);
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.push(DefKind::Fn, &node.sig.ident);
        syn::visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.push(DefKind::Item, &node.ident);
        for field in &node.fields {
            if let Some(ident) = field.ident.as_ref() {
                self.push(DefKind::Field, ident);
            }
        }
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.push(DefKind::Item, &node.ident);
        for variant in &node.variants {
            self.push(DefKind::Variant, &variant.ident);
        }
        syn::visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_union(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_type(self, node);
    }

    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_static(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.push(DefKind::Item, &node.ident);
        syn::visit::visit_item_mod(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        // Self-ty ident (`impl New {`, `impl ... for New {`).
        if let syn::Type::Path(tp) = node.self_ty.as_ref() {
            self.push_path_last(&tp.path);
        }
        // Trait-side last segment (`impl Labeled for S` renames the
        // trait path's last segment alongside the self-ty).
        if let Some((_, path, _)) = node.trait_.as_ref() {
            self.push_path_last(path);
        }
        syn::visit::visit_item_impl(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        self.push_use(&node.tree);
        syn::visit::visit_item_use(self, node);
    }

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        // WI-1.4: `macro_rules! name` definitions carry the name in
        // `ItemMacro::ident` (the token stream holds only the rules), so
        // the text posture's `new_m`-on-the-line check needs this syn
        // counterpart.
        if let Some(ident) = node.ident.as_ref() {
            self.push(DefKind::Item, ident);
        }
        syn::visit::visit_item_macro(self, node);
    }
}

fn collect_def_idents(parsed: &syn::File) -> Vec<DefIdent> {
    let mut collector = DefCollector::default();
    use syn::visit::Visit;
    collector.visit_file(parsed);
    collector.defs
}

/// WI-1.4 syn span count: reported spans overlapping a collected
/// definition ident exactly equal to `needle`. `spans` are post-image
/// coordinates; empty spans (byte-identical self-rewrite probes, which
/// emit no edits) count whole-file ident-equality matches instead —
/// exact idents, never a substring, so the `New`/`New2` prefix hazard
/// stays closed on the fallback path too.
fn syn_spans_count(defs: &[DefIdent], kind: DefKind, spans: &[(u32, u32)], needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    if spans.is_empty() {
        return defs
            .iter()
            .filter(|d| d.kind == kind && d.name == needle)
            .count();
    }
    spans
        .iter()
        .filter(|sp| {
            defs.iter().any(|d| {
                d.kind == kind
                    && d.name == needle
                    && ranges_overlap((d.start_line, d.end_line), **sp)
            })
        })
        .count()
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

/// WI-1.5 string-aware canonical form (kept for SPEC 15.7 binding
/// equality): whitespace removed *outside* string literals (whitespace
/// inside `"..."` is significant and kept), trailing commas before
/// closers stripped outside strings (`(2,)` → `(2)`). String-aware so
/// `"a b"` never equals `"ab".
fn canonical_struct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut in_str = false;
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
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.replace(",)", ")").replace(",]", "]").replace(",}", "}")
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

    /// Test helper: parse-once entry point over a freshly parsed
    /// post-image (mirrors the production [`crate::plan`] call shape).
    fn plus_instantiated(
        compiled: &rsmpl_pattern::CompiledPattern,
        rh: &rsmpl_patch::RsmplHunk,
        bindings_list: &[HashMap<String, String>],
        post_text: &str,
        spans: &[(u32, u32)],
        matches: usize,
    ) -> Option<bool> {
        let parsed = syn::parse_str::<syn::File>(post_text).ok();
        let index = LineIndex::new(post_text);
        plus_instantiated_parsed(
            compiled,
            rh,
            bindings_list,
            post_text,
            parsed.as_ref(),
            Some(&index),
            spans,
            matches,
        )
    }

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
    fn rename_prefix_does_not_satisfy_span() {
        // WI-1.4: syn ident equality — `New` never verifies inside
        // `New2` (the WI-1.2 substring posture passed this).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- struct Old\n+ struct New\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings: Vec<HashMap<String, String>> = vec![HashMap::new()];
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, "struct New2 {}\n", &[(1, 1)], 1,),
            Some(false)
        );
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, "struct New {}\n", &[(1, 1)], 1,),
            Some(true)
        );
    }

    #[test]
    fn expr_prefix_does_not_satisfy_span() {
        // WI-1.4: token-edge comparison — `x + 2` never verifies inside
        // `x + 22` (the WI-1.2 canonical-substring posture passed this).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# expr E\n- E + 1\n+ E + 2\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "x")]);
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    x + 22;\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
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
    }

    #[test]
    fn swapped_bindings_fail_pairwise() {
        // WI-1.4: one span per match pairs binding `i` with span `i` — a
        // swap that the WI-1.2 twin-share existence posture passed now
        // fails.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E, \"hi\")\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings: Vec<HashMap<String, String>> = vec![
            [("E".to_string(), "a".to_string())].into_iter().collect(),
            [("E".to_string(), "b".to_string())].into_iter().collect(),
        ];
        let swapped = "fn f() {\n    bar(b, \"hi\");\n}\nfn g() {\n    bar(a, \"hi\");\n}\n";
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, swapped, &[(2, 2), (5, 5)], 2),
            Some(false)
        );
        let aligned = "fn f() {\n    bar(a, \"hi\");\n}\nfn g() {\n    bar(b, \"hi\");\n}\n";
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, aligned, &[(2, 2), (5, 5)], 2),
            Some(true)
        );
    }

    #[test]
    fn fragment_prefix_does_not_satisfy_span() {
        // WI-1.4: `...`-fragment edges require token boundaries — `bar(1`
        // never verifies inside `bar(10` (the WI-1.2 substring posture
        // passed this via the T157 comma-stripped fallback).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E, ...)\n+ bar(E, ...)\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "1")]);
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn a() {\n    bar(10, 2);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn a() {\n    bar(1, 2);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
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

    #[test]
    fn widening_plus_rematches_own_output() {
        // WI-1.5: a widening rewrite (`foo(E)` → `foo(foo(E))`)
        // re-matches its own output — the plus pattern matches the
        // post-image node with equal bindings (minus-zero stays
        // rejected by design elsewhere; this gate only checks plus).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ foo(foo(E))\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "x")]);
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    foo(foo(x));\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(true)
        );
        // Reverted span fails even though the minus output is present.
        assert_eq!(
            plus_instantiated(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    foo(x);\n}\n",
                &[(2, 2)],
                1,
            ),
            Some(false)
        );
    }

    #[test]
    fn unparseable_post_downgrades_to_reparse_only() {
        // WI-1.5: without a post-image parse the pattern kinds cannot
        // re-match — honest `reparse-only` (`None`), never a new veto.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E)\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "x")]);
        assert_eq!(
            plus_instantiated_parsed(
                &compiled,
                &rh,
                &bindings,
                "fn f() {\n    bar(x);\n}\n",
                None,
                None,
                &[(2, 2)],
                1,
            ),
            None
        );
    }

    #[test]
    fn single_binding_each_span_checked() {
        // WI-1.5: one recorded map across two spans (multi-edit shape)
        // checks *each* span against that map — a reverted second span
        // fails, although the old any-span existence posture passed it
        // (the first window alone satisfied the binding).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E)\n";
        let (rh, compiled) = compile_first_rh(patch);
        let bindings = bindings1(&[("E", "x")]);
        let both = "fn f() {\n    bar(x);\n}\nfn g() {\n    bar(x);\n}\n";
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, both, &[(2, 2), (5, 5)], 1),
            Some(true)
        );
        let reverted = "fn f() {\n    bar(x);\n}\nfn g() {\n    foo(x);\n}\n";
        assert_eq!(
            plus_instantiated(&compiled, &rh, &bindings, reverted, &[(2, 2), (5, 5)], 1),
            Some(false)
        );
    }
}
