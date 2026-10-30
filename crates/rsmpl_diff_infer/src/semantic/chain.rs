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
/// unchanged; WI-4a appends one shape at the end, WI-4b one more, WI-4c one more): slot → sig-evolution → method-to-fn → method-to-method →
/// same-literal → `&`-strip → qualified-call → turbofish →
/// macro-rename → use-prefix → bare call → call-arg-add →
/// call-literal → call-peel → let-type-add → type-sweep. Returns the winning semantic section
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
    // WI-1c: nested same-callee full-strip fold (`Box::new(Box::new(x))`
    // → `x`, `# repeat`) — after the type sweep (those own whole-statement
    // templates; this owns call-spine strips), before the verbatim stage
    // in the caller. No earlier detector claims a call→non-call pair.
    if let Some(fold) =
        super::try_repeat_fold_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        return Some((fold, "repeat_fold"));
    }
    None
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
    let mut verb_fired: Vec<(String, &'static str)> = Vec::new();
    // WI-9a: same-name concrete macro interiors before verbatim (which
    // vetoes every macro, so no double-claim).
    if let Some(interior) =
        super::try_macro_interior_section(section, path, used_rules, file_old, &mut verb_fired)
    {
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
