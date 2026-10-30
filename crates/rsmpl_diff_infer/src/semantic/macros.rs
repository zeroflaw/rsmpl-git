//! Macro-rename sweep (WI-3). Split from `super` per the D5 hygiene
//! gate. One shape, infer-only (the engine already matches it — probed
//! green): N >= 2 single macro invocations `old!(...)` -> `new!(...)`
//! with per-site token-equal bodies (bodies vary across sites — the
//! bench-17 hetero shape). Delimiters are unrestricted (the engine
//! `...` matches `!()`, `![]`, and `!{}` alike, probed). Macro
//! *interiors* stay ordinary (WI-9): a per-site body rewrite is not a
//! rename.

use super::{ident_len, unique_rule};

/// Parse a diff line as a macro invocation: strip one terminator, take
/// the whole line as `Expr::Macro`, else unwrap a `let`-init macro
/// (T449 precedent — `let x = old!(a);` matches `- old!(...)`,
/// probed). `let-else` never qualifies.
fn parse_macro_expr(line: &str) -> Option<syn::ExprMacro> {
    let mut text = line.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = text.strip_suffix(';') {
        text = stripped.trim_end();
    }
    if let Ok(syn::Expr::Macro(mac)) = syn::parse_str::<syn::Expr>(text) {
        return Some(mac);
    }
    match syn::parse_str::<syn::Stmt>(line.trim()) {
        Ok(syn::Stmt::Local(local)) => {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            match *init.expr {
                syn::Expr::Macro(mac) => Some(mac),
                _ => None,
            }
        }
        _ => None,
    }
}

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
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut old_path: Option<String> = None;
    let mut new_path: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n) = parse_macro_pair(m, p)?;
        if old_path.as_ref().is_some_and(|x| *x != o) {
            return None;
        }
        if new_path.as_ref().is_some_and(|x| *x != n) {
            return None;
        }
        old_path = Some(o);
        new_path = Some(n);
    }
    let (old_path, new_path) = (old_path?, new_path?);
    if old_path == new_path {
        return None;
    }
    // Last-segment delta shared across the class; the prefix (possibly
    // empty for bare) must match segment-wise or the class is hetero.
    let (o_pre, o_l) = split_macro_path(&old_path)?;
    let (n_pre, n_l) = split_macro_path(&new_path)?;
    if o_pre != n_pre || o_l == n_l {
        return None;
    }
    let (old_last, _) = (o_l, n_l);
    if o_pre.is_empty() {
        if !crate::totality::macro_holds(file_old, &old_last, minuses, changed) {
            return None;
        }
    } else if !crate::totality::qualified_macro_holds(file_old, &old_path, minuses, changed) {
        return None;
    }
    let flat = |p: &str| p.replace("::", "_");
    let rule = unique_rule(
        used_rules,
        &format!("macros_{}_to_{}", flat(&old_path), flat(&new_path)),
    );
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str("# macro=token_tree\n");
    hunk.push_str(&format!("- {old_path}!(...)\n+ {new_path}!(...)\n"));
    Some(hunk)
}

/// Split a rendered macro path into (prefix segments, last segment).
fn split_macro_path(path: &str) -> Option<(Vec<String>, String)> {
    let mut segs: Vec<String> = path.split("::").map(str::to_string).collect();
    if segs.is_empty() {
        return None;
    }
    let last = segs.pop()?;
    Some((segs, last))
}

/// Parse a `-`/`+` line pair as a macro rename; return (old path, new
/// path) rendered with `::` joins when both sides are macro invocations
/// with a shared prefix, a last-segment delta, generic-free segments,
/// no leading `::`, and per-site token-equal bodies.
fn parse_macro_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o_mac = parse_macro_expr(old_line)?;
    let n_mac = parse_macro_expr(new_line)?;
    let old = macro_path(&o_mac)?;
    let new = macro_path(&n_mac)?;
    if o_mac.mac.tokens.to_string() != n_mac.mac.tokens.to_string() {
        return None;
    }
    Some((old, new))
}

