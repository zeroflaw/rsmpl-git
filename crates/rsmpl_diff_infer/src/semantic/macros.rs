//! Macro-rename sweep + N=1 single (WI-3). Split from `super` per the
//! D5 hygiene gate. Two shapes, infer-only (the engine already matches
//! them — probed green): N >= 2 single macro invocations `old!(...)` ->
//! `new!(...)` with per-site token-equal bodies (bodies vary across
//! sites — the bench-17 hetero shape), and the N=1 lone rename as an
//! allow-one verbatim single under the precision guard (WI-3 slice 3:
//! `macro_holds` at n=1 plus the scratch byte-compare is the uniqueness
//! proof; holed authored legs unify via the D17 verbatim arm).
//! Delimiters are unrestricted (the engine `...` matches `!()`, `![]`,
//! and `!{}` alike, probed). Macro *interiors* stay ordinary (WI-9): a
//! per-site body rewrite is not a rename.

use super::macro_parts::{
    delim_close, is_string_arg, macro_delim_open, macro_path, macro_raw_args, member_sites,
    parse_macro_expr, parse_macro_interior_pair, parse_macro_pair, parse_nested_macro_pair,
    scan_arg_idents, split_macro_path, split_top_spans, trim_range,
};
use super::prov::Counted;
use super::unique_rule;

/// N-site (N >= 2) macro-rename sweep: every pair parses as
/// `old!(body)` -> `new!(body)` (bare) or `prefix::old!(body)` ->
/// `prefix::new!(body)` (qualified, one uniform class per hunk) with one
/// shared old/new macro path, per-site token-equal bodies, and a real
/// last-segment name delta. Emits one `# allow=many`
/// `# macro=token_tree` hunk with the verbatim `...` body (token-tree
/// matching is verbatim-only by design, T133): every `old!` invocation
/// rewrites regardless of body or delimiter.
pub(crate) fn try_macro_rename_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() < 2 {
        return Counted::Miss;
    }
    let mut old_path: Option<String> = None;
    let mut new_path: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some((o, n)) = parse_macro_pair(m, p) else {
            return Counted::Miss;
        };
        if old_path.as_ref().is_some_and(|x| *x != o) {
            return Counted::Miss;
        }
        if new_path.as_ref().is_some_and(|x| *x != n) {
            return Counted::Miss;
        }
        old_path = Some(o);
        new_path = Some(n);
    }
    let (old_path, new_path) = match (old_path, new_path) {
        (Some(o), Some(n)) => (o, n),
        _ => return Counted::Miss,
    };
    if old_path == new_path {
        return Counted::Miss;
    }
    // Last-segment delta shared across the class; the prefix (possibly
    // empty for bare) must match segment-wise or the class is hetero.
    let (o_pre, o_l) = match split_macro_path(&old_path) {
        Some(v) => v,
        None => return Counted::Miss,
    };
    let (n_pre, n_l) = match split_macro_path(&new_path) {
        Some(v) => v,
        None => return Counted::Miss,
    };
    if o_pre != n_pre || o_l == n_l {
        return Counted::Miss;
    }
    let (old_last, _) = (o_l, n_l);
    // WI-4 slice 2b-1: counted decline names `totality:`/`veto:`.
    let verdict = if o_pre.is_empty() {
        crate::totality_counted::macro_verdict(file_old, &old_last, false, minuses, changed)
    } else {
        crate::totality_counted::macro_verdict(file_old, &old_path, true, minuses, changed)
    };
    match verdict {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let flat = |p: &str| p.replace("::", "_");
    let rule = unique_rule(
        used_rules,
        &format!("macros_{}_to_{}", flat(&old_path), flat(&new_path)),
    );
    // WI-17a: rename + hole — one varying stable position across sites
    // rides a diet hole (`old_m!(E0, 1)` → `new_m!(E0, 1)`) when the
    // holed pair proves its own real-pair totality (tighter wins). Any
    // veto (arity, strings, multi-hole, per-site deltas, unmatched twin)
    // falls back to the verbatim `...` below, whose own probe already
    // passed above.
    if let Some((holed_minus, holed_plus)) =
        rename_hole_candidate(&old_path, &new_path, minuses, pluses)
    {
        if crate::totality::template_pair_holds(
            path,
            file_old,
            &holed_minus,
            &holed_plus,
            minuses.len(),
        ) {
            let mut hunk = crate::emission_header(path, &rule, changed);
            hunk.push_str("# macro=token_tree\n");
            hunk.push_str(&format!("- {holed_minus}\n+ {holed_plus}\n"));
            return Counted::Fire(hunk);
        }
    }
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str("# macro=token_tree\n");
    hunk.push_str(&format!("- {old_path}!(...)\n+ {new_path}!(...)\n"));
    Counted::Fire(hunk)
}

/// N=1 macro-rename single (WI-3 slice 3): one `old!(body)` →
/// `new!(body)` invocation pair with a shared prefix, a last-segment
/// name delta, and token-equal bodies compresses to the allow-one
/// verbatim pair (`- old!(...)` / `+ new!(...)` under
/// `# macro=token_tree`, no allow/scope directive — the WI-3 slice-1
/// use-prefix single precedent). Uniqueness is proved by
/// `macro_holds` / `qualified_macro_holds` at n=1 (exactly one `old`
/// invocation in the file, opaque macro/string interiors veto) plus
/// the `scratch_single_reproduces` byte-compare pre-reservation over
/// the actual emission. The holed diet is never emitted (the N≥2
/// `rename_hole_template` needs ≥2 sites by construction); a holed
/// authored leg (fixture 86) unifies via the D17 verbatim equality
/// arm, same posture as the WI-1s1 holed singles. Same-name interior
/// edits (fixture 75) decline here (`parse_macro_pair` returns equal
/// paths — the interior family owns them).
pub(crate) fn try_macro_rename_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return super::prov::Counted::Miss;
    }
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    let Some((old_path, new_path)) = parse_macro_pair(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    if old_path == new_path {
        return super::prov::Counted::Miss;
    }
    let Some((o_pre, o_last)) = split_macro_path(&old_path) else {
        return super::prov::Counted::Miss;
    };
    let Some((n_pre, n_last)) = split_macro_path(&new_path) else {
        return super::prov::Counted::Miss;
    };
    if o_pre != n_pre || o_last == n_last {
        return super::prov::Counted::Miss;
    }
    if o_pre.is_empty() {
        match crate::totality_counted::macro_verdict(file_old, &o_last, false, minuses, changed) {
            crate::totality_counted::FamilyVerdict::Holds => {}
            crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
                return super::prov::Counted::Totality { found, want };
            }
            crate::totality_counted::FamilyVerdict::OpaqueHit => {
                return super::prov::Counted::Opaque;
            }
            crate::totality_counted::FamilyVerdict::Unproven => {
                return super::prov::Counted::Miss;
            }
        }
    } else {
        match crate::totality_counted::macro_verdict(file_old, &old_path, true, minuses, changed) {
            crate::totality_counted::FamilyVerdict::Holds => {}
            crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
                return super::prov::Counted::Totality { found, want };
            }
            crate::totality_counted::FamilyVerdict::OpaqueHit => {
                return super::prov::Counted::Opaque;
            }
            crate::totality_counted::FamilyVerdict::Unproven => {
                return super::prov::Counted::Miss;
            }
        }
    }
    let checkpoint = used_rules.len();
    let flat = |p: &str| p.replace("::", "_");
    let rule = unique_rule(
        used_rules,
        &format!("macros_{}_to_{}", flat(&old_path), flat(&new_path)),
    );
    // Allow-one: ranged hunk with no `# allow` directive (the
    // use-prefix single precedent); `# macro=token_tree` stays (the
    // engine needs it — paren `...` matches every delimiter, probed).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str("# macro=token_tree\n");
    hunk.push_str(&format!("- {old_path}!(...)\n+ {new_path}!(...)\n"));
    // Defensive: the probes above already applied equivalent
    // spellings, but the hunk header is new — never emit a guess.
    if rsmpl_patch::parse_patch(&hunk).is_err() {
        used_rules.truncate(checkpoint);
        return super::prov::Counted::Miss;
    }
    if !macro_single_reproduces(pre, path, &hunk, changed, pluses) {
        used_rules.truncate(checkpoint);
        return super::prov::Counted::Miss;
    }
    super::prov::Counted::Fire(hunk)
}

