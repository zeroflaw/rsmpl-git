//! Whole-section detector chain (WI-4 prerequisite / WI-10 split).
//!
//! Split from `semantic/mod.rs` under the 2000-line gate: pure move, no
//! behavior change. Owns the detector registration — the ordered
//! pair-sweep chain shared by the whole-section path and the per-hunk
//! fallback (`crate::compress_balanced_pair_body` delegates to
//! [`run_pair_chain`]) — plus `compress_section` and its per-hunk
//! fallback. `mod.rs` keeps the facade (`infer_semantic*`), the item
//! helpers, and the unit tests.

use super::prov::{commit_prov, push_section_prov};
use super::HunkProvenance;

/// WI-7a: swap a whole-section detector's collapsed `diff --git` header
/// for the original rename prelude (byte-exact: `similarity index`,
/// `rename from/to`, modes, `---`/`+++`). The detector body (from the
/// first `@@`) is kept verbatim; `None` when the emission holds no
/// hunk (defensive — detectors always emit one).
fn reheader_rename(semantic: &str, prelude: &str) -> Option<String> {
    let at = semantic.find("@@")?;
    let mut out = prelude.to_string();
    out.push_str(&semantic[at..]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// Ordered pair-sweep chain, tightest first (existing order,
/// unchanged; WI-4a appends one shape at the end, WI-4b one more, WI-4c one more, WI-5 one more): slot → sig-evolution → trait-method → method-to-fn → method-to-method →
/// same-literal → `&`-strip → qualified-call → turbofish →
/// macro-rename → use-prefix → bare call → call-perm →
/// call-arg-add →
/// call-literal → call-arg-template → method-arg-template →
/// general-expr → call-peel → let-type-add → type-sweep → let-template → untyped-let + pat-swap → sequence → repeat-fold → partial-strip → single-block → literal-frame. Returns the winning semantic section
/// plus its WI-2 detector name. No T375 byte guard inside — callers
/// keep their existing guards so reservation/truncation semantics
/// stay byte-identical to the pre-split `mod.rs`.
pub(crate) fn run_pair_chain(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    // WI-7b: `macro_rules!` definition + invocation rename (one def plus
    // N uses, hetero by construction) — before the slot sweep, which owns
    // homogeneous header classes and never claims a `macro_rules!` line.
    if let Some(combined) =
        super::try_macro_rules_rename(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((combined, "macro_rules_rename"));
    }
    if let Some(slot) =
        super::try_fn_slot_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((slot, "fn_slot_sweep"));
    }
    // WI-4c: signature evolution (name plus any vis/qual/generics/
    // params/return/where delta, bodies untouched) — immediately after
    // the rename-only slot sweep whose zero-delta classes it
    // generalizes (disjoint by construction: slot keeps rename-only,
    // this requires an evolving non-name slot).
    if let Some(evo) =
        super::try_sig_evolution_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((evo, "sig_evolution"));
    }
    // WI-4b: trait+impl method rename (mixed `;` decl / `{` def
    // classes the slot and sig-evolution sweeps decline by
    // construction) — immediately after sig-evolution, before the
    // method sweeps (those own same-name call shapes, never header
    // pairs).
    if let Some(tm) =
        super::try_trait_method_rename(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((tm, "trait_method_rename"));
    }
    // WI-9b: field/variant member rename (one definition pair plus
    // use-site lines sharing the ident delta, follow emission with a
    // scratch-apply line-equality probe). Disjoint from the trait
    // detector (those own `;`/`{` fn headers; this owns `:`/bare-ident
    // pairs) — immediately after it, before the method sweeps.
    if let Some(member) =
        super::try_member_rename(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((member, "member_rename"));
    }
    if let Some(m2f) =
        super::try_method_to_fn_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((m2f, "method_to_fn"));
    }
    if let Some(m2m) =
        super::try_method_to_method_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((m2m, "method_to_method"));
    }
    if let Some(same) =
        super::try_method_same_literal_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((same, "method_same_literal"));
    }
    if let Some(strip) =
        super::try_ref_method_strip_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((strip, "ref_strip"));
    }
    // WI-3: qualified-path, turbofish, and macro-rename sweeps — after
    // the method sweeps, before the bare-call sweep (tightest first).
    if let Some(qual) =
        super::try_qualified_call_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((qual, "qualified_call"));
    }
    if let Some(turbo) =
        super::try_turbofish_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((turbo, "turbofish"));
    }
    if let Some(mac) =
        super::try_macro_rename_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((mac, "macro_rename"));
    }
    if let Some(uses) =
        super::try_use_prefix_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((uses, "use_prefix"));
    }
    // N >= 2 bare-call rewrites (callee-only delta), use-path prefix
    // renames above included; fall through to the trailing-arg sweeps
    // otherwise.
    if let Some(sweep) = super::try_call_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((sweep, "call_sweep"));
    }
    // WI-6 slice 2: whole-arg permutation sub-line sweep (same-callee
    // same-arity calls with one uniform arg permutation across all
    // subterms; the sub-line form of the arg-template permute promise).
    // After the callee-only sweep (those own token-equal-args classes;
    // this owns permuted-args classes), before the trailing-arg
    // sweeps. Pure whole-line classes decline here (they belong to
    // `call_arg_template` below).
    if let Some(perm) =
        super::try_call_perm_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((perm, "call_arg_template"));
    }
    // T447: same-callee trailing-arg addition (homo concrete tails
    // compress; hetero tails like 09d3446's `symlink` vs `false`
    // veto via minus-arity totality).
    if let Some(add) =
        super::try_call_arg_add_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((add, "call_arg_add"));
    }
    // T448: same-callee same-arity trailing-literal delta (homo concrete
    // trailing pair, hetero leading bound as E-vars; different-trailing
    // same-arity sites never veto via tuple-keyed totality).
    if let Some(lit) =
        super::try_call_literal_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((lit, "call_literal"));
    }
    // WI-5: hetero call/method arg templates (wrap/transform migrations
    // the engine applies but no detector raised) — after the concrete
    // literal sweeps (those own uniform-concrete classes; this owns
    // hole/wrapper classes), before the peel sweep.
    if let Some(arg) =
        super::try_call_arg_template_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((arg, "call_arg_template"));
    }
    if let Some(marg) =
        super::try_method_arg_template_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((marg, "method_arg_template"));
    }
    // WI-6: general-expression sweep (bare-expression anti-unification
    // with at most one hole) — after the call/method arg templates
    // (those own call shapes; this declines them), before the peel
    // sweep (which owns array-head shapes, never bare expressions).
    if let Some(general) =
        super::try_general_expr_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((general, "general_expr"));
    }
    // T441: hetero-arg array-head-peel call migration
    // (`f(D, &[H, M, T…])` → `g(D, M, &[T…])` per-tail-arity groups);
    // all-identical classes stay for the tighter verbatim sweep below.
    if let Some(peel) =
        super::try_call_peel_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((peel, "call_peel"));
    }
    // WI-4a: let type-addition (`let P = E` → `let P: T = E`, P/E
    // hetero across sites, uniform concrete T) — after the call-shape
    // chain (those ignore the `let` wrapper and cannot express an
    // ascription delta), before the verbatim stage in the caller.
    if let Some(let_sweep) =
        super::try_let_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((let_sweep, "let_type_add"));
    }
    // WI-4b: type rewrite (`Vec<T>` → `Box<[T]>`, T hetero across
    // sites) — after the let-shape chain (those own whole-statement
    // templates with init/pattern holes; this owns sub-line type
    // templates), before the verbatim stage in the caller.
    if let Some(type_sweep) =
        super::try_type_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((type_sweep, "type_sweep"));
    }
    // WI-5 slice 1: joint let template (typed→typed `let` pairs with
    // per-site preserved pat/ty plus one uniform init delta) — after
    // the type sweep (that owns per-site-preserved-init classes; this
    // owns per-site init deltas), before the verbatim stage. Disjoint
    // from both let slices by construction.
    if let Some(joint) =
        super::try_let_template_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((joint, "let_template"));
    }
    // WI-8 slice 1: untyped joint let template (same as above without
    // ascriptions — hetero `P0`, uniform or `E0`-hole init incl.
    // const-block/`_` inits via the shape parity gate) plus the
    // tuple-position swap (`let (P0, P1) = E0;` →
    // `let (P1, P0) = E0;`). After the typed template (those own
    // ascribed classes; this declines any ascribed side), before the
    // sequence sweep (which owns multi-line windows, never singles).
    if let Some(untyped) =
        super::try_untyped_let_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((untyped, "let_template"));
    }
    if let Some(swap) =
        super::try_pat_swap_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((swap, "let_template"));
    }
    // WI-5c: repeated-block `let` sequences (N ≥ 2 blocks × 2–4 stmts,
    // hetero positions ride holes) — after the single-statement let
    // sweeps (those own whole-class templates; this owns multi-line
    // windows the singles cannot express), before the fold (which owns
    // call-spine strips, never statement windows).
    if let Some(seq) =
        super::try_sequence_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((seq, "sequence"));
    }
    // WI-1c: nested same-callee full-strip fold (`Box::new(Box::new(x))`
    // → `x`, `# repeat`) — after the type sweep (those own whole-statement
    // templates; this owns call-spine strips), before the verbatim stage
    // in the caller. No earlier detector claims a call→non-call pair.
    if let Some(fold) =
        super::try_repeat_fold_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((fold, "repeat_fold"));
    }
    // WI-1d: uniform one-layer partial strip (`C(C(x))` → `C(x)`,
    // plain hunk, no `# repeat` — a repeat emission would over-strip
    // the kept layers). After the fold (full strips stay folded;
    // disjoint by construction: the fold declines partial pairs and
    // the strip declines full strips), before the verbatim stage.
    if let Some(strip) =
        super::try_partial_strip_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((strip, "partial_strip"));
    }
    // WI-5d: single-block hetero `let` run (one contiguous K = 2–4
    // window, within-pair abstraction) — every earlier detector keeps
    // its classes (this owns only runs declined by all of them).
    if let Some(single) =
        super::try_single_block_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((single, "single_block"));
    }
    // WI-7: lone-literal sweep (`"old"` → `"new"`, `10` → `20`,
    // `false` → `true` inside hetero frames) — last: the bare-literal
    // template is the broadest still-semantic sweep, so tighter
    // detectors (calls, lets, windows) keep their classes first; the
    // verbatim stage in the caller owns all-identical lines (T422,
    // declined inside the detector).
    if let Some(frame) =
        super::try_literal_frame_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((frame, "literal_frame"));
    }
    None
}

