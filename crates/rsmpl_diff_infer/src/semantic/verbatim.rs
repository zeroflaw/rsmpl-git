//! N-site verbatim identical-line sweep (T403 groups + T440 complete
//! `let` statements + T441 slice stranded-unbalanced join). Split from the
//! crate root to hold the 2000-line hygiene gate (peel.rs precedent).

use super::shape::{init_ok, probe_expr};
use super::{count_occurrences, flat_ws, strip_field_label, strip_one_terminator, unique_rule};
use crate::compress_balanced_pair_body;
use crate::totality::{changed_old_lines, covers_changed};
use syn::spanned::Spanned;
use syn::visit::Visit;

/// One N-site (N ≥ 2) verbatim identical-line group: trimmed (minus,
/// plus) pair texts, their terminator-stripped emission lines, and the
/// reserved rule name.
struct VerbatimGroup {
    minus_trim: String,
    plus_trim: String,
    minus: String,
    plus: String,
    rule: String,
    sites: Vec<usize>,
}

/// N-site verbatim identical-line sweep (T403): groups of identical
/// balanced single-line (minus, plus) pairs shared across a section's
/// hunks compress to one tight-range `# allow=many` hunk each,
/// with literal pattern lines and no metavars (the T363 use-prefix
/// precedent; probe-verified against `applyrs --check`). A stranded
/// identical pair inside an unbalanced hunk joins its group instead
/// of vetoing it (T441 slice): exactly-1-minus pure `-`/`+` hunks
/// whose first plus matches shed minus + first plus and stay as a
/// pure-insertion leftover. Whole-line
/// pairs are expressions; single-line complete `let` statements take
/// the `let` path below (T440 — the engine `Stmt` shape matches
/// concrete lets structurally, so `let out = run(..)` migrations
/// compress exactly like expression rewrites). Runs after
/// the whole-section detectors fail and before the per-hunk fallback,
/// so sections whose pure additions veto whole-section balance (e.g.
/// 50d4a49's `+ let parsed = …` lines around eleven `&state.parsed,`
/// → `parsed,` pairs) still compress their runs. Gates favor
/// precision: expression pairs strip one terminator and parse as
/// `syn::Expr` (for-loop headers, struct-field labels and literal-only
/// deltas stay ordinary); single-line complete `let` statements take
/// the `let` path (`verbatim_let_shape` — multi-line fragments like
/// b849869's `let lenient =` never parse as `Stmt` and stay ordinary);
/// the minus side is structural — bare idents
/// (`wide`) and literals would match every mention; the stripped minus
/// occurs exactly group-size times across all old-side section lines (an
/// unchanged `foo(&state.parsed)`-style site would otherwise be
/// rewritten and its ordinary hunk would fail to match). Leftover
/// balanced hunks retry the standard chain in place (fallback-identical
/// — no regression by construction); the T375 byte-guard applies to
/// the verbatim-covered set as a whole, else the section falls through
/// untouched.
pub(crate) fn try_verbatim_sweep(
    section: &str,
    path: &str,
    used_rules: &mut Vec<String>,
    file_old: Option<&str>,
    fired: &mut Vec<(String, &'static str)>,
) -> Option<String> {
    let (prelude, hunks) = split_verbatim_hunks(section);
    // T411: per-hunk changed old-side lines for the line-aware opaque veto.
    let hunk_changed: Vec<Vec<usize>> = hunks.iter().map(|h| changed_old_lines(h)).collect();
    if hunks.is_empty() {
        return None;
    }
    // Balanced-hunk pairs in order; unbalanced hunks can only be leftovers.
    // Every old-side line (minus + context) feeds the uniqueness gate.
    let mut pairs: Vec<(usize, String, String)> = Vec::new();
    let mut pair_old: Vec<usize> = Vec::new();
    let mut old_lines: Vec<String> = Vec::new();
    let mut hunk_balanced = vec![false; hunks.len()];
    // Per-hunk bodies for the stranded join (T441 slice): an unbalanced
    // hunk with exactly one minus whose first plus matches a group joins
    // the group instead of vetoing it. `pure` is false when any
    // context/blank/marker line is present (the join needs `-U0`-pure
    // bodies to split the leftover soundly).
    let mut hunk_minuses: Vec<Vec<String>> = Vec::with_capacity(hunks.len());
    let mut hunk_pluses: Vec<Vec<String>> = Vec::with_capacity(hunks.len());
    let mut hunk_pure = vec![true; hunks.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        let mut minuses = Vec::new();
        let mut pluses = Vec::new();
        let mut pure = true;
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].trim().to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].trim().to_string());
            } else if line.starts_with(' ') {
                old_lines.push(line.strip_prefix(' ').unwrap_or(line).trim().to_string());
                pure = false;
            } else {
                // Blank/empty, `\ No newline` markers, `---`/`+++`
                // markers: not `-U0`-pure, ineligible to join.
                pure = false;
            }
        }
        old_lines.extend(minuses.iter().cloned());
        if minuses.len() == pluses.len() && !minuses.is_empty() {
            hunk_balanced[i] = true;
            for (k, (m, p)) in minuses
                .iter()
                .cloned()
                .zip(pluses.iter().cloned())
                .enumerate()
            {
                let old_line = hunk_changed[i].get(k).copied().unwrap_or(0);
                pairs.push((i, m, p));
                pair_old.push(old_line);
            }
        }
        hunk_pure[i] = pure;
        hunk_minuses.push(minuses);
        hunk_pluses.push(pluses);
    }
    // Group identical pairs, first-occurrence order.
    let mut counts: Vec<(String, String, usize)> = Vec::new();
    for (_, m, p) in &pairs {
        match counts.iter_mut().find(|(gm, gp, _)| gm == m && gp == p) {
            Some((_, _, n)) => *n += 1,
            None => counts.push((m.clone(), p.clone(), 1)),
        }
    }
    let checkpoint = used_rules.len();
    // Group candidates reserve no rules: the atomicity fixed point
    // below drops violators first (macros.rs aa23c77 precedent), so
    // dropped groups burn no `rN` names and leave no stale records.
    struct Candidate {
        minus_trim: String,
        plus_trim: String,
        minus: String,
        plus: String,
        sites: Vec<usize>,
        members: Vec<usize>,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    // Pre-image parse, once per section (None = no entry: legacy
    // section-visible evidence only; Some(Err) = unparseable: veto all).
    let old_file: Option<Option<syn::File>> = file_old.map(|t| syn::parse_str::<syn::File>(t).ok());
    if matches!(old_file, Some(None)) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Stranded-join claims (T441 slice): per-hunk (group-minus,
    // group-plus) contributed beyond the balanced pairs. A hunk holds
    // at most one minus, so at most one group can claim it; claims
    // persist across groups in first-occurrence order and are released
    // when the claiming group vetoes.
    let mut joined: Vec<Option<(String, String)>> = vec![None; hunks.len()];
    for (m, p, n) in &counts {
        if *n < 2 || m == p {
            continue;
        }
        // Resolve the emission pair per shape (T440): expression pairs
        // emit the terminator-stripped lines; complete single-line
        // `let` statements emit the full `;`-terminated lines (the
        // engine `Stmt` shape parses `let` with its semicolon).
        let (Some(stripped_m), Some(stripped_p)) =
            (strip_one_terminator(m), strip_one_terminator(p))
        else {
            continue;
        };
        // (emission minus, emission plus, uniqueness needle, is-let).
        // A `let` scrutinee parses as `syn::Expr::Let`, so the Expr arm
        // below must decline `Let` on either side first — complete
        // single-line `let` statements take the T440 `Stmt` path (which
        // owns the `;` terminator), even though `let` inits now match
        // structurally (WI-5b).
        let parsed = (
            syn::parse_str::<syn::Expr>(stripped_m),
            syn::parse_str::<syn::Expr>(stripped_p),
        );
        let resolved: Option<(String, String, String, bool)> = match parsed {
            (Ok(ref a), Ok(ref b))
                if !matches!(a, syn::Expr::Let(_)) && !matches!(b, syn::Expr::Let(_)) =>
            {
                Some((
                    stripped_m.to_string(),
                    stripped_p.to_string(),
                    flat_ws(stripped_m),
                    false,
                ))
            }
            // T416: struct-literal fields (`label: expr`) fail `Expr`
            // parse on the full line — sweep on the label-stripped inner
            // expression when both sides parse and labels match (the
            // engine visits field values as sub-expressions, c48a6e6).
            _ => match (strip_field_label(stripped_m), strip_field_label(stripped_p)) {
                (Some((ml, mi)), Some((pl, pi))) if ml == pl => {
                    Some((mi.to_string(), pi.to_string(), flat_ws(mi), false))
                }
                _ => verbatim_let_shape(m, p).map(|()| (m.clone(), p.clone(), flat_ws(m), true)),
            },
        };
        let Some((emit_minus, emit_plus, needle, is_let)) = resolved else {
            continue;
        };
        // WI-5b novelty gate: verbatim emits directive-less literal
        // hunks, so diet inference hole-ifies every undeclared
        // uppercase ident. A plus-side ident absent from the minus
        // side would infer as an unbound hole and fail on apply
        // ("used in `+` without binding") — stay ordinary instead
        // (that shape is antiunify territory, which declares its
        // holes). Strings/backticks are masked so literal text never
        // counts as an introduced name.
        if plus_introduces_unbound_name(&emit_minus, &emit_plus) {
            continue;
        }
        // WI-7-upper: literal emission starts as the raw lines; the
        // Expr path may backtick an uppercase call head below.
        let (mut lit_minus, mut lit_plus) = (emit_minus.clone(), emit_plus.clone());
        if !is_let {
            let (minus_expr, plus_expr) = match (
                syn::parse_str::<syn::Expr>(&emit_minus),
                syn::parse_str::<syn::Expr>(&emit_plus),
            ) {
                (Ok(a), Ok(b)) => (a, b),
                _ => continue,
            };
            if !verbatim_shape_ok(&minus_expr, &plus_expr) {
                continue;
            }
            // WI-7-upper: backtick an uppercase call head (bare callee,
            // qualified last segment, turbofish-free method) so it stays
            // literal; the backstop below judges anything left over.
            lit_minus = literal_call_emit(&minus_expr, &emit_minus);
            lit_plus = literal_call_emit(&plus_expr, &emit_plus);
        }
        // Section-visible uniqueness, legacy path only (no pre-image):
        // the stripped minus occurs exactly once per own pair across
        // all old-side section lines. With a parsed pre-image the
        // subterm totality below subsumes this exactly (substring
        // matching false-positives on `&s.p` inside `&s.p.items` —
        // probe-verified a different subterm the engine leaves alone).
        if old_file.is_none() {
            let total: usize = old_lines
                .iter()
                .map(|l| count_occurrences(&flat_ws(l), &needle))
                .sum();
            if total != *n {
                continue;
            }
        }
        // Subterm totality on the pre-image file (T403 soundness),
        // line-aware on opaque text (T411): the needle occurs as a
        // sub-expression exactly group-size times; macro-body and
        // string-interior occurrences veto only when their span covers
        // a changed line of a hunk holding this group (unrelated twins
        // round-trip untouched). Syntactic, not textual: `&s.p` inside
        // `&s.p.items` is a different subterm (probe-verified: the
        // `.items` site is left alone while bare sites rewrite).
        // `let` groups (T440) count whole statements instead: a parsed
        // `let` cannot hide inside a macro body (opaque tokens) or a
        // string literal, so no line-aware veto is needed.
        if let Some(Some(old)) = &old_file {
            // Stranded join (T441 slice): exactly-1-minus pure
            // unbalanced hunks whose FIRST plus matches the group join
            // it — the stranded site would otherwise veto the group
            // while the sweep over-matches it. First-plus match keeps
            // the leftover a single contiguous tail insertion.
            let mut extra = 0usize;
            for j in 0..hunks.len() {
                if hunk_balanced[j] || joined[j].is_some() || !hunk_pure[j] {
                    continue;
                }
                if hunk_minuses[j].len() == 1
                    && hunk_minuses[j][0] == *m
                    && hunk_pluses[j].first().is_some_and(|p0| p0 == p)
                {
                    joined[j] = Some((m.clone(), p.clone()));
                    extra += 1;
                }
            }
            let n_eff = *n + extra;
            // WI-7-upper backstop: any surviving uppercase ident still
            // rides as an inferred hole (let-pat, general-expr positions;
            // backticked call heads are masked out above), so the engine
            // self-rewrite count must equal the group size or the group
            // stays ordinary instead of rewriting an untouched twin.
            // Non-code lines (doc comments) skip the probe — the legacy
            // path judges them. `None` pre-image keeps the legacy allow.
            {
                let code = syn::parse_str::<syn::Expr>(&lit_minus).is_ok()
                    || syn::parse_str::<syn::Stmt>(&lit_minus).is_ok();
                if code
                    && !upper_idents(&mask_code_words(&lit_minus)).is_empty()
                    && !crate::totality::template_expr_holds(path, file_old, &lit_minus, n_eff)
                {
                    for claim in joined.iter_mut() {
                        if claim.as_ref().is_some_and(|(gm, gp)| gm == m && gp == p) {
                            *claim = None;
                        }
                    }
                    continue;
                }
            }
            // Changed lines of the hunks holding this group's pairs.
            let mut group_changed: Vec<usize> = Vec::new();
            for (j, gm, gp) in pairs.iter() {
                if gm == m && gp == p {
                    for &c in &hunk_changed[*j] {
                        if !group_changed.contains(&c) {
                            group_changed.push(c);
                        }
                    }
                }
            }
            for (j, claim) in joined.iter().enumerate() {
                if claim.as_ref().is_some_and(|(gm, gp)| gm == m && gp == p) {
                    for &c in &hunk_changed[j] {
                        if !group_changed.contains(&c) {
                            group_changed.push(c);
                        }
                    }
                }
            }
            let holds = if is_let {
                totality_holds_local(old, &needle, n_eff)
            } else {
                match syn::parse_str::<syn::Expr>(&emit_minus) {
                    Ok(minus_expr) => totality_holds(old, &minus_expr, n_eff, &group_changed),
                    Err(_) => false,
                }
            };
            if !holds {
                // Release this group's claims: the hunks stay ordinary.
                for claim in joined.iter_mut() {
                    if claim.as_ref().is_some_and(|(gm, gp)| gm == m && gp == p) {
                        *claim = None;
                    }
                }
                continue;
            }
        }
        // WI-1b: precise emission sites are the member pair old lines
        // plus joined stranded old lines for this group.
        let mut sites: Vec<usize> = Vec::new();
        let mut members: Vec<usize> = Vec::new();
        for (idx, (_j, gm, gp)) in pairs.iter().enumerate() {
            if gm == m && gp == p {
                members.push(idx);
                let old_line = pair_old.get(idx).copied().unwrap_or(0);
                if !sites.contains(&old_line) {
                    sites.push(old_line);
                }
            }
        }
        for (j, claim) in joined.iter().enumerate() {
            if claim.as_ref().is_some_and(|(gm, gp)| gm == m && gp == p) {
                let old_line = hunk_changed[j].first().copied().unwrap_or(0);
                if !sites.contains(&old_line) {
                    sites.push(old_line);
                }
            }
        }
        // WI-2: record the group rule with its detector; failures below
        // truncate `used_rules`, and commit filters by presence in the
        // final text, so stale records never surface.
        candidates.push(Candidate {
            minus_trim: m.clone(),
            plus_trim: p.clone(),
            // WI-7-upper: literal emission (backticked call heads);
            // `minus_trim` stays raw for grouping/counting.
            minus: lit_minus,
            plus: lit_plus,
            sites,
            members,
        });
    }
    // Hunk-atomicity fixed point (macros.rs aa23c77): a group fires
    // only if every balanced hunk holding a member pair is fully
    // covered by passing groups — otherwise the group hunk and the
    // leftover ordinary hunk would rewrite the same line
    // (GIT_HUNK_CONFLICT overlapping edits). Drop violators (releasing
    // stranded-join claims) to a fixed point (cascade: dropping
    // uncovers hunks).
    loop {
        let covered: std::collections::HashSet<usize> = candidates
            .iter()
            .flat_map(|c| c.members.iter().copied())
            .collect();
        let mut victim: Option<usize> = None;
        'scan: for (gi, cand) in candidates.iter().enumerate() {
            for &idx in &cand.members {
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
        let (dm, dp) = (
            candidates[gi].minus_trim.clone(),
            candidates[gi].plus_trim.clone(),
        );
        for claim in joined.iter_mut() {
            if claim
                .as_ref()
                .is_some_and(|(gm, gp)| gm == &dm && gp == &dp)
            {
                *claim = None;
            }
        }
        candidates.remove(gi);
    }
    if candidates.is_empty() {
        return None;
    }
    // Reserve rules + WI-2 detector records for survivors only;
    // failures below truncate `used_rules`, and commit filters by
    // presence in the final text, so stale records never surface.
    let mut passing: Vec<VerbatimGroup> = Vec::new();
    for cand in candidates {
        let rule = unique_rule(used_rules, "verbatim");
        fired.push((rule.clone(), "verbatim"));
        passing.push(VerbatimGroup {
            minus_trim: cand.minus_trim,
            plus_trim: cand.plus_trim,
            minus: cand.minus,
            plus: cand.plus,
            rule,
            sites: cand.sites,
        });
    }
    // Leftover insertions for joined unbalanced hunks (T441 slice):
    // each claimed hunk sheds its minus + first plus and stays as a
    // pure insertion. Claims of vetoed groups were released above, so
    // every surviving claim belongs to a passing group; a header that
    // does not parse shape-exactly vetoes the whole sweep (fail safe).
    let mut split_leftovers: Vec<Option<(String, String, String)>> = vec![None; hunks.len()];
    for (j, claim) in joined.iter().enumerate() {
        let Some((gm, gp)) = claim else { continue };
        if !passing
            .iter()
            .any(|g| &g.minus_trim == gm && &g.plus_trim == gp)
        {
            continue;
        }
        let Some(leftover) = split_leftover_insertion(&hunks[j]) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        split_leftovers[j] = Some((gm.clone(), gp.clone(), leftover));
    }
    // A hunk is covered iff balanced and every pair sits in a group.
    let covered_hunk: Vec<bool> = hunk_balanced
        .iter()
        .enumerate()
        .map(|(i, b)| {
            *b && pairs.iter().filter(|(j, _, _)| *j == i).all(|(_, m, p)| {
                passing
                    .iter()
                    .any(|g| g.minus_trim == *m && g.plus_trim == *p)
            })
        })
        .collect();
    // Leftover balanced hunks retry the standard chain (identical to
    // the per-hunk fallback): compressible ones splice their bodies in
    // place, the rest stay ordinary.
    let mut leftover_bodies: Vec<(usize, String)> = Vec::new();
    for (i, hunk) in hunks.iter().enumerate() {
        if !hunk_balanced[i] || covered_hunk[i] {
            continue;
        }
        let ms: Vec<&str> = pairs
            .iter()
            .filter(|(j, _, _)| *j == i)
            .map(|(_, m, _)| m.as_str())
            .collect();
        let ps: Vec<&str> = pairs
            .iter()
            .filter(|(j, _, _)| *j == i)
            .map(|(_, _, p)| p.as_str())
            .collect();
        if let Some((body, detector)) = compress_balanced_pair_body(
            &ms,
            &ps,
            path,
            file_old,
            &hunk_changed[i],
            hunk.len(),
            used_rules,
        ) {
            if let Some(rule) = used_rules.last().cloned() {
                fired.push((rule, detector));
            }
            leftover_bodies.push((i, body));
        }
    }
    // T375 over the verbatim-covered set: the leftover retry is
    // fallback-identical, so the section beats the fallback exactly
    // when the verbatim bodies beat the covered ordinary hunks. Joined
    // leftovers are not fallback-identical (the fallback would emit
    // the whole hunk), so their bytes join both sides of the guard.
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
    // WI-1b: ranged bodies per group when spans are disjoint and ≤64;
    // overlapping groups share the sweep dummy (SPEC 10.6).
    let mut spans: Vec<Option<(usize, usize)>> = Vec::new();
    for g in &passing {
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
        .map(|(gi, g)| {
            if overlap[gi] || spans[gi].is_none() {
                format!(
                    "@@ -1,1 @@ rule={}\n# sweep\n- {}\n+ {}\n",
                    g.rule, g.minus, g.plus
                )
            } else {
                format!(
                    "{}- {}\n+ {}\n",
                    crate::emission_body(&g.rule, &g.sites),
                    g.minus,
                    g.plus
                )
            }
        })
        .collect();
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
    // leftover bodies, split insertions and ordinary hunks stay in
    // place.
    let mut out = prelude;
    let mut emitted = vec![false; passing.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        // A joined hunk holds its group pair via the claim (not `pairs`).
        let holds_pair = |g: &VerbatimGroup| {
            pairs
                .iter()
                .any(|(j, m, p)| *j == i && *m == g.minus_trim && *p == g.plus_trim)
                || split_leftovers[i]
                    .as_ref()
                    .is_some_and(|(gm, gp, _)| *gm == g.minus_trim && *gp == g.plus_trim)
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
        match leftover_bodies.iter().find(|(j, _)| *j == i) {
            Some((_, body)) => out.push_str(body),
            None => out.push_str(hunk),
        }
    }
    Some(out)
}

/// Leftover insertion for a joined unbalanced hunk (T441 slice):
/// `@@ -O,1 +N,K @@` (pure `-`/`+` body, minus first, single change
/// block) sheds its minus + first plus and becomes the pure insertion
/// `@@ -O,0 +N+1,K-1 @@` over the remaining pluses. Returns None when
/// the hunk is not exactly that shape (fail safe — the caller vetoes
/// the sweep). The consumed first plus is identical text to the sweep
/// rewrite, so the byte order is exact either way.
pub(crate) fn split_leftover_insertion(hunk: &str) -> Option<String> {
    let mut lines = hunk.lines();
    let header = lines.next()?;
    let mut parts = header.split_whitespace();
    parts.next()?; // `@@`
    let old = parts.next()?;
    let new = parts.next()?;
    let o: usize = old
        .trim_start_matches('-')
        .split(',')
        .next()?
        .parse()
        .ok()?;
    if old.split(',').nth(1).is_some_and(|len| len != "1") {
        return None;
    }
    let n: usize = new
        .trim_start_matches('+')
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let body: Vec<&str> = lines.collect();
    // One minus first, then only pluses (single change block).
    if body.len() < 3 {
        return None;
    }
    if !(body[0].starts_with('-') && !body[0].starts_with("---")) {
        return None;
    }
    for line in &body[1..] {
        if !(line.starts_with('+') && !line.starts_with("+++")) {
            return None;
        }
    }
    let new_len: usize = new.split(',').nth(1).unwrap_or("1").parse().ok()?;
    if new_len != body.len() - 1 {
        return None;
    }
    let rest = &body[2..];
    let mut out = String::new();
    if rest.len() == 1 {
        out.push_str(&format!("@@ -{o},0 +{} @@\n", n + 1));
    } else {
        out.push_str(&format!("@@ -{o},0 +{},{} @@\n", n + 1, rest.len()));
    }
    for line in rest {
        out.push_str(line);
        out.push('\n');
    }
    Some(out)
}

/// Split a section into its prelude lines and native hunks (the split
/// the per-hunk fallback uses; each part keeps trailing newlines).
fn split_verbatim_hunks(section: &str) -> (String, Vec<String>) {
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
    (prelude, hunks)
}

/// True when the plus side introduces an uppercase ident absent from
/// the minus side (WI-5b novelty gate — see the call site).
fn plus_introduces_unbound_name(minus: &str, plus: &str) -> bool {
    let masked_minus = mask_code_words(minus);
    let masked_plus = mask_code_words(plus);
    let minus_ids = upper_idents(&masked_minus);
    upper_idents(&masked_plus)
        .iter()
        .any(|n| !minus_ids.iter().any(|m| m == n))
}

/// Blank `"..."` strings and `` `...` `` literals so their words never
/// count as introduced names (mirrors the compiler's masking).
fn mask_code_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    let mut in_tick = false;
    let mut esc = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
                out.push(' ');
            }
        } else if in_tick {
            if b == b'`' {
                in_tick = false;
                out.push(' ');
            }
        } else if b == b'"' {
            in_str = true;
            out.push(' ');
        } else if b == b'`' {
            in_tick = true;
            out.push(' ');
        } else {
            out.push(b as char);
        }
        i += 1;
    }
    out
}