/// T361 byte-compare probe for the N=1 macro-rename single (the
/// slot.rs `scratch_single_reproduces` precedent, file-local like the
/// trait_methods.rs copy): the allow-one emission must real-apply to
/// the pre-image byte-identical to the native single-line swap.
/// `changed` cites the exact site; every other line stays verbatim.
fn macro_single_reproduces(
    pre: &str,
    path: &str,
    section: &str,
    changed: &[usize],
    pluses: &[&str],
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    let post = &files[path];
    let pre_lines: Vec<&str> = pre.lines().collect();
    let post_lines: Vec<&str> = post.lines().collect();
    if post_lines.len() != pre_lines.len() {
        return false;
    }
    let changed_set: std::collections::HashSet<usize> = changed.iter().copied().collect();
    for (i, &site) in changed.iter().enumerate() {
        let Some(post_line) = post_lines.get(site.saturating_sub(1)) else {
            return false;
        };
        if post_line.trim() != pluses[i].trim() {
            return false;
        }
    }
    for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        if changed_set.contains(&(idx + 1)) {
            continue;
        }
        if pre_line != post_line {
            return false;
        }
    }
    true
}

/// WI-17a rename + hole candidate: uniform delimiter across all sites
/// plus [`super::macro_hetero::rename_hole_template`]. `None` for mixed
/// delimiters (the verbatim `...` matches every delimiter, so it keeps
/// the mixed cell) or any template veto.
fn rename_hole_candidate(
    old_path: &str,
    new_path: &str,
    minuses: &[&str],
    pluses: &[&str],
) -> Option<(String, String)> {
    let mut open: Option<char> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let o_mac = parse_macro_expr(m)?;
        let n_mac = parse_macro_expr(p)?;
        let o_open = macro_delim_open(&o_mac)?;
        let n_open = macro_delim_open(&n_mac)?;
        if o_open != n_open {
            return None;
        }
        if open.as_ref().is_some_and(|o| *o != o_open) {
            return None;
        }
        open = Some(o_open);
    }
    let lines: Vec<(&str, &str)> = minuses
        .iter()
        .zip(pluses.iter())
        .map(|(m, p)| (*m, *p))
        .collect();
    super::macro_hetero::rename_hole_template(old_path, new_path, open?, &lines)
}

/// One N-site (N ≥ 2) same-name concrete macro-interior group: the
/// token-normalized grouping key, the raw-spelled emission pair, the
/// member pair indices, and the covered old-side sites. Rules are
/// reserved after the atomicity fixed point below, so vetoed
/// candidates never burn `rN` names.
struct InteriorGroup {
    key: (String, String),
    minus: String,
    plus: String,
    members: Vec<usize>,
    sites: Vec<usize>,
}