/// The rendered path of a macro invocation (`old!` or `prefix::old!`);
/// `None` for leading `::` (no engine spelling, probed — declined like
/// qualified calls), segment-count mismatch is checked by the caller via
/// exact rendered-path equality, and generic args.
fn macro_path(mac: &syn::ExprMacro) -> Option<String> {
    if mac.mac.path.leading_colon.is_some() {
        return None;
    }
    let mut segs = Vec::new();
    for seg in mac.mac.path.segments.iter() {
        if !matches!(seg.arguments, syn::PathArguments::None) {
            return None;
        }
        let name = seg.ident.to_string();
        if ident_len(&name)? != name.len() {
            return None;
        }
        segs.push(name);
    }
    if segs.is_empty() {
        return None;
    }
    // Bare (len 1) and qualified (len >= 2) both qualify; the sweep
    // requires a uniform class (exact rendered-path equality above), so
    // mixed bare/qualified classes stay ordinary by construction.
    Some(segs.join("::"))
}

/// Open delimiter character of a macro invocation (`(` / `[` / `{`).
fn macro_delim_open(mac: &syn::ExprMacro) -> Option<char> {
    match mac.mac.delimiter {
        syn::MacroDelimiter::Paren(_) => Some('('),
        syn::MacroDelimiter::Bracket(_) => Some('['),
        syn::MacroDelimiter::Brace(_) => Some('{'),
    }
}