/// Uppercase-start ident runs (`is_candidate` mirror without the
/// dependency — `verbatim.rs` must not pull in `rsmpl_pattern`).
fn upper_idents(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_uppercase() {
            let mut j = i + 1;
            while j < bytes.len() {
                let d = bytes[j] as char;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(s[i..j].to_string());
            i = j;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i + 1;
            while j < bytes.len() {
                let d = bytes[j] as char;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Backtick a top-level call head (WI-7-upper): a bare uppercase callee
/// (`Old(x)`), an uppercase last path segment (`crate::Old(x)`), or an
/// uppercase turbofish-free method ident (`a.Old(x)`) infers as an
/// `expr` hole and over-matches twin sites (probed: an interleaved
/// `Other` twin rewritten with a bogus `Old` binding). Backticks pin
/// the literal (engine-probed spellings); lowercase heads pass through
/// untouched via `literal_path_last`/`literal_callee` (zero churn).
/// Leading-`::` (no engine spelling — declined like the qualified
/// sweep), qself, turbofish-carrying methods (already literal-safe:
/// the `::<` suffix defeats `is_candidate`, probed), and non-call exprs
/// pass through for the engine-count backstop at the call site.
fn literal_call_emit(expr: &syn::Expr, emit: &str) -> String {
    match expr {
        syn::Expr::Call(call) => {
            let syn::Expr::Path(p) = call.func.as_ref() else {
                return emit.to_string();
            };
            if p.qself.is_some() {
                return emit.to_string();
            }
            let Some(paren) = emit.find('(') else {
                return emit.to_string();
            };
            let (head, tail) = emit.split_at(paren);
            if head.starts_with("::") {
                return emit.to_string();
            }
            format!("{}{tail}", super::shape::literal_path_last(head))
        }
        syn::Expr::MethodCall(method) => {
            if method.turbofish.is_some() {
                return emit.to_string();
            }
            let m = method.method.to_string();
            let lit = super::shape::literal_callee(&m);
            if lit == m {
                return emit.to_string();
            }
            let Some(paren) = emit.find('(') else {
                return emit.to_string();
            };
            let (head, tail) = emit.split_at(paren);
            let Some(idx) = head.rfind('.') else {
                return emit.to_string();
            };
            if head[idx + 1..] != m {
                return emit.to_string();
            }
            format!("{}.{lit}{tail}", &head[..idx])
        }
        _ => emit.to_string(),
    }
}

/// Engine-capability gates for one verbatim pair (T403): both sides
/// strip to expressions the engine can match and construct. Rejects
/// `let` scrutinees (they fail `Expr` parse — complete single-line
/// `let` statements take the T440 `Stmt` path in the caller instead;
/// multi-line fragments like b849869's `let lenient =` never parse
/// and stay ordinary), macro
/// invocations anywhere on either side (plain patterns never enter
/// macro bodies, SPEC 21 — 33a8cbb failed loud), bare-ident and
/// literal minus sides (they would match every mention), and
/// `?`-applied calls wrapping a closure on either side (WI-5b: `try`
/// around a closure matches structurally now — the a2fc9b3 veto is
/// gone). Plus-side closures construct fine (probe-verified: concrete
/// plus text splices), so only plus-side macros stay rejected. Top-level Unary
/// (`!`, `-`, ...) matches via the general matcher (T415: the router
/// bypasses leading `!`/`-` to `Raw`, mirroring the T252 `&` bypass;
/// the T414 veto was removed when routing was fixed).
fn verbatim_shape_ok(minus: &syn::Expr, plus: &syn::Expr) -> bool {
    match minus {
        syn::Expr::Lit(_) => return false,
        syn::Expr::Path(ep) if ep.qself.is_none() && ep.path.segments.len() == 1 => return false,
        _ => {}
    }
    if matches!(plus, syn::Expr::Lit(_)) {
        return false;
    }
    // Unary (`!`, `-`, ...) matches since T415 (the T414 veto is gone):
    // (T414 veto removed by T415: leading `!`/`-` now bypasses to `Raw`
    // in `classify_call_pattern` and skips the macro pre-route, so the
    // general matcher handles unary structurally. `&` precedent is T252.)
    let m = probe_expr(minus);
    // WI-5b: `let` expressions, closures, `match`/loops (and `try`
    // around them) match structurally now — only macros veto here.
    if m.macros > 0 {
        return false;
    }
    let p = probe_expr(plus);
    if p.macros > 0 {
        return false;
    }
    // T412: the engine general matcher (`match_expr`) handles only a
    // bounded Expr variant set — a top-level Call/MethodCall pair takes
    // the call matcher (concrete args compare opaquely, closures fine),
    // but any other top-level shape compiles to a General hunk where an
    // unsupported node (macros only — blocks/`if` since WI-5a,
    // match/loops/closures/`let` since WI-5b, `return` since T418,
    // `async`/`try`/`unsafe`/`yield` since WI-6 slice 1)
    // never matches. Reject the pair so the section stays ordinary
    // (2382b02 `return ...` failed `--check` live before T418).
    if !matches!(minus, syn::Expr::Call(_) | syn::Expr::MethodCall(_)) && m.unsupported > 0 {
        return false;
    }
    if !matches!(plus, syn::Expr::Call(_) | syn::Expr::MethodCall(_)) && p.unsupported > 0 {
        return false;
    }
    true
}

// Engine-shape truth lives in `super::shape` (WI-4b debt): the probe
// struct, the bounded `match_expr` variant set, and the strict init
// gate are shared with `antiunify.rs` (agreement by construction).

/// Complete-`let` pair gate (T440): both trimmed lines parse as
/// `syn::Stmt::Local` (a successful `Stmt` parse already implies the
/// semicolon — syn 2 types it as a plain `Semi`) with no `let-else`
/// diverge (WI-5a engine matches diverge structurally, but this sweep
/// keeps the conservative veto — no totality change here), and init
/// presence agrees (engine `(Some, None)` never matches). Init
/// expressions satisfy the expression shape gates: no macros and no
/// unsupported nodes on either side (WI-5b: lets/closures/match/loops
/// are matchable inits; the `Stmt` path matches inits via structural
/// matches inits via structural `match_expr`, never the call router,
/// so there is no Call/MethodCall top-level exception). Multi-line
/// fragments (`let lenient =` with no init/semicolon — the b849869
/// "failed loud") never parse as `Stmt` and stay ordinary. The whole
/// statement is the pattern, so no bare-ident/literal minus veto
/// applies (unlike expressions, a `let` statement is always precise).
fn verbatim_let_shape(m: &str, p: &str) -> Option<()> {
    let (minus, plus) = match (
        syn::parse_str::<syn::Stmt>(m),
        syn::parse_str::<syn::Stmt>(p),
    ) {
        (Ok(syn::Stmt::Local(a)), Ok(syn::Stmt::Local(b))) => (a, b),
        _ => return None,
    };
    match (&minus.init, &plus.init) {
        (Some(a), Some(b)) => {
            if a.diverge.is_some() || b.diverge.is_some() {
                return None;
            }
            if !verbatim_init_ok(&a.expr, &b.expr) {
                return None;
            }
        }
        (None, None) => {}
        _ => return None,
    }
    Some(())
}

/// Init-expression gates for the `let` path (T440): mirrors
/// `verbatim_shape_ok` minus/plus rules — the engine `match_expr`
/// rejects macros and unsupported nodes with `_ => false`, so a
/// pattern containing them matches nothing and the sweep would fail
/// loud (WI-5b: lets/closures/match/loops are matchable inits now).
fn verbatim_init_ok(minus: &syn::Expr, plus: &syn::Expr) -> bool {
    init_ok(minus) && init_ok(plus)
}

/// Statement totality (T440 soundness): the needle (flat-ws full
/// `let` line) occurs as a `Stmt::Local` exactly `n` times in the
/// pre-image file. No line-aware opaque veto: a parsed statement
/// cannot hide inside a macro body (opaque tokens the engine never
/// enters either) or a string literal, so statement counting is
/// exactly the engine's match domain.
fn totality_holds_local(old: &syn::File, needle_norm: &str, n: usize) -> bool {
    struct LocalScan<'a> {
        needle: &'a str,
        count: usize,
    }
    impl<'ast, 'a> syn::visit::Visit<'ast> for LocalScan<'a> {
        fn visit_stmt(&mut self, node: &'ast syn::Stmt) {
            if let syn::Stmt::Local(local) = node {
                use quote::ToTokens as _;
                if flat_ws(&local.to_token_stream().to_string()) == self.needle {
                    self.count += 1;
                }
            }
            syn::visit::visit_stmt(self, node);
        }
    }
    let mut scan = LocalScan {
        needle: needle_norm,
        count: 0,
    };
    scan.visit_file(old);
    scan.count == n
}

/// Subterm totality (T403 soundness), line-aware on opaque text (T411):
/// the needle occurs as a sub-expression exactly `n` times in the
/// pre-image file; macro-body and string-literal occurrences veto only
/// when their span covers a changed old-side line (unrelated twins
/// round-trip untouched).
fn totality_holds(old: &syn::File, needle: &syn::Expr, n: usize, changed: &[usize]) -> bool {
    use quote::ToTokens;
    let needle_text = flat_ws(&needle.to_token_stream().to_string());
    let mut scan = TotalityScan {
        needle: needle_text,
        changed: changed.to_vec(),
        subterms: 0,
        opaque_hits: 0,
    };
    scan.visit_file(old);
    scan.subterms == n && scan.opaque_hits == 0
}

/// One AST walk counting flat-ws-equal sub-expressions of the needle;
/// macro-body occurrences (the engine never matches inside one) and
/// string-literal interiors (never match, T360) veto only when their
/// span covers a changed line (T411 line-aware: unrelated twins
/// round-trip untouched).
struct TotalityScan {
    needle: String,
    changed: Vec<usize>,
    subterms: usize,
    opaque_hits: usize,
}

impl<'ast> syn::visit::Visit<'ast> for TotalityScan {
    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        use quote::ToTokens;
        if flat_ws(&node.to_token_stream().to_string()) == self.needle {
            self.subterms += 1;
        }
        syn::visit::visit_expr(self, node);
    }
    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        let hay = flat_ws(&node.mac.tokens.to_string());
        if count_occurrences(&hay, &self.needle) > 0 {
            let span = node.span();
            if covers_changed(span.start().line, span.end().line, &self.changed) {
                self.opaque_hits += 1;
            }
        }
    }
    fn visit_lit_str(&mut self, node: &'ast syn::LitStr) {
        let hay = flat_ws(&node.value());
        if count_occurrences(&hay, &self.needle) > 0 {
            let span = node.span();
            if covers_changed(span.start().line, span.end().line, &self.changed) {
                self.opaque_hits += 1;
            }
        }
    }
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
    fn split_leftover_insertion_pins_shape() {
        // Exact 1-minus-first shape splits; anything else declines.
        let hunk = "@@ -20,1 +21,4 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n+    let done = extra();\n+\n+    done();\n";
        assert_eq!(
            split_leftover_insertion(hunk),
            Some("@@ -20,0 +22,3 @@\n+    let done = extra();\n+\n+    done();\n".to_string())
        );
        // Two minuses: not the single-pair shape.
        let two = "@@ -20,2 +21,4 @@\n-    a();\n-    b();\n+    c();\n+    d();\n+    e();\n";
        assert_eq!(split_leftover_insertion(two), None);
        // Minus not first: positional reasoning fails.
        let late = "@@ -20,1 +21,3 @@\n+    lead();\n-    a();\n+    b();\n";
        assert_eq!(split_leftover_insertion(late), None);
        // Single plus: balanced, never joined.
        let one = "@@ -20,1 +21,1 @@\n-    a();\n+    b();\n";
        assert_eq!(split_leftover_insertion(one), None);
        // Header/body length mismatch.
        let bad = "@@ -20,1 +21,9 @@\n-    a();\n+    b();\n+    c();\n";
        assert_eq!(split_leftover_insertion(bad), None);
    }

    #[test]
    fn verbatim_mixed_hunk_atomicity_drops_group() {
        // WI-9e: H1 holds a repeated pair (foo, 3 sites) plus a
        // singleton (bar); the foo group must drop — emitting it would
        // overlap the H1 ordinary leftover — so inference declines and
        // the section falls through to ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@ pub fn a() {\n-    foo(x);\n-    bar(1);\n+    foo(y);\n+    bar(2);\n@@ -6 +6 @@ pub fn b() {\n-    foo(x);\n+    foo(y);\n@@ -9 +9 @@ pub fn c() {\n-    foo(x);\n+    foo(y);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "pub fn a() {\n    foo(x);\n    bar(1);\n}\npub fn b() {\n    foo(x);\n}\npub fn c() {\n    foo(x);\n}\n",
        )]);
        assert!(
            crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn verbatim_fully_covered_groups_fire() {
        // WI-9e control: both pairs repeat across two fully-covered
        // hunks — the fixed point drops nothing and both groups emit.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@ pub fn a() {\n-    foo(x);\n-    bar(1);\n+    foo(y);\n+    bar(2);\n@@ -6,2 +6,2 @@ pub fn b() {\n-    foo(x);\n-    bar(1);\n+    foo(y);\n+    bar(2);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "pub fn a() {\n    foo(x);\n    bar(1);\n}\npub fn b() {\n    foo(x);\n    bar(1);\n}\n",
        )]);
        let out = crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("- foo(x)\n"), "got: {out}");
        assert!(out.contains("- bar(1)\n"), "got: {out}");
        assert!(
            !out.contains("-    foo(x);"),
            "covered lines must be consumed, got: {out}"
        );
    }
}
