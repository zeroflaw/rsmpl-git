//! v1 semantic inference: fn/item rename + slot-sweep + call-sweep +
//! array-head-peel call-migration sweep (T441) + method-to-function,
//! method-to-method, &-receiver strip, use-path prefix,
//! whole-file item-deletion (T464) + partial-file item-deletion (T465),
//! and whole-item move (T430) fan-outs. Every emission clears a token-win guard (T375): the
//! semantic hunk is kept only when byte-smaller than the ordinary
//! hunk(s) it replaces (bytes proxy tokens), else the ordinary text
//! stays — small singles and tiny sweeps never cost more than the
//! passthrough.
//!
//! Pure detectors over a binary-stripped diff. Whole-file first (cross-hunk
//! sweeps); else per balanced hunk (T349), leftovers ordinary in order
//! (pairwise-disjoint, SPEC 10.6 option-1). Every per-hunk emission uses
//! tight original range + `# allow=many` (SPEC D.5, WI-1b); probe-verified via `applyrs --check`.
//! Whole-item moves (T430 pre-pass) cite real ranges/anchors instead — a
//! scope=file cut would match every item in the file.
mod calls;
mod deletes;
mod literal;
mod methods;
mod moves;
mod peel;
pub(crate) mod prefix;
pub(crate) mod prov;
mod slot;
mod verbatim;

pub(crate) use prefix::try_use_prefix_sweep;
pub(crate) use slot::try_fn_slot_sweep;

pub(crate) use calls::{try_call_arg_add_sweep, try_call_sweep};
pub(crate) use literal::{try_call_literal_section, try_call_literal_sweep};
pub(crate) use methods::{
    try_method_same_literal_sweep, try_method_to_fn_sweep, try_method_to_method_sweep,
    try_ref_method_strip_sweep,
};