/// Same-name concrete macro-interior sweep (WI-9a): N ≥ 2 balanced
/// whole-line pairs rendering to one identical `(minus, plus)` macro
/// pair — same rendered path, same delimiter (paren, bracket, or
/// brace — WI-9b2-delim), body delta, no holes —
/// plus WI-9b2b nested groups (sub-line pairs the whole-line parse
/// declines whose lines differ only inside one same-delimiter
/// same-path nested macro; concrete-only, no hetero, no stranded
/// claims). Cross-hunk driver mirroring the peel section skeleton plus
/// the verbatim stranded join (exactly-1-minus pure unbalanced hunks
/// whose first plus matches shed minus + first plus and stay a
/// pure-insertion leftover), so sections with a pure addition (bench 18
/// shape) still compress their runs. Emission is the concrete
/// invocation pair under `# allow=many` (plain `!` routing, probed —
/// no `# macro` directive); totality is the generic engine-count
/// `template_expr_holds` on the rendered minus (the engine's own match
/// domain: macro-body/string twins veto exactly as the engine sees
/// them). Mixed-delimiter pairs stay ordinary (no cross-delimiter
/// class); same-delimiter interiors splice delimiter-exact, as do
/// renames via the delimiter-exact path splice.
/// Runs before verbatim in both section paths (verbatim still vetoes
/// every macro, so no double-claim); emitted bodies are single-pattern
/// `@@ -1,1 @@` hunks the later verbatim pass groups at n=1 and
/// passes through unchanged (peel-skeleton precedent).
pub(crate) fn try_macro_interior_section(
    section: &str,
    path: &str,
    used_rules: &mut Vec<String>,
    file_old: Option<&str>,
    fired: &mut Vec<(String, &'static str)>,
) -> Option<String> {
    let (prelude, hunks) = super::peel::split_section_hunks(section);
    if hunks.is_empty() {
        return None;
    }
    let hunk_changed: Vec<Vec<usize>> = hunks
        .iter()
        .map(|h| crate::totality::changed_old_lines(h))
        .collect();
    // Balanced-hunk pairs in order; unbalanced hunks can only join via
    // the stranded claim below. Every old-side line feeds nothing here
    // (totality is the engine-count probe, not a textual needle).
    let mut pairs: Vec<(usize, String, String)> = Vec::new();
    let mut pair_old: Vec<usize> = Vec::new();
    let mut hunk_balanced = vec![false; hunks.len()];
    let mut hunk_pure = vec![true; hunks.len()];
    let mut hunk_minuses: Vec<Vec<String>> = Vec::with_capacity(hunks.len());
    let mut hunk_pluses: Vec<Vec<String>> = Vec::with_capacity(hunks.len());
    for (i, hunk) in hunks.iter().enumerate() {
        let mut minuses = Vec::new();
        let mut pluses = Vec::new();
        let mut pure = true;
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            } else {
                // Context/blank/marker lines: not `-U0`-pure, ineligible
                // to join (verbatim T441 precedent).
                pure = false;
            }
        }
        if minuses.len() == pluses.len() && !minuses.is_empty() {
            hunk_balanced[i] = true;
            for (k, (m, p)) in minuses
                .iter()
                .cloned()
                .zip(pluses.iter().cloned())
                .enumerate()
            {
                if m.trim() == p.trim() {
                    continue;
                }
                let old_line = hunk_changed[i].get(k).copied().unwrap_or(0);
                pairs.push((i, m, p));
                pair_old.push(old_line);
            }
        }
        hunk_pure[i] = pure;
        hunk_minuses.push(minuses);
        hunk_pluses.push(pluses);
    }
    // Group identical rendered interior pairs, first-occurrence order.
    let mut classes: Vec<((String, String), Vec<usize>)> = Vec::new();
    for (idx, (.., m, p)) in pairs.iter().enumerate() {
        let Some(key) = parse_macro_interior_pair(m, p) else {
            continue;
        };
        match classes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(idx),
            None => classes.push((key, vec![idx])),
        }
    }
    // WI-9b2b: sub-line nested-macro extraction (bench-17 `tool.rs`
    // shape) — pairs the whole-line parse declines whose lines differ
    // only inside one paren-delimited same-path nested macro.
    // First-occurrence order after the whole-line classes.
    let mut nested_classes: Vec<((String, String), Vec<usize>)> = Vec::new();
    for (idx, (.., m, p)) in pairs.iter().enumerate() {
        if parse_macro_interior_pair(m, p).is_some() {
            continue;
        }
        let Some(key) = parse_nested_macro_pair(m, p) else {
            continue;
        };
        match nested_classes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(idx),
            None => nested_classes.push((key, vec![idx])),
        }
    }
    let checkpoint = used_rules.len();
    // Stranded-join claims (verbatim T441 slice): at most one claim per
    // unbalanced hunk (exactly 1 minus, pure, first plus renders into
    // the group); claims release when the group vetoes.
    let mut joined: Vec<Option<(String, String)>> = vec![None; hunks.len()];
    let mut passing: Vec<InteriorGroup> = Vec::new();
    for ((minus, plus), members) in &classes {
        // Claim stranded mates for this group (verbatim T441 slice):
        // unbalanced pure hunks with exactly 1 minus whose first plus
        // renders into the group. Lone-pair groups claim at most one
        // mate (the break below); multi-pair groups claim every match.
        let mut n_eff = members.len();
        for j in 0..hunks.len() {
            if hunk_balanced[j] || !hunk_pure[j] || joined[j].is_some() {
                continue;
            }
            if hunk_minuses[j].len() != 1 || hunk_pluses[j].is_empty() {
                continue;
            }
            let m0 = &hunk_minuses[j][0];
            let p0 = &hunk_pluses[j][0];
            if parse_macro_interior_pair(m0, p0).as_ref() == Some(&(minus.clone(), plus.clone())) {
                joined[j] = Some((minus.clone(), plus.clone()));
                n_eff += 1;
                if members.len() < 2 {
                    break;
                }
            }
        }
        if n_eff < 2 {
            continue;
        }
        // Emission spells the pair with the first site's raw argument
        // text (byte-faithful, original spacing) instead of the
        // token-normalized grouping key (`out.rsmpl` renders as
        // `out . rsmpl` through `TokenStream` display — still
        // engine-matchable, but noisier and T375-poorer). WI-9b1 hetero:
        // stable non-string args hole to shared `E{i}` when the shorter
        // hetero pair passes its real-pair totality probe (self-rewrite
        // would infer literal uppercase deltas as holes and count a more
        // general domain — `template_pair_holds` runs the emitted hunk's
        // own diet inference instead); else the concrete probe decides.
        // WI-9b2a: concrete probes the real pair too (same soundness —
        // the self probe holed shared uppercase deltas and vetoed on
        // second-arg twins a sound concrete group must ignore).
        let (m0, p0) = (&pairs[members[0]].1, &pairs[members[0]].2);
        let (concrete_minus, concrete_plus) = emit_pair(m0, p0, minus, plus);
        let hetero = hetero_pair(m0, p0).filter(|(hm, hp)| {
            hm.len() + hp.len() < concrete_minus.len() + concrete_plus.len()
                && crate::totality::template_pair_holds(path, file_old, hm, hp, n_eff)
        });
        let (emit_minus, emit_plus) = match hetero {
            Some(pair) => pair,
            None => {
                if !crate::totality::template_pair_holds(
                    path,
                    file_old,
                    &concrete_minus,
                    &concrete_plus,
                    n_eff,
                ) {
                    // Release this group's claims: the hunks stay ordinary.
                    for claim in joined.iter_mut() {
                        if claim
                            .as_ref()
                            .is_some_and(|(gm, gp)| gm == minus && gp == plus)
                        {
                            *claim = None;
                        }
                    }
                    continue;
                }
                (concrete_minus, concrete_plus)
            }
        };
        // WI-1b: precise emission sites are the member pair old lines
        // plus joined stranded old lines for this group.
        let mut sites = member_sites(members, &pair_old);
        for (j, claim) in joined.iter().enumerate() {
            if claim
                .as_ref()
                .is_some_and(|(gm, gp)| gm == minus && gp == plus)
            {
                let old_line = hunk_changed[j].first().copied().unwrap_or(0);
                if !sites.contains(&old_line) {
                    sites.push(old_line);
                }
            }
        }
        // WI-2: the rule is reserved after the atomicity fixed point
        // below (vetoed candidates never burn `rN` names); commit
        // filters by presence in the final text, so stale records never
        // surface.
        passing.push(InteriorGroup {
            key: (minus.clone(), plus.clone()),
            minus: emit_minus,
            plus: emit_plus,
            members: members.clone(),
            sites,
        });
    }
    // WI-9b2b nested groups: concrete-only (no hetero — nested string
    // args never hole and hetero-nested is deferred to WI-9b2), no
    // stranded claims; same InteriorGroup shape so member sites, the
    // atomicity fixed point below, T375, and ranged-vs-sweep emission
    // are shared with the whole-line groups above.
    for ((minus, plus), members) in &nested_classes {
        if members.len() < 2 {
            continue;
        }
        let n_eff = members.len();
        if !crate::totality::template_pair_holds(path, file_old, minus, plus, n_eff) {
            continue;
        }
        passing.push(InteriorGroup {
            key: (minus.clone(), plus.clone()),
            minus: minus.clone(),
            plus: plus.clone(),
            members: members.clone(),
            sites: member_sites(members, &pair_old),
        });
    }
    // WI-9b2 cross-site hetero groups: leftover balanced pairs (not
    // claimed by any passing identical/nested group) keyed by
    // (path, delimiter, arity); the template holes stable positions
    // that vary across sites while the literal delta stays shared.
    // No stranded claims (nested precedent); per-group byte sanity
    // keeps a losing candidate from flipping the section-global T375
    // into a whole-sweep veto.
    {
        let claimed: std::collections::HashSet<usize> = passing
            .iter()
            .flat_map(|g| g.members.iter().copied())
            .collect();
        let mut xclasses: Vec<((String, char, usize), Vec<usize>)> = Vec::new();
        for (idx, (.., m, p)) in pairs.iter().enumerate() {
            if claimed.contains(&idx) {
                continue;
            }
            let Some(key) = super::macro_hetero::hetero_group_key(m, p) else {
                continue;
            };
            match xclasses.iter_mut().find(|(k, _)| *k == key) {
                Some((_, members)) => members.push(idx),
                None => xclasses.push((key, vec![idx])),
            }
        }
        for (_, members) in &xclasses {
            if members.len() < 2 {
                continue;
            }
            let lines: Vec<(&str, &str)> = members
                .iter()
                .map(|&i| (pairs[i].1.as_str(), pairs[i].2.as_str()))
                .collect();
            let Some((hm, hp)) = super::macro_hetero::hetero_group_template(&lines) else {
                continue;
            };
            let raw_len: usize = members
                .iter()
                .map(|&i| pairs[i].1.len() + pairs[i].2.len())
                .sum();
            if hm.len() + hp.len() + 40 >= raw_len {
                continue;
            }
            if !crate::totality::template_pair_holds(path, file_old, &hm, &hp, members.len()) {
                continue;
            }
            // The template always carries ≥1 fresh hole (the builder
            // vetoes hole-free groups), so this key — unlike the
            // concrete identical/nested keys above — can never collide
            // with a stranded `joined` claim: victim-release below
            // only ever clears concrete claims.
            passing.push(InteriorGroup {
                key: (hm.clone(), hp.clone()),
                minus: hm,
                plus: hp,
                members: members.clone(),
                sites: member_sites(members, &pair_old),
            });
        }
    }
    // Hunk-atomicity fixed point (aa23c77 soundness): a group fires only
    // if every balanced hunk holding a member pair is fully covered by
    // passing groups — otherwise the group hunk and the leftover
    // ordinary hunk would rewrite the same line (GIT_HUNK_CONFLICT
    // "overlapping edits cannot be merged"). Drop violators (releasing
    // claims) to a fixed point. Coverage is by member index (whole-line
    // identical, nested, and cross-site hetero groups share one
    // member set — cross-site members share no single render key, so
    // key re-derivation cannot see them; index sets are equivalent
    // for the keyed groups because classes collect every pair with
    // their key).
    loop {
        let covered: std::collections::HashSet<usize> = passing
            .iter()
            .flat_map(|g| g.members.iter().copied())
            .collect();
        let mut victim: Option<usize> = None;
        'scan: for (gi, g) in passing.iter().enumerate() {
            for &idx in &g.members {
                let hj = pairs[idx].0;
                for (jj, (hj2, _, _)) in pairs.iter().enumerate() {
                    if *hj2 != hj {
                        continue;
                    }
                    if !covered.contains(&jj) {
                        victim = Some(gi);
                        break 'scan;
                    }
                }
            }
        }
        let Some(gi) = victim else {
            break;
        };
        let key = passing[gi].key.clone();
        for claim in joined.iter_mut() {
            if claim.as_ref().is_some_and(|c| *c == key) {
                *claim = None;
            }
        }
        passing.remove(gi);
    }
    if passing.is_empty() {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Reserve rules + WI-2 detector records for survivors only.
    let mut fired_groups: Vec<(InteriorGroup, String)> = Vec::new();
    for g in passing {
        let rule = unique_rule(used_rules, &format!("macro_interior_{}", g.minus));
        fired.push((rule.clone(), "macro_interior"));
        fired_groups.push((g, rule));
    }
    let passing: Vec<(InteriorGroup, String)> = fired_groups;
    // Leftover insertions for joined unbalanced hunks: each claimed hunk
    // sheds its minus + first plus and stays a pure insertion. Claims of
    // vetoed groups were released above, so every surviving claim belongs
    // to a passing group; a header that does not split shape-exactly
    // vetoes the whole sweep (fail safe).
    let mut split_leftovers: Vec<Option<(String, String, String)>> = vec![None; hunks.len()];
    for (j, claim) in joined.iter().enumerate() {
        let Some((gm, gp)) = claim else { continue };
        if !passing
            .iter()
            .any(|(g, _)| g.key.0 == *gm && g.key.1 == *gp)
        {
            continue;
        }
        let Some(leftover) = super::verbatim::split_leftover_insertion(&hunks[j]) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        split_leftovers[j] = Some((gm.clone(), gp.clone(), leftover));
    }
    // A hunk is covered iff balanced and every pair sits in a group
    // (identical, nested, or cross-site — one member-index domain).
    let covered: std::collections::HashSet<usize> = passing
        .iter()
        .flat_map(|(g, _)| g.members.iter().copied())
        .collect();
    let covered_hunk: Vec<bool> = hunk_balanced
        .iter()
        .enumerate()
        .map(|(i, b)| {
            *b && pairs
                .iter()
                .enumerate()
                .filter(|(_, (j, _, _))| *j == i)
                .all(|(jj, _)| covered.contains(&jj))
        })
        .collect();
    // WI-1b ranged bodies per group when spans are disjoint and ≤64;
    // overlapping groups share the sweep dummy (SPEC 10.6).
    let mut spans: Vec<Option<(usize, usize)>> = Vec::new();
    for (g, _) in &passing {
        if g.sites.contains(&0) {
            spans.push(None);
            continue;
        }
        let min = g.sites.iter().min().copied().unwrap_or(0);
        let max = g.sites.iter().max().copied().unwrap_or(0);
        if min >= 1 && max.saturating_sub(min).saturating_add(1) <= 64 {
            spans.push(Some((min, max)));
        } else {
            spans.push(None);
        }
    }
    let mut overlap = vec![false; passing.len()];
    for a in 0..spans.len() {
        for b in (a + 1)..spans.len() {
            if let (Some((a0, a1)), Some((b0, b1))) = (spans[a], spans[b]) {
                if a0 <= b1 && b0 <= a1 {
                    overlap[a] = true;
                    overlap[b] = true;
                }
            }
        }
    }
    let bodies: Vec<String> = passing
        .iter()
        .enumerate()
        .map(|(gi, (g, rule))| {
            if overlap[gi] || spans[gi].is_none() {
                format!(
                    "@@ -1,1 @@ rule={rule}\n# sweep\n- {}\n+ {}\n",
                    g.minus, g.plus
                )
            } else {
                format!(
                    "{}- {}\n+ {}\n",
                    crate::emission_body(rule, &g.sites),
                    g.minus,
                    g.plus
                )
            }
        })
        .collect();
    // T375 over the covered set, joined leftovers counted both sides
    // (verbatim precedent: the fallback would emit the whole hunk, so
    // the joined bytes join the guard on both sides).
    let covered_len: usize = hunks
        .iter()
        .zip(&covered_hunk)
        .filter(|(_, b)| **b)
        .map(|(h, _)| super::normalized_hunk_len(h))
        .sum();
    let joined_orig_len: usize = hunks
        .iter()
        .enumerate()
        .filter(|(i, _)| split_leftovers[*i].is_some())
        .map(|(_, h)| super::normalized_hunk_len(h))
        .sum();
    let leftover_len: usize = split_leftovers
        .iter()
        .flatten()
        .map(|(_, _, s)| s.len())
        .sum();
    if bodies.iter().map(|b| b.len()).sum::<usize>() + leftover_len >= covered_len + joined_orig_len
    {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Emit each group's hunk at its first covered or joined hunk;
    // split insertions and ordinary hunks stay in place.
    let mut out = prelude;
    let mut emitted = vec![false; passing.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        let holds_pair = |(g, _): &(InteriorGroup, String)| {
            pairs
                .iter()
                .enumerate()
                .any(|(jj, (j, _, _))| *j == i && g.members.contains(&jj))
                || split_leftovers[i]
                    .as_ref()
                    .is_some_and(|(gm, gp, _)| *gm == g.key.0 && *gp == g.key.1)
        };
        if covered_hunk[i] || split_leftovers[i].is_some() {
            for (gi, g) in passing.iter().enumerate() {
                if !emitted[gi] && holds_pair(g) {
                    out.push_str(&bodies[gi]);
                    emitted[gi] = true;
                }
            }
            if let Some((_, _, leftover)) = &split_leftovers[i] {
                out.push_str(leftover);
            }
            continue;
        }
        out.push_str(hunk);
    }
    Some(out)
}

/// Hetero (per-arg-hole) emission for an identical interior class
/// (WI-9b1): same group as the concrete path, but every stable
/// (minus==plus, trimmed) non-string arg longer than its hole name
/// becomes a shared `E{i}` hole (`E0` first, bumped past idents occurring
/// in surviving literals or the path so SPEC 15.7 equality can never fuse
/// a hole to a literal). Delta positions stay literal with first-site raw
/// spelling (separators preserved verbatim, so literal bytes are exactly
/// the evidence bytes). Returns `None` when nothing holes (hetero ==
/// concrete), arity diverges, or spans unbalance — the caller falls back
/// to concrete.
fn hetero_pair(minus_evidence: &str, plus_evidence: &str) -> Option<(String, String)> {
    let o_mac = parse_macro_expr(minus_evidence)?;
    let n_mac = parse_macro_expr(plus_evidence)?;
    let o_path = macro_path(&o_mac)?;
    let n_path = macro_path(&n_mac)?;
    if o_path != n_path {
        return None;
    }
    let (Some(o_open), Some(n_open)) = (macro_delim_open(&o_mac), macro_delim_open(&n_mac)) else {
        return None;
    };
    if o_open != n_open {
        return None;
    }
    let m_args = macro_raw_args(minus_evidence)?;
    let p_args = macro_raw_args(plus_evidence)?;
    let m_spans = split_top_spans(&m_args)?;
    let p_spans = split_top_spans(&p_args)?;
    if m_spans.len() != p_spans.len() || m_spans.is_empty() {
        return None;
    }
    // Occupied hole names: every ident in the path and in ANY arg slice
    // (holed or not). A hole name never coincides with a literal ident, so
    // SPEC 15.7 equality can never fuse a hole to a literal — including
    // positions demoted back to literal below (their idents already block).
    let mut occupied: Vec<String> = scan_arg_idents(&o_path);
    for ((ms, me), (ps, pe)) in m_spans.iter().zip(p_spans.iter()) {
        occupied.extend(scan_arg_idents(m_args[*ms..*me].trim()));
        occupied.extend(scan_arg_idents(p_args[*ps..*pe].trim()));
    }
    let mut next_hole = 0u32;
    let mut fresh = |used: &mut Vec<String>| loop {
        let name = if next_hole == 0 {
            "E0".to_string()
        } else {
            format!("E{next_hole}")
        };
        next_hole += 1;
        if !used
            .iter()
            .any(|u| u == &name || u.trim_start_matches('$') == name)
        {
            used.push(name.clone());
            return name;
        }
    };
    let mut holed_any = false;
    // Rebuild inner text group by group, preserving separators verbatim:
    // each group slice is replaced whole (hole) or kept byte-exact.
    let mut m_rebuilt = String::new();
    let mut p_rebuilt = String::new();
    let mut m_cursor = 0;
    let mut p_cursor = 0;
    for ((ms, me), (ps, pe)) in m_spans.iter().zip(p_spans.iter()) {
        let (m_g, p_g) = (m_args[*ms..*me].trim(), p_args[*ps..*pe].trim());
        m_rebuilt.push_str(&m_args[m_cursor..*ms]);
        p_rebuilt.push_str(&p_args[p_cursor..*ps]);
        // Hole only when strictly shorter (byte-win): the `> 2` pre-check
        // keeps demotions from consuming hole names (the post-`fresh`
        // length check only fires at `E10`+, where a skipped name is
        // harmless — idents already block). The hole replaces just the
        // trimmed group text so surrounding spacing stays byte-exact.
        if m_g == p_g && !is_string_arg(m_g) && m_g.len() > 2 {
            let hole = fresh(&mut occupied);
            if m_g.len() > hole.len() {
                let (mts, mte) = trim_range(&m_args, *ms, *me);
                let (pts, pte) = trim_range(&p_args, *ps, *pe);
                m_rebuilt.push_str(&m_args[*ms..mts]);
                m_rebuilt.push_str(&hole);
                m_rebuilt.push_str(&m_args[mte..*me]);
                p_rebuilt.push_str(&p_args[*ps..pts]);
                p_rebuilt.push_str(&hole);
                p_rebuilt.push_str(&p_args[pte..*pe]);
                holed_any = true;
                m_cursor = *me;
                p_cursor = *pe;
                continue;
            }
        }
        m_rebuilt.push_str(&m_args[*ms..*me]);
        p_rebuilt.push_str(&p_args[*ps..*pe]);
        m_cursor = *me;
        p_cursor = *pe;
    }
    m_rebuilt.push_str(&m_args[m_cursor..]);
    p_rebuilt.push_str(&p_args[p_cursor..]);
    if !holed_any {
        return None;
    }
    let close = delim_close(o_open);
    Some((
        format!("{o_path}!{o_open}{m_rebuilt}{close}"),
        format!("{n_path}!{n_open}{p_rebuilt}{close}"),
    ))
}

/// Spell an interior pair with the first site's raw argument text
/// (original spacing) instead of the token-normalized grouping key.
/// Falls back to the normalized pair when the raw slice fails (never
/// expected — both lines parsed — but emission must never go loud on
/// a rendering detail).
fn emit_pair(
    minus_evidence: &str,
    plus_evidence: &str,
    norm_minus: &str,
    norm_plus: &str,
) -> (String, String) {
    let fallback = || (norm_minus.to_string(), norm_plus.to_string());
    let (Some(o_mac), Some(n_mac)) = (
        parse_macro_expr(minus_evidence),
        parse_macro_expr(plus_evidence),
    ) else {
        return fallback();
    };
    let (Some(o_path), Some(n_path)) = (macro_path(&o_mac), macro_path(&n_mac)) else {
        return fallback();
    };
    if o_path != n_path {
        return fallback();
    }
    let (Some(o_open), Some(n_open)) = (macro_delim_open(&o_mac), macro_delim_open(&n_mac)) else {
        return fallback();
    };
    if o_open != n_open {
        return fallback();
    }
    let (Some(o_args), Some(n_args)) = (
        macro_raw_args(minus_evidence),
        macro_raw_args(plus_evidence),
    ) else {
        return fallback();
    };
    let close = delim_close(o_open);
    (
        format!("{o_path}!{o_open}{o_args}{close}"),
        format!("{n_path}!{n_open}{n_args}{close}"),
    )
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic;
    use super::super::macro_parts::{is_string_arg, split_top_spans};
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
    fn macro_rename_sweep_compresses() {
        // Hetero bodies (bench-17 shape): verbatim could never group
        // these (count 1 each) — the rename sweep claims them in one hunk.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument, extra_flag);\n-    old_render_widget!(third_widget_argument + offset);\n+    new_render_widget!(first_widget_argument);\n+    new_render_widget!(second_widget_argument, extra_flag);\n+    new_render_widget!(third_widget_argument + offset);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_render_widget!(first_widget_argument);\n    old_render_widget!(second_widget_argument, extra_flag);\n    old_render_widget!(third_widget_argument + offset);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("# macro=token_tree\n"), "got: {out}");
        assert!(out.contains("- old_render_widget!(...)\n"), "got: {out}");
        assert!(out.contains("+ new_render_widget!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_mixed_delimiters_fire() {
        // The engine `...` matches `!()`, `![]`, `!{}` alike (probed) —
        // a delimiter-mixed rename still compresses.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget![second_widget_argument];\n+    new_render_widget!(first_widget_argument);\n+    new_render_widget![second_widget_argument];\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_render_widget!(first_widget_argument);\n    old_render_widget![second_widget_argument];\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_render_widget!(...)\n"), "got: {out}");
        assert!(out.contains("+ new_render_widget!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_let_wrapped_fires() {
        // `let x = old!(a);` matches `- old!(...)` (probed) — the sweep
        // claims let-wrapped invocation renames too.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = old_render_widget!(first_widget_argument);\n-    let second_value = old_render_widget!(second_widget_argument);\n+    let first_value = new_render_widget!(first_widget_argument);\n+    let second_value = new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_value = old_render_widget!(first_widget_argument);\n    let second_value = old_render_widget!(second_widget_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_render_widget!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_body_rewrite_stays_passthrough() {
        // A per-site body rewrite is an interior edit (WI-9), not a rename.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument);\n+    old_render_widget!(changed_first_argument);\n+    old_render_widget!(changed_second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_divergent_new_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument);\n+    new_render_widget!(first_widget_argument);\n+    other_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_single_site_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n+    new_render_widget!(first_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn wi3_macro_verbatim_let_single_stays_ordinary() {
        // WI-3 slice 5: a same-callee literal change inside a whole-line
        // `let` with a macro init is an interior edit, not a rename --
        // and the whole-line shape matches nothing in the engine
        // (T440 `verbatim_let_shape`/`init_ok` wall), so no detector
        // fires. Needs engine literal-statement matching with macro
        // inits; until then the single stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n fn f() {\n-    let b = format!(\"x{}\", 1);\n+    let b = format!(\"y{}\", 1);\n     let _ = b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_rename_single_fires_bare_verbatim() {
        // WI-3 slice 3 (fixture 09 shape): one bare rename with a
        // token-equal body raises as the allow-one verbatim single.
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@ fn a() {\n-    old_mac!(1, foo(x));\n+    new_mac!(1, foo(x));\n }\n",
        );
        let pre = pre_map(&[("f.rs", "fn a() {\n    old_mac!(1, foo(x));\n}\n")]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_mac!(...)\n"), "got:\n{out}");
        assert!(out.contains("+ new_mac!(...)\n"), "got:\n{out}");
        assert!(out.contains("# macro=token_tree\n"), "got:\n{out}");
        assert!(
            !out.contains("allow"),
            "allow-one carries no allow directive, got:\n{out}"
        );
    }

    #[test]
    fn macro_rename_single_vetoes_twin_and_same_path() {
        // An untouched same-name twin vetoes (count 2 at n=1).
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@ fn f() {\n-    old_m!(a, 1);\n+    new_m!(a, 1);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    old_m!(a, 1);\n    old_m!(b, 2);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none(),
            "twin must veto"
        );
        // Same-name interior edits are not renames (fixture 75 posture).
        let pre_same = pre_map(&[("f.rs", "fn f() {\n    let b = format!(\"x{}\", 1);\n}\n")]);
        assert!(
            super::try_macro_rename_single(
                &["    let b = format!(\"x{}\", 1);"],
                &["    let b = format!(\"y{}\", 1);"],
                "f.rs",
                Some(&pre_same["f.rs"]),
                &[2],
                &mut Vec::new(),
            )
            .is_miss(),
            "same-path interior must decline"
        );
        // Prefix mismatch (two renamed segments) stays ordinary.
        assert!(
            super::try_macro_rename_single(
                &["    a::old!(1);"],
                &["    b::new!(1);"],
                "f.rs",
                Some("fn f() {\n    a::old!(1);\n}\n"),
                &[2],
                &mut Vec::new(),
            )
            .is_miss(),
            "prefix mismatch must decline"
        );
    }

    #[test]
    fn macro_rename_single_fires_qualified() {
        // Qualified renames ride the same arm via `qualified_macro_holds`.
        let pre = pre_map(&[("f.rs", "fn f() {\n    crate::old_m!(a);\n}\n")]);
        let out = super::try_macro_rename_single(
            &["    crate::old_m!(a);"],
            &["    crate::new_m!(a);"],
            "f.rs",
            Some(&pre["f.rs"]),
            &[2],
            &mut Vec::new(),
        )
        .unwrap_fire("qualified single must fire");
        assert!(out.contains("- crate::old_m!(...)\n"), "got:\n{out}");
        assert!(out.contains("+ crate::new_m!(...)\n"), "got:\n{out}");
        assert!(out.contains("# macro=token_tree\n"), "got:\n{out}");
    }

    #[test]
    fn macro_extra_site_vetoes() {
        // A third untouched `old!` in the pre-image would also match the
        // `...` pattern — totality vetoes.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument);\n+    new_render_widget!(first_widget_argument);\n+    new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_render_widget!(first_widget_argument);\n    old_render_widget!(second_widget_argument);\n    old_render_widget!(untouched_third_argument);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn macro_qualified_twin_does_not_veto() {
        // Exact-path matching (probed): `crate::old!` never matches the
        // bare pattern — the twin must not veto.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument);\n+    new_render_widget!(first_widget_argument);\n+    new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_render_widget!(first_widget_argument);\n    old_render_widget!(second_widget_argument);\n    crate::old_render_widget!(qualified_twin_argument);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn macro_qualified_path_fires() {
        // WI-3 residue: qualified macro renames compress like bare ones
        // (probed: the engine matches the concrete qualified path).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_render_widget!(first_widget_argument);\n    crate::old_render_widget!(second_widget_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- crate::old_render_widget!(...)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ crate::new_render_widget!(...)\n"),
            "got: {out}"
        );
        assert!(out.contains("# macro=token_tree\n"), "got: {out}");
    }

    #[test]
    fn macro_qualified_delimiter_mix_fires() {
        // The engine `...` matches all delimiters for qualified paths too.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget![second_widget_argument];\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget![second_widget_argument];\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        assert!(
            out.contains("- crate::old_render_widget!(...)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn macro_qualified_let_wrapped_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = crate::old_render_widget!(first_widget_argument);\n-    let second_value = crate::old_render_widget!(second_widget_argument);\n+    let first_value = crate::new_render_widget!(first_widget_argument);\n+    let second_value = crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        assert!(
            out.contains("- crate::old_render_widget!(...)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn macro_qualified_prefix_divergent_stays_passthrough() {
        // Different prefixes share no path class.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    other::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    other::new_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_qualified_last_same_stays_passthrough() {
        // Same last segment is not a rename.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    other::old_render_widget!(first_widget_argument);\n+    other::old_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_bare_twin_does_not_veto_qualified() {
        // Exact-path parity both directions: a bare twin never matches
        // the qualified pattern (probed) so it must not veto.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_render_widget!(first_widget_argument);\n    crate::old_render_widget!(second_widget_argument);\n    old_render_widget!(bare_twin_argument);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn macro_qualified_extra_site_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_render_widget!(first_widget_argument);\n    crate::old_render_widget!(second_widget_argument);\n    crate::old_render_widget!(untouched_third_argument);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn macro_rename_hole_fires() {
        // WI-17a (fixture-48 shape): rename + one varying stable arg
        // emits the holed pair instead of the verbatim `...` sweep.
        // The engine replays it byte-exact (validated_live).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_m!(a, 1);\n-    old_m!(b, 1);\n+    new_m!(a, 1);\n+    new_m!(b, 1);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_m!(a, 1);\n    old_m!(b, 1);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_m!(E0, 1)\n"), "got: {out}");
        assert!(out.contains("+ new_m!(E0, 1)\n"), "got: {out}");
        assert!(out.contains("# macro=token_tree\n"), "got: {out}");
    }

    #[test]
    fn macro_rename_hole_twin_vetoes_to_ordinary() {
        // An untouched `old_m!(c, 1)` twin binds the hole too: the holed
        // probe counts 3 ≠ 2 and the `...` probe counts 3 ≠ 2, so the
        // section stays ordinary (no half-holed emission).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_m!(a, 1);\n-    old_m!(b, 1);\n+    new_m!(a, 1);\n+    new_m!(b, 1);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_m!(a, 1);\n    old_m!(b, 1);\n    old_m!(c, 1);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn macro_rename_hole_qualified_fires() {
        // WI-17a qualified leg: the same hole under a shared prefix.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_m!(alpha_value, key);\n-    crate::old_m!(beta_value, key);\n+    crate::new_m!(alpha_value, key);\n+    crate::new_m!(beta_value, key);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_m!(alpha_value, key);\n    crate::old_m!(beta_value, key);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- crate::old_m!(E0, key)\n"), "got: {out}");
        assert!(out.contains("+ crate::new_m!(E0, key)\n"), "got: {out}");
    }

    #[test]
    fn macro_rename_two_varying_stays_verbatim() {
        // Multi-hole renames keep the verbatim `...` cell (tighter
        // emission deferred): still one semantic hunk, no holes.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_m!(alpha_value, first_flag);\n-    old_m!(beta_value, second_flag);\n+    new_m!(alpha_value, first_flag);\n+    new_m!(beta_value, second_flag);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_m!(alpha_value, first_flag);\n    old_m!(beta_value, second_flag);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_m!(...)\n"), "got: {out}");
        assert!(out.contains("+ new_m!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_rename_mixed_delimiter_stays_verbatim() {
        // Mixed delimiters keep the verbatim cell (one delimiter per
        // holed template; `...` matches all three, probed).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_m!(a, 1);\n-    old_m![b, 1];\n+    new_m!(a, 1);\n+    new_m![b, 1];\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_m!(a, 1);\n    old_m![b, 1];\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_m!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_rename_interior_delta_stays_ordinary() {
        // Rename + per-site body delta is neither the rename sweep
        // (bodies differ) nor the interior sweep (names differ).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_m!(a, 1);\n-    old_m!(b, 1);\n+    new_m!(a, 2);\n+    new_m!(b, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_rule_body_interior_fires() {
        // WI-17b: the engine now mints rule-body candidates and the
        // rule-body detector raises the inner pair.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n macro_rules! foo {\n-    ($x:expr) => { inner!(1); };\n-    ($y:expr) => { inner!(1); };\n+    ($x:expr) => { inner!(2); };\n+    ($y:expr) => { inner!(2); };\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "macro_rules! foo {\n    ($x:expr) => { inner!(1); };\n    ($y:expr) => { inner!(1); };\n}\n",
        )]);
        let out = super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("rule-body interiors must fire post WI-17b");
        validated_live(&out, &pre);
        assert!(out.contains("- inner!(1)\n"), "got: {out}");
        assert!(out.contains("+ inner!(2)\n"), "got: {out}");
    }

    #[test]
    fn macro_leading_colon_stays_passthrough() {
        // Leading `::` has no engine spelling (probed) — declined.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    ::crate::old_render_widget!(first_widget_argument);\n-    ::crate::old_render_widget!(second_widget_argument);\n+    ::crate::new_render_widget!(first_widget_argument);\n+    ::crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_mixed_bare_qualified_stays_passthrough() {
        // Bare + qualified share no path class.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_uppercase_fires_plain() {
        // WI-7-upper: macro names never infer (no `# macro` hole for
        // the name — probed clean), so uppercase renames stay plain:
        // no backticks, real-apply byte-exact.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    Old!(first_argument);\n-    Old!(second_argument);\n+    New!(first_argument);\n+    New!(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    Old!(first_argument);\n    Old!(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains('`'), "no backticks expected, got: {out}");
        assert!(out.contains("- Old!(...)\n"), "got: {out}");
        assert!(out.contains("+ New!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_qualified_uppercase_fires_plain() {
        // WI-7-upper: qualified twin of the above — the whole path
        // stays verbatim (probed clean, names never infer).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::Old!(first_argument);\n-    crate::Old!(second_argument);\n+    crate::New!(first_argument);\n+    crate::New!(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::Old!(first_argument);\n    crate::Old!(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains('`'), "no backticks expected, got: {out}");
        assert!(out.contains("- crate::Old!(...)\n"), "got: {out}");
        assert!(out.contains("+ crate::New!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_interior_fires() {
        // WI-9b1: two identical same-name interior pairs compress to one
        // hetero `# allow=many` hunk (plain `!` routing, no `# macro`
        // directive, no `# expr` — diet infers the shared `E0` via the
        // both-sides rule while single-side `TEXT_*` stay literal deltas).
        // The engine replays it byte-exact (validated_live).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# macro"), "plain routing, got: {out}");
        assert!(!out.contains("# expr"), "diet, got: {out}");
        assert!(
            out.contains("- assert_eq!(E0, TEXT_SECTION)\n"),
            "hetero hole, got: {out}"
        );
        assert!(
            out.contains("+ assert_eq!(E0, TEXT_STRIPPED)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn macro_interior_qualified_fires() {
        // Same-name qualified interiors share the uniform-class rule:
        // exact rendered path, one group.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::check!(first_value);\n-    crate::check!(first_value);\n+    crate::check!(second_value);\n+    crate::check!(second_value);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::check!(first_value);\n    crate::check!(first_value);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- crate::check!"), "got: {out}");
        assert!(out.contains("+ crate::check!"), "got: {out}");
    }

    #[test]
    fn macro_interior_let_wrapped_fires() {
        // `let`-wrapped evidence emits the bare macro pattern (T449
        // precedent — the pattern shape is `NAME!(...)`, never `let`).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = same_mac!(1);\n-    let second_value = same_mac!(1);\n+    let first_value = same_mac!(2);\n+    let second_value = same_mac!(2);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_value = same_mac!(1);\n    let second_value = same_mac!(1);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- same_mac!(1)\n"), "got: {out}");
        assert!(out.contains("+ same_mac!(2)\n"), "got: {out}");
    }

    #[test]
    fn macro_interior_single_site_stays_passthrough() {
        // N=1 never pays (T375 + the N≥2 gate).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_interior_rename_divergent_stays_passthrough() {
        // Name delta + body delta is neither rename (bodies differ) nor
        // interior (names differ) — stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_mac!(first_value);\n-    old_mac!(first_value);\n+    new_mac!(second_value);\n+    new_mac!(second_value);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_interior_bracket_fires() {
        // WI-9b2-delim: same-delimiter bracket interiors compress — the
        // engine splice emits with the plus pattern's delimiter, so
        // `![...]` round-trips delimiter-exact (validated_live).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    my_vec![1, 2];\n-    my_vec![1, 2];\n+    my_vec![1, 3];\n+    my_vec![1, 3];\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    my_vec![1, 2];\n    my_vec![1, 2];\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- my_vec![1, 2]\n"), "got: {out}");
        assert!(out.contains("+ my_vec![1, 3]\n"), "got: {out}");
    }

    #[test]
    fn macro_interior_brace_fires() {
        // WI-9b2-delim: brace interiors compress delimiter-exact too.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    my_map!{a: 1};\n-    my_map!{a: 1};\n+    my_map!{a: 2};\n+    my_map!{a: 2};\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    my_map!{a: 1};\n    my_map!{a: 1};\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- my_map!{a: 1}\n"), "got: {out}");
        assert!(out.contains("+ my_map!{a: 2}\n"), "got: {out}");
    }

    #[test]
    fn macro_interior_mixed_delimiter_stays_passthrough() {
        // No cross-delimiter class: `!(..)` vs `![..]` stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    my_vec!(1, 2);\n-    my_vec!(1, 2);\n+    my_vec![1, 3];\n+    my_vec![1, 3];\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_interior_extra_site_vetoes() {
        // A third untouched `assert_eq!(out.rsmpl, TEXT_SECTION)` in the
        // pre-image would also match the concrete minus — totality vetoes.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn macro_interior_partial_hunk_vetoes() {
        // aa23c77 soundness (hunk atomicity): a hunk mixing a repeated
        // pair (`move_big`, grouped elsewhere) with a singleton
        // (`move_one`, no group) stays fully ordinary — emitting the
        // group hunk alongside the leftover ordinary hunk would rewrite
        // the shared line twice (GIT_HUNK_CONFLICT "overlapping edits
        // cannot be merged").
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    assert!(text.contains(\"rule=move_big\"), \"got\");\n-    assert!(text.contains(\"rule=move_one\"), \"got\");\n+    assert!(text.contains(\"rule=r1\"), \"got\");\n+    assert!(text.contains(\"rule=r1\"), \"got\");\n }\n@@ -6,2 +6,2 @@\n fn n() {\n-    assert!(text.contains(\"rule=move_big\"), \"got\");\n-    other_call_site();\n+    assert!(text.contains(\"rule=r1\"), \"got\");\n+    other_call_site();\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    assert!(text.contains(\"rule=move_big\"), \"got\");\n    assert!(text.contains(\"rule=move_one\"), \"got\");\n}\nfn n() {\n    assert!(text.contains(\"rule=move_big\"), \"got\");\n    other_call_site();\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn macro_interior_stranded_join_fires() {
        // Bench-18 shape: one balanced pair plus one unbalanced hunk
        // (minus + first-plus match, pure-insertion tail) join to N=2;
        // the tail stays an ordinary insertion.
        let raw = section(
            "f.rs",
            "@@ -6,1 +6,1 @@\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n@@ -10,1 +10,2 @@\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n+fn extra_tail() {}\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha() {}\nfn beta() {}\nfn gamma() {}\nfn delta() {}\nfn check_one() {\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n}\nfn epsilon() {}\nfn check_two() {\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
        assert!(
            out.contains("- assert_eq!(E0, TEXT_SECTION)\n"),
            "hetero hole, got: {out}"
        );
        assert!(
            out.contains("+fn extra_tail() {}"),
            "tail stays, got: {out}"
        );
    }

    #[test]
    fn macro_hetero_string_arg_stays_concrete() {
        // String-literal-only rule: a stable string arg never holes even
        // when longer than the hole name — the pair stays concrete.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    log!(long_stable_prefix, \"a,b\");\n-    log!(long_stable_prefix, \"a,b\");\n+    log!(long_stable_prefix, \"c\");\n+    log!(long_stable_prefix, \"c\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    log!(long_stable_prefix, \"a,b\");\n    log!(long_stable_prefix, \"a,b\");\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- log!(E0, \"a,b\")\n"),
            "stable non-string holes, string stays, got: {out}"
        );
        assert!(out.contains("+ log!(E0, \"c\")\n"), "got: {out}");
    }

    #[test]
    fn macro_hetero_short_stable_stays_literal() {
        // Byte-win gate: a stable arg no longer than its hole name
        // (`1` vs `E0`) stays literal — hetero degrades to concrete.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = same_mac!(1);\n-    let second_value = same_mac!(1);\n+    let first_value = same_mac!(2);\n+    let second_value = same_mac!(2);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_value = same_mac!(1);\n    let second_value = same_mac!(1);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- same_mac!(1)\n"), "got: {out}");
        assert!(!out.contains("E0"), "no holes expected, got: {out}");
    }

    #[test]
    fn macro_hetero_twin_falls_back_to_concrete() {
        // An untouched `assert_eq!(other_value, TEXT_SECTION)` matches the
        // hetero minus (`E0` binds anything) but not the tighter concrete
        // minus — the real-pair probe counts 3 ≠ 2 and vetoes hetero, the
        // concrete self-probe still counts exactly 2, so the sweep fires
        // concrete instead of staying ordinary (precision-first fallback).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n-    assert_eq!(out.rsmpl, TEXT_SECTION);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n+    assert_eq!(out.rsmpl, TEXT_STRIPPED);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n    assert_eq!(out.rsmpl, TEXT_SECTION);\n    assert_eq!(other_value, TEXT_SECTION);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- assert_eq!(out.rsmpl, TEXT_SECTION)\n"),
            "concrete fallback, got: {out}"
        );
        assert!(!out.contains("E0"), "hetero vetoed, got: {out}");
    }

    #[test]
    fn macro_concrete_second_arg_twin_fires() {
        // WI-9b2a: a twin with its own delta no longer vetoes the concrete
        // group when hetero is absent (the only stable arg `x` is shorter
        // than any hole name, so `hetero_pair` is `None` and the concrete
        // fallback decides alone). The old self-rewrite probe (`- M / + M`)
        // saw `FIRST_DELTA` on both sides, holed it, and counted
        // `check!(x-lit, <any>)` (3 ≠ 2 → ordinary); the real-pair probe
        // keeps the single-side delta literal, so the twin's `THIRD_DELTA`
        // never matches (2 == 2 → fires concrete). A/B-pinned: fails on
        // the old probe, passes on the new.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    check!(x, FIRST_DELTA);\n-    check!(x, FIRST_DELTA);\n+    check!(x, SECOND_DELTA);\n+    check!(x, SECOND_DELTA);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    check!(x, FIRST_DELTA);\n    check!(x, FIRST_DELTA);\n    check!(x, THIRD_DELTA);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- check!(x, FIRST_DELTA)\n"),
            "concrete recovery, got: {out}"
        );
        assert!(out.contains("+ check!(x, SECOND_DELTA)\n"), "got: {out}");
        assert!(!out.contains("E0"), "hetero vetoed, got: {out}");
    }

    #[test]
    fn macro_hetero_hole_name_freshness() {
        // A literal `E0` arg blocks the name: the holed neighbor takes
        // `E1`, so SPEC 15.7 equality can never fuse hole to literal.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    check!(E0, stable_longer_argument, FIRST_DELTA);\n-    check!(E0, stable_longer_argument, FIRST_DELTA);\n+    check!(E0, stable_longer_argument, SECOND_DELTA);\n+    check!(E0, stable_longer_argument, SECOND_DELTA);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    check!(E0, stable_longer_argument, FIRST_DELTA);\n    check!(E0, stable_longer_argument, FIRST_DELTA);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- check!(E0, E1, FIRST_DELTA)\n"),
            "fresh hole, got: {out}"
        );
        assert!(
            out.contains("+ check!(E0, E1, SECOND_DELTA)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn split_top_spans_string_aware() {
        // Commas/parens inside strings split nothing (T428 precedent).
        let spans = split_top_spans("long_stable_prefix, \"a,b\"").unwrap();
        assert_eq!(spans.len(), 2);
        let spans = split_top_spans("foo(bar(1, 2)), \"c,d\", e").unwrap();
        assert_eq!(spans.len(), 3);
        // Unbalanced delimiters bail (hetero falls back to concrete).
        assert!(split_top_spans("foo(bar").is_none());
        assert!(split_top_spans("").unwrap().is_empty());
    }

    #[test]
    fn is_string_arg_prefixes() {
        assert!(is_string_arg("\"a,b\""));
        assert!(is_string_arg("r\"a,b\""));
        assert!(is_string_arg("b\"x\""));
        assert!(!is_string_arg("TEXT_SECTION"));
        assert!(!is_string_arg("out.rsmpl"));
        assert!(!is_string_arg("'a'"));
    }

    #[test]
    fn macro_nested_fires() {
        // WI-9b2b (bench-17 `tool.rs` shape): two identical `let`
        // statements differing only inside the nested `env!` macro
        // compress to one concrete `# allow=many` hunk spelling just the
        // macro pair (the engine rewrites nested occurrences in place —
        // probed). The minus/plus lines are complete statements the
        // verbatim sweep vetoes (macro inside the init), and the
        // whole-line interior parse declines (the init is a Call).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n-    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n+    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n+    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# macro"), "plain routing, got: {out}");
        assert!(
            out.contains("- env!(\"CARGO_BIN_EXE_gitrs\")\n"),
            "nested minus missing, got: {out}"
        );
        assert!(
            out.contains("+ env!(\"CARGO_BIN_EXE_applyrs\")\n"),
            "nested plus missing, got: {out}"
        );
        assert!(!out.contains("E0"), "concrete, no holes, got: {out}");
    }

    #[test]
    fn macro_nested_extra_site_vetoes() {
        // A third untouched `env!("CARGO_BIN_EXE_gitrs")` in the
        // pre-image also matches the nested minus — totality vetoes.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n-    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n+    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n+    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n    let third_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn macro_nested_multi_delta_stays_passthrough() {
        // Two differing nested macros in one line pair is not one
        // interior — stays ordinary (array init keeps the whole-line
        // interior parse out: the init is not a macro invocation).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let both = [env!(\"OLD_A\"), env!(\"OLD_B\")];\n-    let both = [env!(\"OLD_A\"), env!(\"OLD_B\")];\n+    let both = [env!(\"NEW_A\"), env!(\"NEW_B\")];\n+    let both = [env!(\"NEW_A\"), env!(\"NEW_B\")];\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_nested_outside_delta_stays_passthrough() {
        // The lines must differ ONLY inside the nested macro: a
        // simultaneous `let`-binding rename keeps the pair ordinary
        // (the macro hunk plus a leftover whole-line hunk would rewrite
        // the shared line twice).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n-    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n+    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n+    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_nested_bracket_fires() {
        // WI-9b2-delim: a bracket delta nested in a call arg takes the
        // nested path (the whole-line parse declines — the `let` init is
        // a call, not a macro) and splices delimiter-exact. Multi-token
        // bodies render from raw source bytes (WI-9b2 nested-raw), so
        // only whitespace-variant spellings across sites group apart.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = wrap(my_vec![7]);\n-    let second_value = wrap(my_vec![7]);\n+    let first_value = wrap(my_vec![8]);\n+    let second_value = wrap(my_vec![8]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_value = wrap(my_vec![7]);\n    let second_value = wrap(my_vec![7]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- my_vec![7]\n"), "got: {out}");
        assert!(out.contains("+ my_vec![8]\n"), "got: {out}");
    }

    #[test]
    fn macro_nested_multi_token_fires() {
        // WI-9b2 nested-raw: a multi-token bracket delta nested in a
        // call arg renders from raw source bytes (TokenStream display
        // would be 1 , 2 and never occur byte-exact in the line), so
        // the pair compresses to one concrete hunk, delimiter-exact.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = wrap(my_vec![1, 2]);\n-    let second_value = wrap(my_vec![1, 2]);\n+    let first_value = wrap(my_vec![1, 3]);\n+    let second_value = wrap(my_vec![1, 3]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_value = wrap(my_vec![1, 2]);\n    let second_value = wrap(my_vec![1, 2]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- my_vec![1, 2]\n"), "got: {out}");
        assert!(out.contains("+ my_vec![1, 3]\n"), "got: {out}");
        assert!(!out.contains("E0"), "concrete, no holes, got: {out}");
    }

    #[test]
    fn macro_raw_text_preserves_spacing() {
        // The raw-span helper slices the trimmed line by span: unusual
        // spacing survives, unlike TokenStream display (which would
        // render 1 , 2 with spaces around the comma).
        let line = "    let v = wrap(my_vec![1,  2]);";
        let nodes = super::super::macro_parts::line_macros(line).expect("line must parse");
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            super::super::macro_parts::macro_raw_text(line.trim(), &nodes[0]).as_deref(),
            Some("my_vec![1,  2]")
        );
    }

    #[test]
    fn macro_nested_rename_stays_passthrough() {
        // A nested path delta is a rename, not an interior — and bodies
        // differ too, so the rename sweep also declines: stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = old_mac!(first_argument);\n-    let second_value = old_mac!(first_argument);\n+    let first_value = new_mac!(second_argument);\n+    let second_value = new_mac!(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_nested_format_body_stays_ordinary() {
        // Bench-17 `lib.rs` shape: single-line fragments changed inside a
        // multi-line `format!` body. The minus line carries the macro
        // while the plus line carries none (macro-count divergent), so
        // nested extraction declines — macro-body descent is WI-9b2
        // remainder territory.
        let raw = section(
            "f.rs",
            "@@ -4 +4 @@\n fn render() {\n-        header = rsmpl_patch::collapse_git_header(&format!(\"diff --git a/{path} b/{path}\"))\n-        header = rsmpl_patch::collapse_git_header(&format!(\"diff --git a/{path} b/{path}\"))\n+        header = super::git_header(path)\n+        header = super::git_header(path)\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }
}