/// WI-8 pair partition: a whole balanced section with N ≥ 2 pairs whose
/// pair chain declined compresses pair by pair when every pair is a
/// single-pair item rename (including the plain-`fn` arm, whose
/// `{`/no-return/no-`where` gate applies per pair). All-or-nothing:
/// one unclaimable pair vetoes the whole section (a mixed hunk stays
/// ordinary whole, never half-semantic). Duplicate minus lines veto
/// (identical pairs belong to the verbatim sweep); duplicate emitted
/// minus patterns veto too (terse `fn` shapes drop params, so distinct
/// raw lines can share one pattern and would overlap-match). Each pair
/// also clears pre-image totality (`template_pair_holds` at n = 1, the
/// `slot_holds` posture lifted to single pairs): an untouched twin
/// anywhere in the file vetoes, `None` pre-image keeps the
/// section-visible legacy allow. Emission is one file
/// section with one hunk per pair (per-pair covering ranges from that
/// pair's changed line, positional `rN` rules — the shared counter is
/// already positional per WI-1e diet; distinct deltas never group so
/// `group` stays `""`). Section-level T375 lives with the caller (same
/// shape as the `run_pair_chain` block, rename prelude included).
/// Detector `"item_rename"` so cross-file same-delta grouping keeps
/// working with no `group.rs` change.
pub(crate) fn try_partition_section(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if minuses.len() < 2 || minuses.len() != pluses.len() {
        return None;
    }
    // Pair-to-site mapping for the per-pair covering ranges below.
    if changed.len() != minuses.len() {
        return None;
    }
    // Identical pairs belong to the verbatim sweep (and their patterns
    // would overlap-match under `# allow=many`).
    let mut ordered: Vec<&&str> = minuses.iter().collect();
    ordered.sort();
    ordered.dedup();
    if ordered.len() != minuses.len() {
        return None;
    }
    let checkpoint = used_rules.len();
    let mut bodies: Vec<String> = Vec::new();
    for i in 0..minuses.len() {
        match super::try_item_rename(
            minuses[i],
            pluses[i],
            path,
            file_old,
            &changed[i..i + 1],
            used_rules,
        ) {
            Some(full) => {
                let Some((_, body)) = full.split_once('\n') else {
                    used_rules.truncate(checkpoint);
                    return None;
                };
                bodies.push(body.to_string());
            }
            None => {
                used_rules.truncate(checkpoint);
                return None;
            }
        }
    }
    // One minus pattern per pair; identical emitted patterns would
    // overlap-match under `# allow=many` (terse `fn` shapes drop params,
    // so distinct raw lines can share one pattern — those classes stay
    // with the slot/verbatim sweeps).
    let mut pats: Vec<(&str, &str)> = Vec::new();
    for body in &bodies {
        let mut minus = None;
        let mut plus = None;
        for line in body.lines() {
            if let Some(pat) = line.strip_prefix("- ") {
                if minus.is_none() {
                    minus = Some(pat);
                }
            } else if let Some(pat) = line.strip_prefix("+ ") {
                if plus.is_none() {
                    plus = Some(pat);
                }
            }
        }
        let (Some(minus), Some(plus)) = (minus, plus) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        pats.push((minus, plus));
    }
    let mut by_pat: Vec<&&str> = pats.iter().map(|(m, _)| m).collect();
    by_pat.sort();
    by_pat.dedup();
    if by_pat.len() != pats.len() {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Pre-image totality per pair (the `slot_holds` posture lifted to
    // single pairs): each minus pattern must match exactly its own site.
    // An untouched twin anywhere in the file vetoes; `None` pre-image
    // keeps the section-visible legacy allow (provenance marks it).
    for &(minus, plus) in &pats {
        if !crate::totality::template_pair_holds(path, file_old, minus, plus, 1) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    let mut out = crate::git_header(path);
    out.push('\n');
    for body in &bodies {
        out.push_str(body);
    }
    Some((out, "item_rename"))
}

pub(crate) fn compress_section(
    section: &str,
    used_rules: &mut Vec<String>,
    pre: &std::collections::HashMap<String, String>,
    prov: &mut Vec<HunkProvenance>,
) -> Option<String> {
    let header = section.lines().next()?;
    // T385: CRLF content stays ordinary passthrough — detectors strip
    // `\r` via `str::lines` and would emit hunks that cannot match
    // the worktree file.
    if !header.starts_with("diff --git ") || section.contains('\r') {
        return None;
    }
    // WI-7a: rename sections compress their content hunks instead of
    // staying ordinary passthrough. The engine seeds a rename section's
    // new-side entry from the OLD file's content, so semantic hunks
    // citing old-side coordinates apply transparently; the lowering
    // rebuilds `rename from/to` around the new content. The original
    // rename prelude (`similarity index`, `rename from/to`, modes,
    // `---`/`+++`) is preserved byte-exact — detectors rebuild only
    // `diff --git`, which would drop it. Pre-image totality reads the
    // old path (the new path does not exist at base; `rsmpl_diff`
    // maps it, with section-visible fallback here).
    let rename_new: Option<String> = if super::is_rename_header(header) {
        let rest = header.strip_prefix("diff --git ")?;
        let parts = rsmpl_patch::split_git_paths(rest)?;
        if parts.len() != 2 {
            return None;
        }
        let _old = parts[0].strip_prefix("a/").unwrap_or(&parts[0]);
        let new = parts[1].strip_prefix("b/").unwrap_or(&parts[1]);
        if new == "/dev/null" || new == "dev/null" {
            return None;
        }
        if !new.ends_with(".rs") {
            return None;
        }
        // Headers-only renames (no hunks) stay passthrough below via
        // the `saw_hunk` gate; nothing else is rename-specific here —
        // the shared machinery runs on the original section (whose
        // prelude splitters already preserve) with `path` = new side.
        // Pre-image totality reads `pre[new]` (mapped from the old path
        // by `rsmpl_diff`; section-visible fallback otherwise).
        Some(new.to_string())
    } else {
        None
    };
    let path_owned;
    let path = if let Some(n) = &rename_new {
        path_owned = n.clone();
        &path_owned
    } else {
        let p = crate::b_side_path(header.strip_prefix("diff --git ")?)?;
        if !p.ends_with(".rs") {
            return None;
        }
        path_owned = p;
        &path_owned
    };
    // Rename prelude (everything before the first `@@`): preserved
    // byte-exact around winning semantic bodies.
    let rename_prelude: Option<String> = rename_new.as_ref().and_then(|_| {
        let at = section.find("@@")?;
        Some(section[..at].to_string())
    });
    // T464: whole-file item-deletion sweep (minus-only deletion sections
    // compress to one many-delete hunk + ordinary leftovers). Renames
    // always have both sides real, so the delete sweep cannot fire —
    // skip it on the rename path.
    let delete_before = used_rules.len();
    if rename_new.is_none() {
        if let Some(deleted) = super::deletes::try_delete_sweep(section, used_rules) {
            push_section_prov(
                prov,
                &deleted,
                section,
                path,
                "delete",
                &crate::totality::changed_old_lines(section),
                pre.get(path.as_str()).map(String::as_str),
                delete_before,
                used_rules,
            );
            return Some(deleted);
        }
    }
    // T403 totality input: the pre-image text for this path, if the
    // caller supplied it (None = section-visible evidence only).
    let file_old = pre.get(path.as_str()).map(String::as_str);
    // Group changed lines per `@@` hunk: unified diff lists each
    // hunk's `-` block before its `+` block, so global order pairing
    // is only sound when every hunk is a pure modification.
    let mut minuses: Vec<&str> = Vec::new();
    let mut pluses: Vec<&str> = Vec::new();
    let mut hunk_minus = 0usize;
    let mut hunk_plus = 0usize;
    let mut balanced = true;
    let mut in_hunk = false;
    let mut saw_hunk = false;
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            if in_hunk && hunk_minus != hunk_plus {
                balanced = false;
            }
            in_hunk = true;
            saw_hunk = true;
            hunk_minus = 0;
            hunk_plus = 0;
            continue;
        }
        if !in_hunk {
            balanced &= !(line.starts_with("old mode ") || line.starts_with("new mode ")); // T401
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            minuses.push(&line[1..]);
            hunk_minus += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            pluses.push(&line[1..]);
            hunk_plus += 1;
        }
    }
    if in_hunk && hunk_minus != hunk_plus {
        balanced = false;
    }
    // Whole-section attempt first (preserves cross-hunk sweeps): it
    // needs every hunk balanced and equal non-zero global counts, so
    // global order pairing stays sound. Gate failures skip straight
    // to the per-hunk fallback (T349).
    let whole = balanced && saw_hunk && minuses.len() == pluses.len() && !minuses.is_empty();
    // T375: every whole-section candidate clears the token-win guard
    // (byte-smaller than the ordinary section, same prelude either
    // way); a losing shape truncates its reserved rule name and falls
    // through to the per-hunk retry so one losing shape never vetoes
    // winning runs in other hunks.
    let checkpoint = used_rules.len();
    // T411: old-side changed lines for the line-aware opaque veto.
    let changed = crate::totality::changed_old_lines(section);
    if whole && minuses.len() == 1 {
        // Single pair: fn/item rename only (a lone call rewrite has
        // no token win over the ordinary hunk; a lone heterogeneous
        // fn header likewise costs more as a slot hunk than ordinary).
        if let Some(rename) =
            super::try_item_rename(minuses[0], pluses[0], path, file_old, &changed, used_rules)
        {
            // WI-7a: rename sections re-emit under the preserved
            // prelude; the T375 guard compares the final bytes.
            if let Some(prelude) = &rename_prelude {
                if let Some(final_text) = reheader_rename(&rename, prelude) {
                    if final_text.len() < section.len() {
                        push_section_prov(
                            prov,
                            &final_text,
                            section,
                            path,
                            "item_rename",
                            &changed,
                            file_old,
                            checkpoint,
                            used_rules,
                        );
                        return Some(final_text);
                    }
                    used_rules.truncate(checkpoint);
                } else {
                    used_rules.truncate(checkpoint);
                }
            } else if rename.len() < section.len() {
                push_section_prov(
                    prov,
                    &rename,
                    section,
                    path,
                    "item_rename",
                    &changed,
                    file_old,
                    checkpoint,
                    used_rules,
                );
                return Some(rename);
            } else {
                used_rules.truncate(checkpoint);
            }
        }
        return compress_section_per_hunk(section, path, file_old, used_rules, prov);
    }
    if !whole {
        // T441: hetero-arg peel cross-hunk sweep before verbatim
        // (claims hetero classes; pure-identical classes stay for the
        // tighter verbatim sweep, which ignores the single-pair peel
        // bodies — pinned by `peel_body_verbatim_still_fires`). T449:
        // same-callee trailing-literal cross-hunk sweep between peel
        // and verbatim (claims hetero literal classes split across
        // singleton hunks, e.g. 7025a14's spaced `let` prize pair;
        // all-identical classes stay for verbatim, T422 precedent).
        // Each rewrite composes on the previous one and is kept even
        // when later stages find nothing (returning `None` here
        // would discard it); downstream stages group the single-pair
        // bodies at n=1 (skipped) and pass them through unchanged.
        let mut rewritten: Option<String> = None;
        // WI-2: the section drivers below record every rule they reserve
        // into `fired` with its exact detector (verbatim also records its
        // leftover-retry rules with theirs); commit keeps only the rules
        // present in the final composed text.
        let mut fired: Vec<(String, &'static str)> = Vec::new();
        if let Some(peeled) =
            super::peel::try_call_peel_section(section, path, used_rules, file_old, &mut fired)
        {
            rewritten = Some(peeled);
        }
        let after_peel = rewritten.as_deref().unwrap_or(section);
        if let Some(lit) = super::literal::try_call_literal_section(
            after_peel, path, used_rules, file_old, &mut fired,
        ) {
            rewritten = Some(lit);
        }
        let after_lit = rewritten.as_deref().unwrap_or(section);
        // WI-9a: same-name concrete macro interiors (whole-line pairs and
        // stranded joins) before verbatim (which vetoes every macro, so no
        // double-claim); emitted single-pattern hunks group at n=1 below.
        if let Some(interior) =
            super::try_macro_interior_section(after_lit, path, used_rules, file_old, &mut fired)
        {
            rewritten = Some(interior);
        }
        // WI-7a: enclosing-macro interiors (changed lines inside one
        // shared macro invocation) over the interior output: already
        // semantic (`rule=`) hunks are never re-harvested, so claimed
        // pairs cannot double-fire.
        let after_interior = rewritten.as_deref().unwrap_or(section);
        if let Some(enclosing) = super::macro_enclosing::try_macro_enclosing_section(
            after_interior,
            path,
            used_rules,
            file_old,
            &mut fired,
        ) {
            rewritten = Some(enclosing);
        }
        let after_mac = rewritten.as_deref().unwrap_or(section);
        if let Some(verbatim) =
            super::verbatim::try_verbatim_sweep(after_mac, path, used_rules, file_old, &mut fired)
        {
            commit_prov(prov, &verbatim, section, path, &fired, &changed, file_old);
            return Some(verbatim);
        }
        if let Some(r) = rewritten {
            commit_prov(prov, &r, section, path, &fired, &changed, file_old);
            return Some(r);
        }
        // T403: non-whole sections try the cross-hunk verbatim sweep
        // before per-hunk (whole-section detectors need balance) —
        // already attempted above on the unmodified section.
        return compress_section_per_hunk(section, path, file_old, used_rules, prov);
    }
    // N >= 2 whole-section pair sweeps in chain order; a losing shape
    // truncates its reserved rule and falls through to verbatim, so
    // one losing shape never vetoes the rest.
    if let Some((sweep, detector)) =
        run_pair_chain(&minuses, &pluses, path, file_old, &changed, used_rules)
    {
        // WI-7a: rename sections re-emit under the preserved prelude;
        // the T375 guard compares the final bytes.
        if let Some(prelude) = &rename_prelude {
            if let Some(final_text) = reheader_rename(&sweep, prelude) {
                if final_text.len() < section.len() {
                    push_section_prov(
                        prov,
                        &final_text,
                        section,
                        path,
                        detector,
                        &changed,
                        file_old,
                        checkpoint,
                        used_rules,
                    );
                    return Some(final_text);
                }
            }
            used_rules.truncate(checkpoint);
        } else if sweep.len() < section.len() {
            push_section_prov(
                prov, &sweep, section, path, detector, &changed, file_old, checkpoint, used_rules,
            );
            return Some(sweep);
        } else {
            used_rules.truncate(checkpoint);
        }
    }
    // WI-8 pair partition: the chain declines hetero multi-rename
    // classes (distinct deltas share no template); when every pair is
    // a single-pair item rename the section still compresses pair by
    // pair (one hunk per pair, positional rules). Verbatim owns
    // identical pairs (the duplicate-minus veto), so at most one of
    // the two fires; the per-hunk fallback below keeps every other
    // class. Same T375 + provenance shape as the chain block.
    let part_checkpoint = used_rules.len();
    if let Some((part, part_detector)) =
        try_partition_section(&minuses, &pluses, path, file_old, &changed, used_rules)
    {
        // WI-7a: rename sections re-emit under the preserved prelude;
        // the T375 guard compares the final bytes.
        if let Some(prelude) = &rename_prelude {
            if let Some(final_text) = reheader_rename(&part, prelude) {
                if final_text.len() < section.len() {
                    push_section_prov(
                        prov,
                        &final_text,
                        section,
                        path,
                        part_detector,
                        &changed,
                        file_old,
                        part_checkpoint,
                        used_rules,
                    );
                    return Some(final_text);
                }
            }
            used_rules.truncate(part_checkpoint);
        } else if part.len() < section.len() {
            push_section_prov(
                prov,
                &part,
                section,
                path,
                part_detector,
                &changed,
                file_old,
                part_checkpoint,
                used_rules,
            );
            return Some(part);
        } else {
            used_rules.truncate(part_checkpoint);
        }
    }
    let mut verb_fired: Vec<(String, &'static str)> = Vec::new();
    // WI-9a: same-name concrete macro interiors before verbatim (which
    // vetoes every macro, so no double-claim).
    if let Some(interior) =
        super::try_macro_interior_section(section, path, used_rules, file_old, &mut verb_fired)
    {
        // WI-7a: enclosing-macro interiors over the interior output
        // (already-semantic hunks are never re-harvested).
        if let Some(enclosing) = super::macro_enclosing::try_macro_enclosing_section(
            &interior,
            path,
            used_rules,
            file_old,
            &mut verb_fired,
        ) {
            commit_prov(
                prov,
                &enclosing,
                section,
                path,
                &verb_fired,
                &changed,
                file_old,
            );
            return Some(enclosing);
        }
        commit_prov(
            prov,
            &interior,
            section,
            path,
            &verb_fired,
            &changed,
            file_old,
        );
        return Some(interior);
    }
    // WI-7a: the interior sweep declined the section whole — the
    // enclosing sweep still sees the original ordinary hunks.
    if let Some(enclosing) = super::macro_enclosing::try_macro_enclosing_section(
        section,
        path,
        used_rules,
        file_old,
        &mut verb_fired,
    ) {
        commit_prov(
            prov,
            &enclosing,
            section,
            path,
            &verb_fired,
            &changed,
            file_old,
        );
        return Some(enclosing);
    }
    if let Some(verbatim) =
        super::verbatim::try_verbatim_sweep(section, path, used_rules, file_old, &mut verb_fired)
    {
        commit_prov(
            prov,
            &verbatim,
            section,
            path,
            &verb_fired,
            &changed,
            file_old,
        );
        return Some(verbatim);
    }
    // T403: verbatim sweep after the whole-section detectors fail.
    // Per-hunk fallback (T349): pairwise-disjoint hunks merge by
    // construction (SPEC 10.6 option-1).
    compress_section_per_hunk(section, path, file_old, used_rules, prov)
}

/// Per-hunk fallback for `compress_section`: compress each balanced
/// native hunk independently with the same detectors, keep the rest as
/// ordinary hunks in original order. Returns `None` when no hunk
/// compressed (caller keeps the ordinary passthrough).
fn compress_section_per_hunk(
    section: &str,
    path: &str,
    file_old: Option<&str>,
    used_rules: &mut Vec<String>,
    prov: &mut Vec<HunkProvenance>,
) -> Option<String> {
    // Split the prelude (`diff --git`/`index`/`---`/`+++`) from the
    // native hunks; a section without hunks has nothing to compress.
    let mut prelude = String::new();
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(String::new());
        }
        match current.as_mut() {
            Some(h) => {
                h.push_str(line);
                h.push('\n');
            }
            None => {
                prelude.push_str(line);
                prelude.push('\n');
            }
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    if hunks.is_empty() {
        return None;
    }
    let mut out = prelude;
    let mut compressed_any = false;
    for hunk in &hunks {
        let mut minuses: Vec<&str> = Vec::new();
        let mut pluses: Vec<&str> = Vec::new();
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(&line[1..]);
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(&line[1..]);
            }
        }
        // T465: partial item-deletion (whole-item minus-heavy hunks only).
        let partial_before = used_rules.len();
        if let Some(d) = super::deletes::try_partial_delete_hunk(hunk, file_old, used_rules) {
            push_section_prov(
                prov,
                &d,
                hunk,
                path,
                "delete",
                &crate::totality::changed_old_lines(hunk),
                file_old,
                partial_before,
                used_rules,
            );
            compressed_any = true;
            out.push_str(&d);
            continue;
        }
        // One balanced hunk through the shared detector chain
        // (`compress_balanced_pair_body` — same shapes the verbatim
        // sweep retries leftovers with); ordinary hunks pass through.
        let changed = crate::totality::changed_old_lines(hunk);
        let pair_before = used_rules.len();
        match crate::compress_balanced_pair_body(
            &minuses,
            &pluses,
            path,
            file_old,
            &changed,
            hunk.len(),
            used_rules,
        ) {
            Some((body, detector)) => {
                push_section_prov(
                    prov,
                    &body,
                    hunk,
                    path,
                    detector,
                    &changed,
                    file_old,
                    pair_before,
                    used_rules,
                );
                compressed_any = true;
                out.push_str(&body);
            }
            None => out.push_str(hunk),
        }
    }
    if compressed_any {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // WI-8: two consts interleaved with two plain fns (long suffixes so
    // the per-pair semantic hunks beat ordinary bytes under T375).
    const PRE: &str = "const OLD_A: usize = 1001;\nfn old_a(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nconst OLD_B: usize = 1002;\nfn old_b(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\n";
    const POST: &str = "const NEW_A: usize = 1001;\nfn new_a(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nconst NEW_B: usize = 1002;\nfn new_b(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\n";

    fn pairs() -> (Vec<&'static str>, Vec<&'static str>) {
        (
            vec![
                "const OLD_A: usize = 1001;",
                "fn old_a(first_argument: FirstLongTypeArgument) {",
                "const OLD_B: usize = 1002;",
                "fn old_b(second_argument: SecondLongTypeArgument) {",
            ],
            vec![
                "const NEW_A: usize = 1001;",
                "fn new_a(first_argument: FirstLongTypeArgument) {",
                "const NEW_B: usize = 1002;",
                "fn new_b(second_argument: SecondLongTypeArgument) {",
            ],
        )
    }

    fn pre_map() -> HashMap<String, String> {
        HashMap::from([("f.rs".to_string(), PRE.to_string())])
    }

    fn applied(pre: &HashMap<String, String>, patch: &str) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("partition emission must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("partition must apply cleanly");
        files
    }

    #[test]
    fn partition_fires_and_round_trips() {
        let (minuses, pluses) = pairs();
        let mut used = Vec::new();
        let (out, detector) = try_partition_section(
            &minuses,
            &pluses,
            "f.rs",
            Some(PRE),
            &[1, 2, 5, 6],
            &mut used,
        )
        .expect("all-item section must partition");
        assert_eq!(detector, "item_rename");
        assert_eq!(used.len(), 4);
        assert_eq!(out.matches("rule=").count(), 4, "one hunk per pair:\n{out}");
        assert!(
            out.contains("rule=r1") && out.contains("rule=r4"),
            "positional rules:\n{out}"
        );
        let files = applied(&pre_map(), &out);
        assert_eq!(files["f.rs"], POST, "round-trip must be byte-identical");
    }

    #[test]
    fn partition_vetoes_matrix() {
        let (minuses, pluses) = pairs();
        // Single pair: owned by the single-pair path, never partition.
        let mut used = Vec::new();
        assert!(try_partition_section(
            &minuses[..1],
            &pluses[..1],
            "f.rs",
            Some(PRE),
            &[1],
            &mut used
        )
        .is_none());
        assert!(used.is_empty());
        // Duplicate minus lines: identical pairs belong to verbatim.
        let mut used = Vec::new();
        let dup_m = vec![minuses[0], minuses[0]];
        let dup_p = vec![pluses[0], pluses[0]];
        assert!(
            try_partition_section(&dup_m, &dup_p, "f.rs", Some(PRE), &[1, 1], &mut used).is_none()
        );
        assert!(used.is_empty());
        // One unclaimable pair vetoes the whole section (atomicity) and
        // releases every reservation.
        let mut used = Vec::new();
        let mut bad_m = minuses.clone();
        let mut bad_p = pluses.clone();
        bad_m.push("    indented = true;");
        bad_p.push("    indented = false;");
        assert!(try_partition_section(
            &bad_m,
            &bad_p,
            "f.rs",
            Some(PRE),
            &[1, 2, 5, 6, 3],
            &mut used
        )
        .is_none());
        assert!(used.is_empty());
        // Pair-to-site mapping must be exact.
        let mut used = Vec::new();
        assert!(
            try_partition_section(&minuses, &pluses, "f.rs", Some(PRE), &[1, 2, 4], &mut used)
                .is_none()
        );
        assert!(used.is_empty());
        // The plain-fn gate applies per pair: visibility declines.
        let mut used = Vec::new();
        let mut vis_m = minuses.clone();
        let mut vis_p = pluses.clone();
        vis_m[1] = "pub fn old_a(first_argument: FirstLongTypeArgument) {";
        vis_p[1] = "pub fn new_a(first_argument: FirstLongTypeArgument) {";
        assert!(
            try_partition_section(&vis_m, &vis_p, "f.rs", Some(PRE), &[1, 2, 5, 6], &mut used)
                .is_none()
        );
        assert!(used.is_empty());
        // Return-type headers decline (today's single-pair gate).
        let mut used = Vec::new();
        let mut ret_m = minuses.clone();
        let mut ret_p = pluses.clone();
        ret_m[1] = "fn old_a(first_argument: FirstLongTypeArgument) -> i32 {";
        ret_p[1] = "fn new_a(first_argument: FirstLongTypeArgument) -> i32 {";
        assert!(
            try_partition_section(&ret_m, &ret_p, "f.rs", Some(PRE), &[1, 2, 5, 6], &mut used)
                .is_none()
        );
        assert!(used.is_empty());
    }

    #[test]
    fn partition_section_fires_end_to_end() {
        // Two -U0 hunks (adjacent const+fn pairs merge per hunk), whole
        // balanced: chain and verbatim decline, partition emits 4/4.
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,2 @@\n-const OLD_A: usize = 1001;\n-fn old_a(first_argument: FirstLongTypeArgument) {\n+const NEW_A: usize = 1001;\n+fn new_a(first_argument: FirstLongTypeArgument) {\n@@ -5,2 +5,2 @@\n-const OLD_B: usize = 1002;\n-fn old_b(second_argument: SecondLongTypeArgument) {\n+const NEW_B: usize = 1002;\n+fn new_b(second_argument: SecondLongTypeArgument) {\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let pre = pre_map();
        let out =
            compress_section(section, &mut used, &pre, &mut prov).expect("partition must fire");
        assert_eq!(out.matches("rule=").count(), 4, "got:\n{out}");
        assert_eq!(prov.len(), 4, "one provenance entry per hunk: {prov:?}");
        assert!(prov.iter().all(|p| p.detector == "item_rename"), "{prov:?}");
        assert!(
            prov.iter().all(|p| p.group.is_empty()),
            "distinct deltas never group: {prov:?}"
        );
        let files = applied(&pre, &out);
        assert_eq!(files["f.rs"], POST);
    }

    #[test]
    fn partition_section_declines_tiny_structs() {
        // -U0 tiny structs: per-pair semantic overhead beats the shared
        // header saving, so section T375 declines (per-hunk singles lose
        // too) — the section stays ordinary.
        let tiny = "struct Old1 {\n    x: i32,\n}\nstruct Old2 {\n    x: i32,\n}\nstruct Old3 {\n    x: i32,\n}\n";
        let section = "diff --git a/g.rs b/g.rs\n--- a/g.rs\n+++ b/g.rs\n@@ -1 +1 @@\n-struct Old1 {\n+struct New1 {\n@@ -4 +4 @@\n-struct Old2 {\n+struct New2 {\n@@ -7 +7 @@\n-struct Old3 {\n+struct New3 {\n";
        let mut pre = HashMap::new();
        pre.insert("g.rs".to_string(), tiny.to_string());
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(
            compress_section(section, &mut used, &pre, &mut prov).is_none(),
            "T375 must decline"
        );
        assert!(used.is_empty());
        assert!(prov.is_empty());
    }

    #[test]
    fn partition_section_atomicity_stays_ordinary() {
        // One hunk, three pairs: two renames plus one ordinary edit — the
        // edit vetoes partition, the chain declines hetero pairs, and the
        // per-hunk fallback declines the mixed hunk: fully ordinary.
        let pre = "const OLD_A: usize = 1001;\nconst OLD_B: usize = 1002;\nlet x = 1;\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,3 +1,3 @@\n-const OLD_A: usize = 1001;\n-const OLD_B: usize = 1002;\n-let x = 1;\n+const NEW_A: usize = 1001;\n+const NEW_B: usize = 1002;\n+let x = 2;\n";
        let mut files = HashMap::new();
        files.insert("f.rs".to_string(), pre.to_string());
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(compress_section(section, &mut used, &files, &mut prov).is_none());
        assert!(used.is_empty());
        assert!(prov.is_empty());
    }
}