/// WI-1b: covering item span for item-shape detectors (item rename,
/// slot sweep). Item patterns (`- fn Old`, slot headers with `...`
/// bodies) match whole multi-line items, so a 1-line site range can
/// never match under Contained scope — the range must cover every
/// item containing a site line. Returns the tight
/// `(start, end)` over the smallest covering items, or `None` when
/// the pre-image is absent/unparseable or a site sits outside any
/// item (callers keep `# sweep`). Spans come from `syn` (which
/// includes attrs/docs) via the workspace `span-locations` probe.
pub(crate) fn covering_item_span(
    file_old: Option<&str>,
    sites: &[usize],
) -> Option<(usize, usize)> {
    let file: syn::File = syn::parse_str(file_old?).ok()?;
    fn push(sp: proc_macro2::Span, out: &mut Vec<(usize, usize)>) {
        let (s, e) = (sp.start().line, sp.end().line);
        if s >= 1 && e >= s {
            out.push((s, e));
        }
    }
    fn walk(items: &[syn::Item], out: &mut Vec<(usize, usize)>) {
        use syn::spanned::Spanned;
        for it in items {
            push(it.span(), out);
            match it {
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, out);
                    }
                }
                syn::Item::Impl(imp) => {
                    for i in &imp.items {
                        push(i.span(), out);
                    }
                }
                syn::Item::Trait(tr) => {
                    for i in &tr.items {
                        push(i.span(), out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    walk(&file.items, &mut spans);
    let mut start = usize::MAX;
    let mut end = 0usize;
    for &site in sites {
        let mut best: Option<(usize, usize)> = None;
        for &(s, e) in &spans {
            if s <= site && site <= e && best.is_none_or(|(bs, be)| e - s < be - bs) {
                best = Some((s, e));
            }
        }
        let (s, e) = best?;
        start = start.min(s);
        end = end.max(e);
    }
    if end == 0 {
        return None;
    }
    Some((start, end))
}

/// WI-1b: item-shape emission header. `sites` are the changed header
/// lines; the emitted range covers their items when provable, else
/// the sweep dummy (a 1-line header range could never contain the
/// multi-line match).
pub(crate) fn item_emission_header(
    path: &str,
    rule: &str,
    file_old: Option<&str>,
    sites: &[usize],
) -> String {
    match covering_item_span(file_old, sites) {
        Some((s, e)) => crate::emission_header(path, rule, &[s, e]),
        None => crate::emission_header(path, rule, &[]),
    }
}
pub(crate) use peel::{try_call_peel_section, try_call_peel_sweep};

/// Try to compress binary-stripped `sections` (one full
/// `diff --git …` section each) into semantic hunks. Returns the
/// compressed patch, or `None` when no section compressed (caller
/// keeps the ordinary passthrough).
pub fn infer_semantic(sections: &[String]) -> Option<String> {
    infer_semantic_with_pre_images(sections, &std::collections::HashMap::new())
}

/// `infer_semantic` with pre-image file contents (T403 totality):
/// `pre` maps collapsed b-side paths to the old file text (assembled
/// by the caller, which owns the repo). Legacy `infer_semantic` passes
/// an empty map (section-visible evidence only).
pub fn infer_semantic_with_pre_images(
    sections: &[String],
    pre: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let mut prov = Vec::new();
    infer_semantic_with_provenance(sections, pre, &mut prov)
}

/// WI-2 provenance-collecting inference: as above, plus one
/// [`HunkProvenance`] entry per emitted semantic hunk in `prov`.
pub fn infer_semantic_with_provenance(
    sections: &[String],
    pre: &std::collections::HashMap<String, String>,
    prov: &mut Vec<HunkProvenance>,
) -> Option<String> {
    let mut out = String::new();
    let mut used_rules: Vec<String> = Vec::new();
    let mut compressed_any = false;
    // T430 whole-patch move pre-pass (cross-section bind/use pairs);
    // per-section detectors run on the rewritten sections below.
    let sections_owned: Vec<String>;
    let move_rules_before = used_rules.len();
    let sections: &[String] = match moves::try_move_sweep(sections, pre, &mut used_rules) {
        Some(rewritten) => {
            compressed_any = true;
            push_move_prov(
                prov,
                sections,
                &rewritten,
                pre,
                move_rules_before,
                &used_rules,
            );
            sections_owned = rewritten;
            &sections_owned
        }
        None => sections,
    };
    for section in sections {
        match compress_section(section, &mut used_rules, pre, prov) {
            Some(semantic) => {
                compressed_any = true;
                out.push_str(&semantic);
            }
            None => out.push_str(section),
        }
    }
    if compressed_any {
        Some(out)
    } else {
        None
    }
}

fn compress_section(
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
    // T377: rename sections stay ordinary passthrough. Detectors key
    // emissions to the b-side path, which would seed from the wrong
    // file; rename passthrough is already near-minimal.
    if is_rename_header(header) {
        return None;
    }
    let path = super::b_side_path(header.strip_prefix("diff --git ")?)?;
    if !path.ends_with(".rs") {
        return None;
    }
    // T464: whole-file item-deletion sweep (minus-only deletion sections
    // compress to one many-delete hunk + ordinary leftovers).
    let delete_before = used_rules.len();
    if let Some(deleted) = deletes::try_delete_sweep(section, used_rules) {
        push_section_prov(
            prov,
            &deleted,
            section.len(),
            &path,
            "delete",
            &crate::totality::changed_old_lines(section),
            pre.get(&path).map(String::as_str),
            delete_before,
            used_rules,
        );
        return Some(deleted);
    }
    // T403 totality input: the pre-image text for this path, if the
    // caller supplied it (None = section-visible evidence only).
    let file_old = pre.get(&path).map(String::as_str);
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
            try_item_rename(minuses[0], pluses[0], &path, file_old, &changed, used_rules)
        {
            if rename.len() < section.len() {
                push_section_prov(
                    prov,
                    &rename,
                    section.len(),
                    &path,
                    "item_rename",
                    &changed,
                    file_old,
                    checkpoint,
                    used_rules,
                );
                return Some(rename);
            }
            used_rules.truncate(checkpoint);
        }
        return compress_section_per_hunk(section, &path, file_old, used_rules, prov);
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
            try_call_peel_section(section, &path, used_rules, file_old, &mut fired)
        {
            rewritten = Some(peeled);
        }
        let after_peel = rewritten.as_deref().unwrap_or(section);
        if let Some(lit) =
            try_call_literal_section(after_peel, &path, used_rules, file_old, &mut fired)
        {
            rewritten = Some(lit);
        }
        let after_lit = rewritten.as_deref().unwrap_or(section);
        if let Some(verbatim) =
            verbatim::try_verbatim_sweep(after_lit, &path, used_rules, file_old, &mut fired)
        {
            commit_prov(
                prov,
                &verbatim,
                section.len(),
                &path,
                &fired,
                &changed,
                file_old,
            );
            return Some(verbatim);
        }
        if let Some(r) = rewritten {
            commit_prov(prov, &r, section.len(), &path, &fired, &changed, file_old);
            return Some(r);
        }
        // T403: non-whole sections try the cross-hunk verbatim sweep
        // before per-hunk (whole-section detectors need balance) —
        // already attempted above on the unmodified section.
        return compress_section_per_hunk(section, &path, file_old, used_rules, prov);
    }
    // N >= 2 heterogeneous fn/method renames compress to one slot
    // sweep (T344); fall through to the call sweep otherwise.
    if let Some(slot) = try_fn_slot_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if slot.len() < section.len() {
            push_section_prov(
                prov,
                &slot,
                section.len(),
                &path,
                "fn_slot_sweep",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(slot);
        }
        used_rules.truncate(checkpoint);
    }
    // N >= 2 method-to-function fan-outs (T345), method-to-method
    // renames (T347), `&`-receiver strips (T359), and use-path prefix
    // renames (T363); fall through to the bare-call sweep otherwise.
    if let Some(m2f) =
        try_method_to_fn_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if m2f.len() < section.len() {
            push_section_prov(
                prov,
                &m2f,
                section.len(),
                &path,
                "method_to_fn",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(m2f);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(m2m) =
        try_method_to_method_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if m2m.len() < section.len() {
            push_section_prov(
                prov,
                &m2m,
                section.len(),
                &path,
                "method_to_method",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(m2m);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(same) =
        try_method_same_literal_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if same.len() < section.len() {
            push_section_prov(
                prov,
                &same,
                section.len(),
                &path,
                "method_same_literal",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(same);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(strip) =
        try_ref_method_strip_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if strip.len() < section.len() {
            push_section_prov(
                prov,
                &strip,
                section.len(),
                &path,
                "ref_strip",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(strip);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(uses) =
        try_use_prefix_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if uses.len() < section.len() {
            push_section_prov(
                prov,
                &uses,
                section.len(),
                &path,
                "use_prefix",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(uses);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(sweep) = try_call_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules) {
        if sweep.len() < section.len() {
            push_section_prov(
                prov,
                &sweep,
                section.len(),
                &path,
                "call_sweep",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(sweep);
        }
        used_rules.truncate(checkpoint);
    }
    // T447: same-callee trailing-arg addition (homo concrete tails
    // compress; hetero tails like 09d3446's `symlink` vs `false`
    // veto via minus-arity totality).
    if let Some(add) =
        try_call_arg_add_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if add.len() < section.len() {
            push_section_prov(
                prov,
                &add,
                section.len(),
                &path,
                "call_arg_add",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(add);
        }
        used_rules.truncate(checkpoint);
    }
    // T448: same-callee same-arity trailing-literal delta (homo concrete
    // trailing pair, hetero leading bound as E-vars; different-trailing
    // same-arity sites never veto via tuple-keyed totality).
    if let Some(lit) =
        try_call_literal_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if lit.len() < section.len() {
            push_section_prov(
                prov,
                &lit,
                section.len(),
                &path,
                "call_literal",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(lit);
        }
        used_rules.truncate(checkpoint);
    }
    // T441: hetero-arg array-head-peel call migration
    // (`f(D, &[H, M, T…])` → `g(D, M, &[T…])` per-tail-arity groups);
    // all-identical classes stay for the tighter verbatim sweep below.
    if let Some(peel) =
        try_call_peel_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if peel.len() < section.len() {
            push_section_prov(
                prov,
                &peel,
                section.len(),
                &path,
                "call_peel",
                &changed,
                file_old,
                checkpoint,
                used_rules,
            );
            return Some(peel);
        }
        used_rules.truncate(checkpoint);
    }
    let mut verb_fired: Vec<(String, &'static str)> = Vec::new();
    if let Some(verbatim) =
        verbatim::try_verbatim_sweep(section, &path, used_rules, file_old, &mut verb_fired)
    {
        commit_prov(
            prov,
            &verbatim,
            section.len(),
            &path,
            &verb_fired,
            &changed,
            file_old,
        );
        return Some(verbatim);
    }
    // T403: verbatim sweep after the whole-section detectors fail.
    // Per-hunk fallback (T349): pairwise-disjoint hunks merge by
    // construction (SPEC 10.6 option-1).
    compress_section_per_hunk(section, &path, file_old, used_rules, prov)
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
        if let Some(d) = deletes::try_partial_delete_hunk(hunk, file_old, used_rules) {
            push_section_prov(
                prov,
                &d,
                hunk.len(),
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
        match super::compress_balanced_pair_body(
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
                    hunk.len(),
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

/// One-ident-delta item rename: exactly one `-`/`+` line pair, same
/// kind/prefix/suffix, idents differ. Column-0 (top-level) is
/// enforced: indented lines are methods or block items a bare
/// pattern cannot match.
pub(crate) fn try_item_rename(
    old_line: &str,
    new_line: &str,
    path: &str,
    file_old: Option<&str>,
    sites: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let old = old_line.trim_end();
    let new = new_line.trim_end();
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(old)?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(new)?;
    if old_kind != new_kind || old_prefix != new_prefix || old_suffix != new_suffix {
        return None;
    }
    if old_ident == new_ident {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_ident}_to_{new_ident}"));
    let mut hunk = item_emission_header(path, &rule, file_old, sites);
    if old_kind == "fn" {
        // WI-1e diet terse: `- fn Old` / `+ fn New` (params/return/
        // generics/where unconstrained, SPEC 14.1.1). Plain top-level
        // fns only; vis-prefixed or evolved signatures stay ordinary
        // (slot sweep covers the multi-site case).
        if !old_prefix.is_empty() {
            return None;
        }
        let suffix = old_suffix.trim_start();
        if !suffix.starts_with('(') || !old.trim_end().ends_with('{') {
            return None;
        }
        if suffix.contains("->") || contains_word(suffix, "where") {
            return None;
        }
        hunk.push_str(&format!("- fn {old_ident}\n+ fn {new_ident}\n"));
    } else {
        hunk.push_str(&format!(
            "- {old_prefix}{old_kind} {old_ident}\n+ {new_prefix}{new_kind} {new_ident}\n"
        ));
    }
    Some(hunk)
}

/// Split `fn foo() {`-style header lines into
/// (vis prefix, kind, ident, suffix). Prefix is `""` or `"pub … "`
/// (with trailing space); suffix is everything after the ident.
fn split_item(line: &str) -> Option<(String, String, String, String)> {
    const KINDS: &[&str] = &[
        "fn", "struct", "enum", "trait", "type", "mod", "union", "const", "static",
    ];
    let trimmed = line.trim_start();
    // Column-0 is enforced by the caller; a trimmed start here only
    // tolerates nothing — require the line itself unindented.
    if trimmed.len() != line.len() {
        return None;
    }
    let (prefix, rest) = if let Some(r) = trimmed.strip_prefix("pub") {
        // `pub`, `pub(…)`, each followed by whitespace.
        if r.is_empty() || r.starts_with([' ', '\t']) {
            ("pub ".to_string(), r.trim_start().to_string())
        } else if r.starts_with('(') {
            let end = r.find(')')?;
            let after = &r[end + 1..];
            if after.is_empty() || !after.starts_with([' ', '\t']) {
                return None;
            }
            (
                format!("pub{} ", &r[..=end]),
                after.trim_start().to_string(),
            )
        } else {
            return None;
        }
    } else {
        (String::new(), trimmed.to_string())
    };
    let kind_end = rest.find([' ', '\t'])?;
    let kind = &rest[..kind_end];
    if !KINDS.contains(&kind) {
        return None;
    }
    let after_kind = rest[kind_end..].trim_start();
    let ident_len = ident_len(after_kind)?;
    let ident = &after_kind[..ident_len];
    let suffix = after_kind[ident_len..].to_string();
    if ident.is_empty() || suffix.is_empty() {
        return None;
    }
    Some((prefix, kind.to_string(), ident.to_string(), suffix))
}

/// Length of a plain (non-raw) Rust ident prefix.
pub(crate) fn ident_len(s: &str) -> Option<usize> {
    let mut chars = s.char_indices();
    let (i, c) = chars.next()?;
    if c != '_' && !c.is_ascii_alphabetic() {
        return None;
    }
    let mut len = i + c.len_utf8();
    for (i, c) in chars {
        if c == '_' || c.is_ascii_alphanumeric() {
            len = i + c.len_utf8();
        } else {
            break;
        }
    }
    Some(len)
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w == word)
}

pub(crate) fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Strip one trailing statement/arg terminator (`;`, else `,`) for
/// shape parsing; the emitted hunk matches sub-expressions, so the
/// terminator never reaches the pattern.
pub(crate) fn strip_one_terminator(text: &str) -> Option<&str> {
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = text.strip_suffix(';') {
        return Some(stripped.trim_end());
    }
    if let Some(stripped) = text.strip_suffix(',') {
        return Some(stripped.trim_end());
    }
    Some(text)
}

/// Strip one leading struct-literal field label (`label:`) for shape
/// parsing (T416): `text: f(&sub)` sweeps on the inner `f(&sub)` — the
/// engine general-expr matcher visits field values as sub-expressions,
/// so a Call pattern rewrites them with the label preserved
/// (probe-verified on c48a6e6). Returns the label and the remainder.
/// `a::b(...)` paths never strip (colon followed by another colon).
pub(crate) fn strip_field_label(text: &str) -> Option<(&str, &str)> {
    let (label, rest) = text.split_once(':')?;
    if rest.starts_with(':') {
        return None;
    }
    let label = label.trim();
    if label.is_empty() || !label.chars().all(is_ident_char) {
        return None;
    }
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((label, rest))
}

/// Whitespace-insensitive text for detection-time containment and
/// replacement checks (the engine re-verifies structurally at apply).
pub(crate) fn flat_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(crate) fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.match_indices(needle).count()
}

fn parse_call_expr(line: &str) -> Option<syn::Expr> {
    let mut text = line.trim();
    if text.is_empty() {
        return None;
    }
    // Call sites end the statement with `;`.
    if let Some(stripped) = text.strip_suffix(';') {
        text = stripped.trim_end();
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        // A `let` scrutinee parses as `Expr::Let` — decline it here;
        // the `Stmt` unwrap below extracts the init call instead
        // (verbatim takes the same `Let`-declines-`Expr` split, T440).
        if !matches!(expr, syn::Expr::Let(_)) {
            return Some(expr);
        }
    }
    // T449: call sites inside `let` inits (`let a = f(x);`) — the
    // whole line parses as a `Stmt`, not an `Expr`. Unwrap the init
    // (peel `peel_init` precedent; the `Stmt` parse needs the
    // semicolon `text` just lost, so retry on the trimmed line); the
    // emission stays a bare-call pattern and the engine rewrites the
    // call sub-expression in place (probe-verified `applyrs --check`
    // green). `let-else` never qualifies (the engine never moves
    // into diverge targets).
    match syn::parse_str::<syn::Stmt>(line.trim()) {
        Ok(syn::Stmt::Local(local)) => {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            Some(*init.expr)
        }
        _ => None,
    }
}

/// Bare single-segment call callee + args; rejects method calls,
/// paths, turbofish, and non-ident callees.
pub(crate) fn call_parts(expr: &syn::Expr) -> Option<(String, Vec<syn::Expr>)> {
    if let syn::Expr::Call(call) = expr {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            if path.qself.is_some() || path.path.leading_colon.is_some() {
                return None;
            }
            if path.path.segments.len() != 1 {
                return None;
            }
            let seg = &path.path.segments[0];
            if !matches!(seg.arguments, syn::PathArguments::None) {
                return None;
            }
            let name = seg.ident.to_string();
            if ident_len(&name)? != name.len() {
                return None;
            }
            return Some((name, call.args.iter().cloned().collect()));
        }
    }
    None
}

pub(crate) fn unique_rule(used: &mut Vec<String>, _base: &str) -> String {
    // WI-1e diet: rule ids are `r1…` in emission order (SPEC 9.3 diet).
    // The descriptive base is ignored; uniqueness is positional.
    let candidate = format!("r{}", used.len() + 1);
    used.push(candidate.clone());
    candidate
}

pub use prov::HunkProvenance;

use prov::{commit_prov, push_move_prov, push_section_prov};

/// True when a `diff --git` header names a rename (differing real
/// sides, T377) via the shared space-aware splitter.
fn is_rename_header(header: &str) -> bool {
    let header = header.lines().next().unwrap_or(header);
    let Some(rest) = header.strip_prefix("diff --git ") else {
        return false;
    };
    let Some(parts) = rsmpl_patch::split_git_paths(rest) else {
        return false;
    };
    if parts.len() != 2 {
        return false;
    }
    let old = parts[0].strip_prefix("a/").unwrap_or(&parts[0]);
    let new = parts[1].strip_prefix("b/").unwrap_or(&parts[1]);
    if old == "/dev/null" || old == "dev/null" || new == "/dev/null" || new == "dev/null" {
        return false;
    }
    old != new
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

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
    fn rename_sections_stay_ordinary_passthrough() {
        // T377: rename sections never compress -- detectors key
        // emissions to the b-side path, which would seed from the
        // wrong file. The content below is the `fn_rename_compresses`
        // shape (a proven winner), yet the rename header vetoes it.
        let raw = "diff --git a/old.rs b/new.rs\nsimilarity index 89%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1,3 +1,3 @@\n-fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n+fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n     1\n }\n";
        assert!(infer_semantic(&[raw.to_string()]).is_none());
        assert!(is_rename_header("diff --git a/old.rs b/new.rs"));
        assert!(!is_rename_header("diff --git a/f.rs b/f.rs"));
        assert!(!is_rename_header(
            "diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644"
        ));
    }

    #[test]
    fn space_path_rs_sections_compress() {
        // T380: the old `splitn(2, ' ')` b-side mis-split unquoted
        // space paths (real git never quotes them), so a space-path
        // `.rs` section silently never compressed. Shared splitter
        // recovers the proven-winner shape.
        let raw = "diff --git a/my file.rs b/my file.rs\n--- a/my file.rs\n+++ b/my file.rs\n@@ -1,3 +1,3 @@\n-fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n+fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n     1\n }\n";
        let pre = pre_map(&[(
            "my file.rs",
            "fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n1\n}\n",
        )]);
        let out = infer_semantic(&[raw.to_string()]).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("diff --git my file.rs\n"), "got: {out}");
        assert!(!out.contains("diff --git a/"), "got: {out}");
        assert!(out.contains("- fn foo"), "got: {out}");
    }

    #[test]
    fn item_rename_with_pre_image_emits_covering_range() {
        // WI-1b: with a parseable pre-image the single rename emits the
        // covering item range (the 3-line fn), not the sweep dummy — a
        // 1-line header range could never contain the multi-line match.
        // Without a pre-image the same shape stays sweep (legacy path,
        // pinned by `fn_rename_compresses`).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n+fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n     1\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n1\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("@@ -1,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn call_sweep_scattered_beyond_64_lines_stays_sweep() {
        // WI-1b: sites 99 lines apart exceed the 64-line rule — the
        // sweep dummy keeps one hunk where a ranged hunk would
        // over-claim original coordinates.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-    foo(alpha);\n+    bar(alpha);\n@@ -100,1 +100,1 @@\n-    foo(beta);\n+    bar(beta);\n",
        );
        let mut body = String::from("fn m() {\n    foo(alpha);\n");
        for i in 0..97 {
            body.push_str(&format!("    filler{i}();\n"));
        }
        body.push_str("    foo(beta);\n}\n");
        let pre = pre_map(&[("f.rs", body.as_str())]);
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("@@ -1,1 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# sweep\n"), "got: {out}");
    }

    #[test]
    fn fn_rename_compresses() {
        // Long-signature rename: the `fn …(...)` pattern lines are far
        // shorter than the ordinary lines, so the semantic hunk wins
        // (T375 guard). Short headers stay ordinary (see below).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n+fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n     1\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n1\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn foo"), "got: {out}");
        assert!(out.contains("+ fn bar"), "got: {out}");
        assert!(out.contains("# sweep"), "got: {out}");
        assert!(!out.contains("-fn foo"), "got: {out}");
    }

    #[test]
    fn short_fn_rename_fires_as_diet() {
        // WI-1e diet (terse `- fn foo` + `# sweep`, no lang): the
        // short-header single now wins the T375 byte guard.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo() {\n+fn bar() {\n     1\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("diet wins T375");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# sweep\n"), "got: {out}");
        assert!(out.contains("- fn foo\n"), "got: {out}");
        assert!(out.contains("+ fn bar\n"), "got: {out}");
    }

    #[test]
    fn struct_rename_fires_as_diet() {
        // WI-1e diet: `- struct Old` (no suffix) + `# sweep` beats the
        // ordinary hunk, so the single now fires (byte-guard flip pinned).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-struct Old {\n+struct New {\n     x: i32,\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("diet wins T375");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("- struct Old\n"), "got: {out}");
    }

    #[test]
    fn pub_struct_rename_fires_as_diet() {
        // WI-1e diet: same flip as the plain struct single.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-pub struct Old {\n+pub struct New {\n     x: i32,\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("diet wins T375");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("- pub struct Old\n"), "got: {out}");
    }

    #[test]
    fn indented_fn_stays_passthrough() {
        // Methods live indented; a bare fn pattern cannot match them.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n impl T {\n-    fn foo(&self) {\n+    fn bar(&self) {\n     }\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn generic_fn_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo<T>(x: T) {\n+fn bar<T>(x: T) {\n     1\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_sweep_three_sites_one_hunk() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n-    foo(3);\n+    bar(1);\n+    bar(2);\n+    bar(3);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
        // One hunk for all three sites.
        assert_eq!(out.matches("@@ -2,3 @@ rule=r1\n").count(), 1, "got: {out}");
    }

    #[test]
    fn call_sweep_zero_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo();\n-    foo();\n+    bar();\n+    bar();\n }\n",
        );
        let pre = pre_map(&[("f.rs", "fn m() {\n    foo();\n    foo();\n}\n")]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- foo()\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
    }

    #[test]
    fn call_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    old_function(first, second);\n-    old_function(third, fourth);\n-    old_function(fifth, sixth);\n+    new_function(first, second);\n+    new_function(third, fourth);\n+    new_function(fifth, sixth);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_function(first, second);\n    old_function(third, fourth);\n    old_function(fifth, sixth);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_function(E0, E1)\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
    }

    #[test]
    fn call_sweep_split_hunks_pair_in_order() {
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-    old_function(first_argument);\n+    new_function(first_argument);\n@@ -9,2 +9,2 @@\n-    old_function(second_argument);\n+    new_function(second_argument);\n",
        );
        // WI-1b: the ranged hunk `@@ -1,9 @@` must be covered by the
        // pre-image (sweep dummies never needed this); filler lines
        // carry no `old_function` sites.
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_function(first_argument);\n    filler_one();\n    filler_two();\n    filler_three();\n    filler_four();\n    filler_five();\n    filler_six();\n    old_function(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -1,9 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn single_call_site_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    foo(1);\n+    bar(1);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn mixed_callees_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    baz(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn changed_args_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(3);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_arg_add_three_sites_fire() {
        // T447: homo concrete trailing addition, hetero leading args.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    make_widget(alpha, 1);\n-    make_widget(beta, 2);\n-    make_widget(gamma, 3);\n+    make_widget(alpha, 1, false);\n+    make_widget(beta, 2, false);\n+    make_widget(gamma, 3, false);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha, 1);\n    make_widget(beta, 2);\n    make_widget(gamma, 3);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("- make_widget(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ make_widget(E0, E1, false)\n"), "got: {out}");
    }

    #[test]
    fn call_arg_add_zero_prefix_fire() {
        // T447: minus arity 0 grows by one concrete arg.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget();\n-    make_widget();\n+    make_widget(false);\n+    make_widget(false);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget();\n    make_widget();\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(out.contains("- make_widget()\n"), "got: {out}");
        assert!(out.contains("+ make_widget(false)\n"), "got: {out}");
    }

    #[test]
    fn call_arg_add_multi_trailing_fire() {
        // T447: two shared concrete trailing args.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha);\n-    make_widget(beta);\n+    make_widget(alpha, false, 1);\n+    make_widget(beta, false, 1);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha);\n    make_widget(beta);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("+ make_widget(E0, false, 1)\n"), "got: {out}");
    }

    #[test]
    fn call_arg_add_hetero_trailing_vetoes() {
        // T447: 09d3446 miniature — `symlink` vs `false` trailing is
        // unexpressible (plus-side metavars bind nothing, T442).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1);\n-    make_widget(beta, 2);\n+    make_widget(alpha, 1, symlink);\n+    make_widget(beta, 2, false);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_arg_add_extra_old_arity_site_vetoes() {
        // T447: minus-arity totality — an unchanged third `make_widget`
        // call in the pre-image over-counts (3 != 2).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1);\n-    make_widget(beta, 2);\n+    make_widget(alpha, 1, false);\n+    make_widget(beta, 2, false);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha, 1);\n    make_widget(beta, 2);\n    make_widget(gamma, 3);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn call_arg_add_changed_prefix_vetoes() {
        // T447: a leading arg rewritten in transit is not an addition.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1);\n-    make_widget(beta, 2);\n+    make_widget(alpha, 9, false);\n+    make_widget(beta, 2, false);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_arg_add_single_site_vetoes() {
        // T447: lone additions keep the ordinary hunk (no token win).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    make_widget(alpha, 1);\n+    make_widget(alpha, 1, false);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_sweep_j0_hetero_fires() {
        // T441 slice 1: 05de4d1 miniature — hetero `M` (ident,
        // literal, call) and hetero method-chain receivers share one
        // 2-element head-peel class.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", patch]);\n-    let b = run(dir.path(), &[\"--patch\", \"\"]);\n-    let c = run(other.path(), &[\"--patch\", rename_patch()]);\n+    let a = run_stdin(dir.path(), patch, &[]);\n+    let b = run_stdin(dir.path(), \"\", &[]);\n+    let c = run_stdin(other.path(), rename_patch(), &[]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = run(dir.path(), &[\"--patch\", patch]);\n    let b = run(dir.path(), &[\"--patch\", \"\"]);\n    let c = run(other.path(), &[\"--patch\", rename_patch()]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("# expr T0"), "got: {out}");
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("- run(D, &[\"--patch\", M])\n"), "got: {out}");
        assert!(out.contains("+ run_stdin(D, M, &[])\n"), "got: {out}");
        // Native sites consumed into the pattern.
        assert!(
            !out.contains("run_stdin(dir.path(), patch, &[])"),
            "got: {out}"
        );
    }

    #[test]
    fn call_peel_sweep_j1_hetero_fires() {
        // T441 slice 1: one shared tail position replayed into the new
        // trailing slice; both the peeled value and the tail are hetero.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", patch, \"--check\"]);\n-    let b = run(other.path(), &[\"--patch\", \"\", \"--format\"]);\n+    let a = run_stdin(dir.path(), patch, &[\"--check\"]);\n+    let b = run_stdin(other.path(), \"\", &[\"--format\"]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = run(dir.path(), &[\"--patch\", patch, \"--check\"]);\n    let b = run(other.path(), &[\"--patch\", \"\", \"--format\"]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(
            out.contains("- run(D, &[\"--patch\", M, T0])\n"),
            "got: {out}"
        );
        assert!(out.contains("+ run_stdin(D, M, &[T0])\n"), "got: {out}");
    }

    #[test]
    fn call_peel_sweep_bare_call_statement_fires() {
        // The statement form does not matter: bare call-statements peel
        // exactly like `let` inits (the engine rewrites the call).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    run(dir.path(), &[\"--patch\", a]);\n-    run(dir.path(), &[\"--patch\", b]);\n+    run_stdin(dir.path(), a, &[]);\n+    run_stdin(dir.path(), b, &[]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    run(dir.path(), &[\"--patch\", a]);\n    run(dir.path(), &[\"--patch\", b]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
    }

    #[test]
    fn call_peel_single_pair_stays_passthrough() {
        // T375: one pair can never win as a sweep (the whole-section
        // single path only tries item rename; per-hunk singles likewise).
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", patch]);\n+    let a = run_stdin(dir.path(), patch, &[]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_all_identical_stays_for_verbatim() {
        // An all-identical class compresses tighter without metavars:
        // peel declines, verbatim claims it.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    run(dir.path(), &[\"--patch\", some_long_patch_value]);\n-    run(dir.path(), &[\"--patch\", some_long_patch_value]);\n+    run_stdin(dir.path(), some_long_patch_value, &[]);\n+    run_stdin(dir.path(), some_long_patch_value, &[]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    run(dir.path(), &[\"--patch\", some_long_patch_value]);\n    run(dir.path(), &[\"--patch\", some_long_patch_value]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains("peel"), "got: {out}");
        assert!(out.contains("rule=r1\n"), "got: {out}");
    }

    #[test]
    fn call_peel_tail_drift_stays_passthrough() {
        // Different tail lengths are different classes: no class reaches
        // N=2, and singletons stay ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", x]);\n-    let b = run(dir.path(), &[\"--patch\", y, \"--check\"]);\n+    let a = run_stdin(dir.path(), x, &[]);\n+    let b = run_stdin(dir.path(), y, &[\"--check\"]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_head_drift_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", x]);\n-    let b = run(dir.path(), &[\"--check\", y]);\n+    let a = run_stdin(dir.path(), x, &[]);\n+    let b = run_stdin(dir.path(), y, &[]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_non_string_head_stays_passthrough() {
        // The head replays verbatim: only a string literal qualifies.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[head(), x]);\n-    let b = run(dir.path(), &[head(), y]);\n+    let a = run_stdin(dir.path(), x, &[]);\n+    let b = run_stdin(dir.path(), y, &[]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_plus_tail_mismatch_stays_passthrough() {
        // Minus carries one tail, plus replays none: not a peel.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", x, \"--check\"]);\n-    let b = run(dir.path(), &[\"--patch\", y, \"--check\"]);\n+    let a = run_stdin(dir.path(), x, &[]);\n+    let b = run_stdin(dir.path(), y, &[]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_totality_overcount_vetoes() {
        // An unchanged same-shape site in the pre-image breaks the
        // count gate: the sweep would silently rewrite it.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(d, &[\"--patch\", x]);\n-    let b = run(d, &[\"--patch\", y]);\n+    let a = run_stdin(d, x, &[]);\n+    let b = run_stdin(d, y, &[]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = run(d, &[\"--patch\", x]);\n    let b = run(d, &[\"--patch\", y]);\n    let c = run(d, &[\"--patch\", z]);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn call_peel_section_multi_class_fires_twice() {
        // T441 cross-hunk driver: a non-whole section (pure-addition
        // hunk) with two hetero classes emits one hunk per class.
        let raw = section(
            "f.rs",
            "@@ -1,0 +1,1 @@\n+// marker\n@@ -10,2 +11,2 @@\n-    let a = run(dir.path(), &[\"--patch\", patch]);\n-    let b = run(dir.path(), &[\"--patch\", \"\"]);\n+    let a = run_stdin(dir.path(), patch, &[]);\n+    let b = run_stdin(dir.path(), \"\", &[]);\n@@ -20,2 +22,2 @@\n-    let c = run(dir.path(), &[\"--patch\", patch, \"--check\"]);\n-    let d = run(dir.path(), &[\"--patch\", other, \"--check\"]);\n+    let c = run_stdin(dir.path(), patch, &[\"--check\"]);\n+    let d = run_stdin(dir.path(), other, &[\"--check\"]);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let f0 = 0;\n    let f1 = 0;\n    let f2 = 0;\n    let f3 = 0;\n    let f4 = 0;\n    let f5 = 0;\n    let f6 = 0;\n    let f7 = 0;\n    let a = run(dir.path(), &[\"--patch\", patch]);\n    let b = run(dir.path(), &[\"--patch\", \"\"]);\n    let g0 = 0;\n    let g1 = 0;\n    let g2 = 0;\n    let g3 = 0;\n    let g4 = 0;\n    let g5 = 0;\n    let g6 = 0;\n    let g7 = 0;\n    let c = run(dir.path(), &[\"--patch\", patch, \"--check\"]);\n    let d = run(dir.path(), &[\"--patch\", other, \"--check\"]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- run(D, &[\"--patch\", M])\n"), "got: {out}");
        assert!(
            out.contains("- run(D, &[\"--patch\", M, T0])\n"),
            "got: {out}"
        );
        assert!(out.contains("+ run_stdin(D, M, &[])\n"), "got: {out}");
        assert!(out.contains("+ run_stdin(D, M, &[T0])\n"), "got: {out}");
        // WI-1b: one ranged hunk per class, disjoint spans.
        assert!(out.contains("@@ -10,2 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("@@ -20,2 @@ rule=r2\n"), "got: {out}");
        assert!(!out.contains("# sweep\n"), "got: {out}");
        // The pure addition stays ordinary in place.
        assert!(out.contains("+// marker"), "got: {out}");
    }

    #[test]
    fn call_peel_body_verbatim_still_fires() {
        // Composition pin: peel bodies are `@@ -1,1 @@` single-pattern
        // hunks, so the later verbatim pass groups them at n=1 (skipped)
        // and still compresses the remaining identical group.
        let raw = section(
            "f.rs",
            "@@ -1,0 +1,1 @@\n+// marker\n@@ -10,2 +11,2 @@\n-    let a = run(dir.path(), &[\"--patch\", patch]);\n-    let b = run(dir.path(), &[\"--patch\", \"\"]);\n+    let a = run_stdin(dir.path(), patch, &[]);\n+    let b = run_stdin(dir.path(), \"\", &[]);\n@@ -20,2 +22,2 @@\n-    compute(some_long_state_value);\n-    compute(some_long_state_value);\n+    compute_fast(some_long_state_value);\n+    compute_fast(some_long_state_value);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let f0 = 0;\n    let f1 = 0;\n    let f2 = 0;\n    let f3 = 0;\n    let f4 = 0;\n    let f5 = 0;\n    let f6 = 0;\n    let f7 = 0;\n    let a = run(dir.path(), &[\"--patch\", patch]);\n    let b = run(dir.path(), &[\"--patch\", \"\"]);\n    let g0 = 0;\n    let g1 = 0;\n    let g2 = 0;\n    let g3 = 0;\n    let g4 = 0;\n    let g5 = 0;\n    let g6 = 0;\n    let g7 = 0;\n    compute(some_long_state_value);\n    compute(some_long_state_value);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- run(D,"), "got: {out}");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            out.contains("- compute(some_long_state_value)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn method_calls_divergent_new_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    a.bar(1);\n+    b.baz(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn non_rust_file_stays_passthrough() {
        let raw = section("notes.txt", "@@ -1 +1 @@\n-foo\n+bar\n");
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn deletion_only_stays_passthrough() {
        let raw = section("f.rs", "@@ -1,3 +1,2 @@\n fn m() {\n-    foo(1);\n }\n");
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn new_file_section_stays_passthrough() {
        let raw = section(
            "f.rs",
            "new file mode 100644\n--- /dev/null\n+++ b/f.rs\n@@ -0,0 +1,1 @@\n+fn n() {}\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn free_slot_sweep_heterogeneous_compresses() {
        // Heterogeneous vis/qual across sites, rename-only per site.
        // Long headers: the slot pattern beats the ordinary lines
        // (T375 guard); `foo`/`bar`-scale headers stay ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n-async fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n+fn new_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n+async fn new_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n    1\n}\nasync fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n    2\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains("# visibility V\n"), "got: {out}");
        assert!(!out.contains("# qualifier_list Q\n"), "got: {out}");
        assert!(
            out.contains("- V Q fn old_function_name G (...) W {\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ V Q fn new_function_name G (...) W {\n"),
            "got: {out}"
        );
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
        assert!(
            !out.contains("impl Svc"),
            "free sweep needs no impl wrapper, got: {out}"
        );
    }

    #[test]
    fn short_slot_sweep_fires_as_diet() {
        // WI-1e diet (no slot decls, `# sweep`): the short slot sweep
        // now wins T375 (byte-guard flip pinned).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn foo(x: i32) -> i32 {\n-async fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n+async fn bar(x: i32) -> i32 {\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("diet wins T375");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# sweep\n"), "got: {out}");
        assert!(!out.contains("# visibility"), "got: {out}");
    }

    #[test]
    fn method_slot_sweep_compresses_with_impl_wrapper() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n impl T {\n-    fn old_method_name(&self, first_argument: FirstLongType) {\n-    pub fn old_method_name(&self, first_argument: FirstLongType) {\n+    fn new_method_name(&self, first_argument: FirstLongType) {\n+    pub fn new_method_name(&self, first_argument: FirstLongType) {\n     }\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "impl T {\n    fn old_method_name(&self, first_argument: FirstLongType) {\n        1\n    }\n    pub fn old_method_name(&self, first_argument: FirstLongType) {\n        2\n    }\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("impl Svc {\n"), "got: {out}");
        assert!(
            out.contains("-     V Q fn old_method_name G (...) W {\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+     V Q fn new_method_name G (...) W {\n"),
            "got: {out}"
        );
    }

    #[test]
    fn mixed_indent_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) {\n-    fn foo(&self) {\n+fn bar(x: i32) {\n+    fn bar(&self) {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn divergent_renames_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) {\n-fn foo(y: i32) {\n+fn bar(x: i32) {\n+fn baz(y: i32) {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn param_edit_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) {\n-fn foo(y: i32) {\n+fn bar(x: u32) {\n+fn bar(y: i32) {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn generic_single_stays_passthrough() {
        // N >= 2 only: a lone heterogeneous header costs more as a
        // slot hunk than as an ordinary hunk.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo<T>(x: T) {\n+fn bar<T>(x: T) {\n     1\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn rule_names_unique_across_files() {
        let a = section(
            "a.rs",
            "@@ -1,4 +1,4 @@\n-    old_function(argument1);\n-    old_function(argument2);\n-    old_function(argument3);\n+    new_function(argument1);\n+    new_function(argument2);\n+    new_function(argument3);\n",
        );
        let b = section(
            "b.rs",
            "@@ -1,4 +1,4 @@\n-    old_function(argument1);\n-    old_function(argument2);\n-    old_function(argument3);\n+    new_function(argument1);\n+    new_function(argument2);\n+    new_function(argument3);\n",
        );
        let pre = pre_map(&[
            ("a.rs", "fn m() {\n    old_function(argument1);\n    old_function(argument2);\n    old_function(argument3);\n}\n"),
            ("b.rs", "fn m() {\n    old_function(argument1);\n    old_function(argument2);\n    old_function(argument3);\n}\n"),
        ]);
        let out = infer_semantic(&[a, b]).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("rule=r2\n"), "got: {out}");
    }

    #[test]
    fn per_hunk_sweep_compresses_beside_ordinary_hunk() {
        // Mixed file: hunk 1 is a 3-site call sweep, hunk 2 a body
        // edit. The whole-section detectors stay passthrough; the
        // per-hunk fallback (T349) compresses hunk 1 and keeps hunk 2
        // ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    foo(argument1);\n-    foo(argument2);\n-    foo(argument3);\n+    bar(argument1);\n+    bar(argument2);\n+    bar(argument3);\n }\n@@ -10,3 +10,3 @@\n fn n() {\n-    1\n+    2\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    foo(argument1);\n    foo(argument2);\n    foo(argument3);\n}\n// pad6\n// pad7\n// pad8\n// pad9\nfn n() {\n    1\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
        assert!(out.contains("@@ -10,3 +10,3 @@\n"), "got: {out}");
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn per_hunk_unbalanced_hunk_stays_ordinary() {
        // A pure-addition hunk beside a balanced sweep: the addition
        // stays ordinary, the sweep still compresses (T349).
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    foo(argument1);\n-    foo(argument2);\n-    foo(argument3);\n+    bar(argument1);\n+    bar(argument2);\n+    bar(argument3);\n }\n@@ -6,0 +7,2 @@\n+fn added() {}\n+// new\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    foo(argument1);\n    foo(argument2);\n    foo(argument3);\n}\n// tail\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+fn added() {}\n"), "got: {out}");
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn per_hunk_mixed_file_without_sweep_stays_passthrough() {
        // No compressible hunk: `None`, so the caller keeps the
        // ordinary passthrough (T349).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    1\n+    2\n }\n@@ -10,3 +10,3 @@\n fn n() {\n-    3\n+    4\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_basic_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::Thing;\n- use crate::old::Other;\n+ use crate::new::Thing;\n+ use crate::new::Other;\n",
        );
        let pre = pre_map(&[("f.rs", "use crate::old::Thing;\nuse crate::old::Other;\n")]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
        assert!(out.contains("@@ -1,2 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_heterogeneous_tails_compress() {
        // Plain, grouped, glob, `as`, and `pub use` tails share one
        // hunk — the engine preserves tails verbatim (SPEC 14.9).
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n- use crate::old::Thing;\n- use crate::old::{A, B};\n- use crate::old::*;\n- use crate::old::Widget as W;\n- pub use crate::old::Reexport;\n+ use crate::new::Thing;\n+ use crate::new::{A, B};\n+ use crate::new::*;\n+ use crate::new::Widget as W;\n+ pub use crate::new::Reexport;\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "use crate::old::Thing;\nuse crate::old::{A, B};\nuse crate::old::*;\nuse crate::old::Widget as W;\npub use crate::old::Reexport;\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
        assert!(!out.contains('{'), "pattern must stay bare, got: {out}");
        assert!(out.contains("@@ -1,5 @@ rule=r1\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_pub_normalized() {
        // Leading `pub`/`pub(…)` is normalized away — mixed vis sites
        // share one hunk.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n- use crate::old::A;\n- pub use crate::old::B;\n- pub(crate) use crate::old::C;\n+ use crate::new::A;\n+ pub use crate::new::B;\n+ pub(crate) use crate::new::C;\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "use crate::old::A;\npub use crate::old::B;\npub(crate) use crate::old::C;\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -1,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_leading_colons_tolerated() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use ::crate::old::A;\n- use ::crate::old::B;\n+ use ::crate::new::A;\n+ use ::crate::new::B;\n",
        );
        let pre = pre_map(&[("f.rs", "use ::crate::old::A;\nuse ::crate::old::B;\n")]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_single_site_stays_passthrough() {
        // A lone pair costs more as a semantic hunk than ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n- use crate::old::Thing;\n+ use crate::new::Thing;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_single_segment_stays_passthrough() {
        // `use foo` matches too broadly to infer from N lines of
        // evidence — v1 needs a two-or-more-segment prefix.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use foo::A;\n- use foo::B;\n+ use bar::A;\n+ use bar::B;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_parent_mismatch_stays_passthrough() {
        // Shared last segment under divergent parents is not a tight
        // rename delta.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::A;\n- use crate::old::B;\n+ use other::old::A;\n+ use other::old::B;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_divergent_tail_stays_passthrough() {
        // Prefix-only changes keep per-site tails byte-equal.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::A;\n- use crate::old::B;\n+ use crate::new::A;\n+ use crate::new::C;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_vis_only_stays_passthrough() {
        // No ident delta — equal prefixes with equal tails.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::x::A;\n- use crate::x::B;\n+ pub use crate::x::A;\n+ pub use crate::x::B;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_non_use_stays_passthrough() {
        // `useful::x` is an ident prefix, not the `use` keyword; bare
        // calls never parse as use paths.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::A;\n- useful::x;\n+ use crate::new::A;\n+ useful::y;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_spaced_separators_normalized() {
        // Spaces around `::` are trimmed per segment; the emitted
        // pattern is the canonical `::`-joined prefix.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate :: old :: A;\n- use crate :: old :: B;\n+ use crate::new::A;\n+ use crate::new::B;\n",
        );
        let pre = pre_map(&[("f.rs", "use crate :: old :: A;\nuse crate :: old :: B;\n")]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
    }

    #[test]
    fn strip_field_label_edge_cases() {
        // T417: T416 shipped the helper with integration tests only;
        // pin the guards directly (the `::` guard is load-bearing).
        assert_eq!(
            strip_field_label("text: normalize(&sub)"),
            Some(("text", "normalize(&sub)"))
        );
        assert_eq!(strip_field_label("a::b()"), None);
        assert_eq!(strip_field_label("::foo: bar"), None);
        assert_eq!(strip_field_label("\"http://x\""), None);
        assert_eq!(strip_field_label("text:"), None);
        assert_eq!(strip_field_label("text : foo()"), Some(("text", "foo()")));
        assert_eq!(strip_field_label("_t1: foo()"), Some(("_t1", "foo()")));
    }
}