/// Close delimiter matching [`macro_delim_open`].
fn delim_close(open: char) -> char {
    match open {
        '[' => ']',
        '{' => '}',
        _ => ')',
    }
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
    // Hunk-atomicity fixed point (aa23c77 soundness): a group fires only
    // if every balanced hunk holding a member pair is fully covered by
    // passing groups — otherwise the group hunk and the leftover
    // ordinary hunk would rewrite the same line (GIT_HUNK_CONFLICT
    // "overlapping edits cannot be merged"). Drop violators (releasing
    // claims) to a fixed point. Coverage is whole-line OR nested keys
    // (WI-9b2b: the two key domains are disjoint — nested requires the
    // whole-line parse to fail — so one shared key list is exact).
    loop {
        let keys: Vec<(String, String)> = passing.iter().map(|g| g.key.clone()).collect();
        let mut victim: Option<usize> = None;
        'scan: for (gi, g) in passing.iter().enumerate() {
            for &idx in &g.members {
                let hj = pairs[idx].0;
                for (jj, m2, p2) in pairs.iter() {
                    if *jj != hj {
                        continue;
                    }
                    if !pair_key_matches(m2, p2, &keys) {
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
    // (whole-line or nested — WI-9b2b unified domain).
    let group_keys: Vec<(String, String)> = passing.iter().map(|(g, _)| g.key.clone()).collect();
    let covered_hunk: Vec<bool> = hunk_balanced
        .iter()
        .enumerate()
        .map(|(i, b)| {
            *b && pairs
                .iter()
                .filter(|(j, _, _)| *j == i)
                .all(|(_, m, p)| pair_key_matches(m, p, &group_keys))
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
        .map(|(h, _)| h.len())
        .sum();
    let joined_orig_len: usize = hunks
        .iter()
        .enumerate()
        .filter(|(i, _)| split_leftovers[*i].is_some())
        .map(|(_, h)| h.len())
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
                .any(|(j, m, p)| *j == i && pair_key_matches(m, p, std::slice::from_ref(&g.key)))
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

/// Old-side lines for group member pairs, deduped, first-occurrence
/// order (WI-1b precise emission sites, shared by whole-line and
/// WI-9b2b nested groups).
fn member_sites(members: &[usize], pair_old: &[usize]) -> Vec<usize> {
    let mut sites = Vec::new();
    for &idx in members {
        let old_line = pair_old.get(idx).copied().unwrap_or(0);
        if !sites.contains(&old_line) {
            sites.push(old_line);
        }
    }
    sites
}

/// A line pair is covered iff its whole-line OR nested interior key is
/// among the passing groups (WI-9b2b unified hunk-atomicity domain).
fn pair_key_matches(old_line: &str, new_line: &str, keys: &[(String, String)]) -> bool {
    parse_macro_interior_pair(old_line, new_line).is_some_and(|k| keys.contains(&k))
        || parse_nested_macro_pair(old_line, new_line).is_some_and(|k| keys.contains(&k))
}

/// Parse a `-`/`+` line pair as a same-name concrete macro interior:
/// both sides render (via the rename sweep's `parse_macro_expr` —
/// whole-line invocations and `let`-init unwraps, `let-else` never) to
/// one identical path with the same delimiter on both sides (paren,
/// bracket, or brace — the engine splice emits with the plus pattern's
/// delimiter, WI-9b2-delim) and a body delta. Returns the rendered
/// `(minus, plus)` invocation pair (`path!(args)` / `path![args]` /
/// `path!{args}`). `None` for renames (name delta — the rename sweep
/// owns those), mixed delimiters (no cross-delimiter class — each side
/// keeps its own spelling), leading-`::`/generic paths (no engine
/// spelling), and identical bodies (no change).
fn parse_macro_interior_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o_mac = parse_macro_expr(old_line)?;
    let n_mac = parse_macro_expr(new_line)?;
    let (Some(o_open), Some(n_open)) = (macro_delim_open(&o_mac), macro_delim_open(&n_mac)) else {
        return None;
    };
    if o_open != n_open {
        return None;
    }
    let old_path = macro_path(&o_mac)?;
    let new_path = macro_path(&n_mac)?;
    if old_path != new_path {
        return None;
    }
    let old_args = o_mac.mac.tokens.to_string();
    let new_args = n_mac.mac.tokens.to_string();
    if old_args == new_args {
        return None;
    }
    let close = delim_close(o_open);
    Some((
        format!("{old_path}!{o_open}{old_args}{close}"),
        format!("{new_path}!{n_open}{new_args}{close}"),
    ))
}

/// Expression-position macro invocations of one diff line in visit
/// (source) order. `syn::visit` never descends into macro bodies
/// (opaque token streams), so macros nested inside other macro
/// arguments are invisible by construction (WI-9b2 non-goal) — only
/// macros in real expression positions (call args, `let` inits,
/// operands) collect.
struct MacroNodes {
    out: Vec<syn::ExprMacro>,
}

impl<'ast> syn::visit::Visit<'ast> for MacroNodes {
    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        self.out.push(node.clone());
        syn::visit::visit_expr_macro(self, node);
    }
}

/// Expression-position macros of one diff line in source order, via one
/// `Stmt` parse (fallback: `Expr`). `None` when neither parses.
fn line_macros(line: &str) -> Option<Vec<syn::ExprMacro>> {
    use syn::visit::Visit as _;
    let mut out = MacroNodes { out: Vec::new() };
    if let Ok(stmt) = syn::parse_str::<syn::Stmt>(line.trim()) {
        out.visit_stmt(&stmt);
        return Some(out.out);
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(line.trim()) {
        out.visit_expr(&expr);
        return Some(out.out);
    }
    None
}

/// Sub-line nested-macro interior pair (WI-9b2b, bench-17 `tool.rs`
/// shape): the whole-line interior parse fails, but both lines parse
/// as one `Stmt` (fallback `Expr`) holding the same count (≥1) of
/// expression-position macros, all token-equal except exactly one delta
/// with the same rendered path, the same delimiter on both sides, and
/// different tokens (paren, bracket, or brace — WI-9b2-delim). The
/// rendered delta texts must each occur
/// byte-exact exactly once in their line, and the plus line must equal
/// the minus line with exactly that one substring replaced (the rest of
/// the line is byte-identical). Returns the rendered
/// `(macro_minus, macro_plus)` pair. `None` for renames (path delta —
/// the rename sweep owns whole-line renames; nested renames stay
/// ordinary), mixed delimiters (no cross-delimiter class), leading-`::`/
/// generic paths (no engine spelling), multi-delta lines, and any
/// outside-macro difference.
fn parse_nested_macro_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    // Whole-line pairs belong to the whole-line sweep (no double-claim).
    if parse_macro_interior_pair(old_line, new_line).is_some() {
        return None;
    }
    let old_macros = line_macros(old_line)?;
    let new_macros = line_macros(new_line)?;
    if old_macros.len() != new_macros.len() || old_macros.is_empty() {
        return None;
    }
    let mut delta: Option<usize> = None;
    for (i, (o, n)) in old_macros.iter().zip(new_macros.iter()).enumerate() {
        let (Some(op), Some(np)) = (macro_path(o), macro_path(n)) else {
            return None;
        };
        let (Some(o_open), Some(n_open)) = (macro_delim_open(o), macro_delim_open(n)) else {
            return None;
        };
        if op == np && o_open == n_open {
            if o.mac.tokens.to_string() == n.mac.tokens.to_string() {
                continue;
            }
            if delta.is_some() {
                return None;
            }
            delta = Some(i);
            continue;
        }
        return None;
    }
    let i = delta?;
    let (o, n) = (&old_macros[i], &new_macros[i]);
    let (Some(o_open), Some(n_open)) = (macro_delim_open(o), macro_delim_open(n)) else {
        return None;
    };
    let close = delim_close(o_open);
    let minus = format!("{}!{o_open}{}{close}", macro_path(o)?, o.mac.tokens);
    let plus = format!("{}!{n_open}{}{close}", macro_path(n)?, n.mac.tokens);
    if minus == plus {
        return None;
    }
    if old_line.match_indices(&minus).count() != 1 || new_line.match_indices(&plus).count() != 1 {
        return None;
    }
    if *new_line != old_line.replacen(&minus, &plus, 1) {
        return None;
    }
    Some((minus, plus))
}

/// Split macro inner-args text into top-level comma-group byte spans
/// (string-aware like the engine `split_top_level`: commas/parens inside
/// `"..."` split nothing). Returns `None` on unbalanced delimiters
/// (e.g. a `'('` char literal — the engine deliberately skips `'` for
/// lifetime ambiguity, T419); hetero then bails to concrete.
fn split_top_spans(body: &str) -> Option<Vec<(usize, usize)>> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if matches!(b, b'(' | b'[' | b'{') {
            depth += 1;
        } else if matches!(b, b')' | b']' | b'}') {
            depth -= 1;
            if depth < 0 {
                return None;
            }
        } else if b == b',' && depth == 0 {
            out.push((start, i));
            start = i + 1;
        }
        i += 1;
    }
    if in_str || depth != 0 {
        return None;
    }
    out.push((start, body.len()));
    // Drop empty groups (leading/trailing/double commas carry no arg —
    // engine `split_macro_args` retains the same way).
    let spans: Vec<(usize, usize)> = out
        .into_iter()
        .filter(|(s, e)| !body[*s..*e].trim().is_empty())
        .collect();
    Some(spans)
}

/// Trimmed byte range of a group span (leading/trailing whitespace excluded).
fn trim_range(s: &str, ms: usize, me: usize) -> (usize, usize) {
    let bytes = s.as_bytes();
    let mut ts = ms;
    while ts < me && bytes[ts].is_ascii_whitespace() {
        ts += 1;
    }
    let mut te = me;
    while te > ts && bytes[te - 1].is_ascii_whitespace() {
        te -= 1;
    }
    (ts, te)
}

/// True for string-literal args (`".."`, `r".."`, `b".."`, `br".."`):
/// strings are opaque positions (WI-9 string-literal-only rule) and never
/// hole, even when stable. Char literals and lifetimes hole like any other
/// non-string arg under totality.
fn is_string_arg(arg: &str) -> bool {
    let t = arg.trim();
    let u = t
        .strip_prefix("br\"")
        .or_else(|| t.strip_prefix("rb\""))
        .or_else(|| t.strip_prefix('r'))
        .or_else(|| t.strip_prefix('b'))
        .unwrap_or(t);
    u.len() >= 2 && u.starts_with('"') && u.ends_with('"')
}

/// Scan ident-like tokens (mirrors the inference candidate scan).
fn scan_arg_idents(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '$' || c.is_ascii_alphabetic() || c == '_' {
            let mut j = i + 1;
            while j < bytes.len() {
                let d = bytes[j] as char;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(text[i..j].to_string());
            i = j;
        } else {
            i += 1;
        }
    }
    out
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

/// The raw argument text between a line's macro-invocation delimiters:
/// everything after the open delimiter past `!` up to the matching
/// close (mod a trailing `;`). Exact because the line parsed as a
/// complete invocation — nothing trails the close delimiter but
/// `;`/whitespace — so interior closers inside strings cannot mislead
/// the `strip_suffix`.
fn macro_raw_args(line: &str) -> Option<String> {
    let bang = line.find('!')?;
    let after = line[bang + 1..].trim_start();
    let (open, close) = match after.chars().next()? {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        '{' => ('{', '}'),
        _ => return None,
    };
    let mut inner = after[open.len_utf8()..].trim_end();
    inner = inner.strip_suffix(';').unwrap_or(inner).trim_end();
    Some(inner.strip_suffix(close)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic;
    use super::{is_string_arg, split_top_spans};
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
        // a call, not a macro) and splices delimiter-exact. Single-token
        // bodies only: multi-token TokenStream rendering (`1 , 2`) never
        // occurs byte-exact in the line, so nested multi-token stays
        // ordinary (WI-9b2 remainder — raw-span rendering).
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
