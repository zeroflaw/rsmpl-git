//! WI-4 sub-hunk partition (SPEC D.4): detector-boundary split.
//!
//! Split from `semantic/chain.rs` under the 2000-line gate: new module,
//! `chain.rs` keeps the whole-section path and the per-hunk fallback.
//! Tried only after every whole-section and whole-hunk detector declines
//! (inside `compress_section_per_hunk`): a single balanced native hunk
//! with N ≥ 2 pairs whose pairs belong to different detector classes
//! raises per contiguous same-class run — each run offered to the shared
//! `compress_balanced_pair_body` chain, declining runs kept ordinary
//! verbatim in place, document order preserved, never reordered.
//!
//! Laws (IMPROVE.md WI-4): maximal contiguous runs of one classification
//! key; an interleaved key (A B A) vetoes the whole hunk (order is never
//! manufactured); a hunk holding a `struct`/`enum` rename beside a
//! member line of that same container stays ordinary (WI-5/WI-21 joint
//! territory — splitting it would strand the container joint; proven via
//! the `carrier` exactly-one-container law, not by spelling); any
//! suppressed-callee mention vetoes (WI-4a suppression posture — the
//! call-run offer would otherwise resurrect the suppressed sweep);
//! duplicate minus lines veto (verbatim territory); duplicate emitted
//! minus patterns veto (terse shapes would overlap-match). Totality is the
//! D13 composed scratch-apply (emission applied to a `pre` clone must
//! reproduce the native post tree byte-identically) plus the scaled
//! precision guard (`emission <= hunk + 16 * claimed_pairs`, the WI-19
//! partial-partition posture).
//!
//! Evidence posture (WI-15): the single-pair/run classification probes
//! below pass a throwaway declined vec by design — they are speculative
//! offers, not chain attempts. The whole-hunk attempt that precedes the
//! split already runs on the caller's vec and owns per-family evidence.

use super::prov::push_section_prov;
use super::{normalized_hunk_len, HunkProvenance, PRECISION_GUARD};

/// One emitted semantic hunk: body, detector name, and the rule-checkpoint
/// before its reservation (for per-group provenance).
type EmittedHunk = (String, &'static str, usize);

/// Try to split one balanced native hunk (including its `@@` header line)
/// at detector-class boundaries. Returns the mixed fragment (semantic
/// group bodies interleaved with ordinary remainder hunks, in document
/// order) and commits one provenance entry per semantic group. `None`
/// with `used_rules`/`prov` untouched when the hunk stays ordinary.
pub(crate) fn try_subhunk_partition(
    hunk: &str,
    path: &str,
    file_old: Option<&str>,
    used_rules: &mut Vec<String>,
    prov: &mut Vec<HunkProvenance>,
    suppressed: &std::collections::HashSet<(String, String)>,
) -> Option<String> {
    let pre = file_old?;
    // Split header from body; every body line must be a -/+ pair line
    // (-U0 hunks carry no context — a context line means this hunk is
    // not a pure pair list and stays ordinary).
    let header = hunk.lines().next()?;
    if !header.starts_with("@@") {
        return None;
    }
    let mut minuses: Vec<&str> = Vec::new();
    let mut pluses: Vec<&str> = Vec::new();
    for line in hunk.lines().skip(1) {
        if line.starts_with('-') && !line.starts_with("---") {
            minuses.push(&line[1..]);
        } else if line.starts_with('+') && !line.starts_with("+++") {
            pluses.push(&line[1..]);
        } else {
            return None;
        }
    }
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    // Verbatim territory: identical minus lines belong to the verbatim
    // sweep, never to a partition (same law as both section partitions).
    {
        let mut ordered: Vec<&&str> = minuses.iter().collect();
        ordered.sort();
        ordered.dedup();
        if ordered.len() != minuses.len() {
            return None;
        }
    }
    // WI-4a suppression posture: any suppressed-callee mention poisons
    // the hunk — the per-hunk gates keep pure-call hunks ordinary, and
    // the def hunk of a vetoed joint must never split (the call-run
    // offer below would otherwise resurrect the suppressed sweep).
    if !suppressed.is_empty()
        && minuses.iter().any(|m| {
            suppressed
                .iter()
                .any(|(p, o)| p == path && contains_ident(m, o))
        })
    {
        return None;
    }
    let sites = crate::totality::changed_old_lines(hunk);
    if sites.len() != minuses.len() {
        return None;
    }
    // Old-side contiguity (-U0 invariant): remainder rebuilds cite
    // covering ranges, so a gapped site list vetoes.
    for w in sites.windows(2) {
        if w[1] != w[0] + 1 {
            return None;
        }
    }
    let (new_start, _) = parse_hunk_ranges(header)?;
    // Budget coherence: synthetic per-pair/run headers are budgeted with
    // the native hunk title the real per-hunk path sees (`compress_balanced
    // `_pair_body` prices `hunk_len` title-included). Titles stay stripped
    // from emitted text (T343) — only budgets grow, never output.
    // WI-2b: every budget below measures the rendering-normalized span
    // (`normalized_hunk_len`), so elided real-git headers price like the
    // explicit synthetics.
    let title: &str = header.split("@@").nth(2).unwrap_or("");
    let title_len = title.len();
    // Classify each pair without reserving rules (scratch counter): the
    // key is the detector the shared chain picks for the pair alone.
    let mut keys: Vec<Option<&'static str>> = Vec::with_capacity(minuses.len());
    for i in 0..minuses.len() {
        let mut scratch = used_rules.clone();
        let pair_native = format!(
            "@@ -{} +{} @@{}\n-{}\n+{}\n",
            sites[i],
            new_start + i,
            title,
            minuses[i],
            pluses[i]
        );
        let key = crate::compress_balanced_pair_body(
            &minuses[i..i + 1],
            &pluses[i..i + 1],
            path,
            Some(pre),
            &sites[i..i + 1],
            normalized_hunk_len(&pair_native),
            &mut scratch,
            &mut Vec::new(),
        )
        .map(|(_, detector)| detector);
        keys.push(key);
    }
    // WI-5/WI-21 joint territory: a `struct`/`enum` rename beside a
    // member line of that same container is a container joint (or its
    // residue) — never split. Proven by the carrier law, so a
    // coincidentally co-located alias evolution still partitions.
    if joint_territory_veto(pre, &minuses, &pluses, &sites, &keys) {
        return None;
    }
    // WI-5 binder fusion: maximal contiguous `[binder-def + use*]`
    // groups relabel to one `let_binder` run. The run offer below
    // routes the fused group whole to the chain arm, which carries the
    // full proof (count + follow-coverage + scratch-apply); proof
    // failure falls back to per-pair singles (today's behavior: the
    // binder stays ordinary, the use fires alone). The gate is purely
    // textual (no pre-image); the proof lives in the detector.
    {
        let mut i = 0;
        while i < minuses.len() {
            if let Some((old, new)) = super::is_binder_def_pair(minuses[i], pluses[i]) {
                let mut j = i + 1;
                while j < minuses.len()
                    && super::is_binder_use_pair(minuses[j], pluses[j], &old, &new)
                {
                    j += 1;
                }
                if j > i + 1 {
                    for slot in keys.iter_mut().take(j).skip(i) {
                        *slot = Some("let_binder");
                    }
                    i = j;
                    continue;
                }
            }
            i += 1;
        }
    }
    // Maximal contiguous runs of one key.
    let mut runs: Vec<(Option<&'static str>, usize, usize)> = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        if let Some((last_key, _, end)) = runs.last_mut() {
            if *last_key == *key {
                *end = i + 1;
                continue;
            }
        }
        runs.push((*key, i, i + 1));
    }
    // Interleave veto: a non-ordinary key in ≥ 2 non-adjacent runs
    // stays ordinary (order is never manufactured).
    {
        let mut seen: Vec<Option<&'static str>> = Vec::new();
        for (key, _, _) in &runs {
            if key.is_some() && seen.contains(key) {
                return None;
            }
            seen.push(*key);
        }
    }
    let checkpoint = used_rules.len();
    // Offer each run whole (multi shapes ride here); runs that decline
    // retry per-pair singles, all-or-nothing per run — a half-claimed
    // run stays ordinary whole.
    enum Claimed {
        Whole(EmittedHunk),
        Singles(Vec<EmittedHunk>),
        /// An all-`None` run split into two firing multi sides at `mid`
        /// (both halves proved whole; see `try_split_none_run`).
        Split(Vec<EmittedHunk>, usize),
    }
    let mut outcomes: Vec<Option<Claimed>> = Vec::with_capacity(runs.len());
    for (_, start, end) in &runs {
        let run_start = used_rules.len();
        let run_native =
            run_native_hunk(header, &minuses, &pluses, &sites, new_start, *start, *end);
        let run_changed = &sites[*start..*end];
        if let Some((body, detector)) = crate::compress_balanced_pair_body(
            &minuses[*start..*end],
            &pluses[*start..*end],
            path,
            Some(pre),
            run_changed,
            normalized_hunk_len(&run_native) + title_len,
            used_rules,
            &mut Vec::new(),
        ) {
            outcomes.push(Some(Claimed::Whole((body, detector, run_start))));
            continue;
        }
        used_rules.truncate(run_start);
        // Runs that decline whole may still hold two firing multi
        // sides — including runs keyed by last-resort literal singles
        // (WI-6 slice 1: a literal key must never shadow the hetero
        // multis its pairs belong to, bench 48). The bounded
        // split-search therefore runs before per-pair singles for
        // every run; for all-`None` runs this is the established path
        // (each homogeneous side proves as a sweep while no single
        // pair proves at n=1).
        if let Some((split, mid)) = try_split_none_run(
            header, &minuses, &pluses, &sites, new_start, *start, *end, path, pre, used_rules,
        ) {
            outcomes.push(Some(Claimed::Split(split, mid)));
            continue;
        }
        used_rules.truncate(run_start);
        // No split claims the run. All-`None` runs stay ordinary whole
        // here (a lone pair's single already declined by construction
        // of the `None` keys, so per-pair retry is vacuous).
        if keys[*start..*end].iter().all(|k| k.is_none()) {
            outcomes.push(None);
            continue;
        }
        let mut bodies = Vec::new();
        let mut ok = true;
        for i in *start..*end {
            let single_start = used_rules.len();
            let pair_native = format!(
                "@@ -{} +{} @@{}\n-{}\n+{}\n",
                sites[i],
                new_start + i,
                title,
                minuses[i],
                pluses[i]
            );
            match crate::compress_balanced_pair_body(
                &minuses[i..i + 1],
                &pluses[i..i + 1],
                path,
                Some(pre),
                &sites[i..i + 1],
                normalized_hunk_len(&pair_native),
                used_rules,
                &mut Vec::new(),
            ) {
                Some((body, detector)) => bodies.push((body, detector, single_start)),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            outcomes.push(Some(Claimed::Singles(bodies)));
        } else {
            used_rules.truncate(run_start);
            outcomes.push(None);
        }
    }
    if outcomes.iter().all(|o| o.is_none()) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Duplicate emitted minus patterns across semantic runs would
    // overlap-match under their rules — veto the whole hunk (terse
    // shapes share spellings across distinct raw lines).
    {
        let mut pats: Vec<&str> = Vec::new();
        for o in outcomes.iter().flatten() {
            match o {
                Claimed::Whole((body, _, _)) => first_minus_pat(body, &mut pats),
                Claimed::Singles(bodies) | Claimed::Split(bodies, _) => {
                    for (body, _, _) in bodies {
                        first_minus_pat(body, &mut pats);
                    }
                }
            }
        }
        let mut ordered = pats.clone();
        ordered.sort();
        ordered.dedup();
        if ordered.len() != pats.len() {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    // Rebuild in document order: semantic bodies plus ordinary
    // remainder hunks, with per-group provenance inputs.
    let mut fragment = String::new();
    let mut n_claimed_pairs = 0usize;
    let mut groups: Vec<(String, String, &'static str, Vec<usize>, usize)> = Vec::new();
    for (ri, (_, start, end)) in runs.iter().enumerate() {
        match &outcomes[ri] {
            Some(Claimed::Whole((body, detector, run_start))) => {
                n_claimed_pairs += end - start;
                fragment.push_str(body);
                if !body.ends_with('\n') {
                    fragment.push('\n');
                }
                groups.push((
                    body.clone(),
                    run_native_hunk(header, &minuses, &pluses, &sites, new_start, *start, *end),
                    *detector,
                    sites[*start..*end].to_vec(),
                    *run_start,
                ));
            }
            Some(Claimed::Split(bodies, mid)) => {
                // Split halves cover [start, mid) and [mid, end).
                let bounds = [(*start, *mid), (*mid, *end)];
                for ((body, detector, half_start), (hs, he)) in bodies.iter().zip(bounds.iter()) {
                    n_claimed_pairs += he - hs;
                    fragment.push_str(body);
                    if !body.ends_with('\n') {
                        fragment.push('\n');
                    }
                    groups.push((
                        body.clone(),
                        run_native_hunk(header, &minuses, &pluses, &sites, new_start, *hs, *he),
                        *detector,
                        sites[*hs..*he].to_vec(),
                        *half_start,
                    ));
                }
            }
            Some(Claimed::Singles(bodies)) => {
                for (k, (body, detector, single_start)) in bodies.iter().enumerate() {
                    n_claimed_pairs += 1;
                    fragment.push_str(body);
                    if !body.ends_with('\n') {
                        fragment.push('\n');
                    }
                    let i = start + k;
                    groups.push((
                        body.clone(),
                        format!(
                            "@@ -{} +{} @@\n-{}\n+{}\n",
                            sites[i],
                            new_start + i,
                            minuses[i],
                            pluses[i]
                        ),
                        *detector,
                        vec![sites[i]],
                        *single_start,
                    ));
                }
            }
            None => fragment.push_str(&run_native_hunk(
                header, &minuses, &pluses, &sites, new_start, *start, *end,
            )),
        }
    }
    // Scaled precision guard + D13 composed scratch-apply: the emission
    // applied to a `pre` clone must reproduce the native post tree.
    // Method `sig_evolution` runs carry the mandatory WI-4a filter
    // block plus the run's own header (~53 measured over the native
    // slice: filter + rule tag + extra header). Each method run therefore
    // costs three extra guards beyond its base (total 64/run — the same
    // method-run price the per-hunk `* 4` budget pays). The filter header
    // itself is priced per emission (`filter_header_excess`): long
    // enclosing `impl` headers contribute exactly their surplus over the
    // measured baseline on top of the fixed per-run price.
    let n_method_runs = groups
        .iter()
        .filter(|(_, native, detector, _, _)| {
            *detector == "sig_evolution"
                && native
                    .lines()
                    .skip(1)
                    .find(|l| l.starts_with('-'))
                    .is_some_and(|l| l[1..].starts_with([' ', '\t']))
        })
        .count();
    if fragment.len()
        > normalized_hunk_len(hunk)
            + super::guard::precision_allowance(
                n_claimed_pairs,
                3 * PRECISION_GUARD * n_method_runs,
            )
            + super::block_headers::filter_header_excess(&fragment)
    {
        used_rules.truncate(checkpoint);
        return None;
    }
    if !composed_reproduces_native(path, pre, &fragment, hunk) {
        used_rules.truncate(checkpoint);
        return None;
    }
    for (body, native, detector, changed, rules_before) in &groups {
        push_section_prov(
            prov,
            body,
            native,
            path,
            detector,
            changed,
            Some(pre),
            *rules_before,
            used_rules,
        );
    }
    Some(fragment)
}

/// Bounded split-search for a run that declined whole (no single shape
/// owns the run together): try every internal split with both halves
/// ≥ 2 pairs, each half offered whole (multi shapes ride here — the
/// per-pair singles run only when no split claims the run). The first
/// split whose halves both fire wins, in document order; halves that
/// are themselves hetero decline whole and fail the split (no
/// recursion, so interleaved multis stay ordinary). Returns the two
/// bodies with their rule checkpoints. `used_rules` is rolled back to
/// entry on failure.
#[allow(clippy::too_many_arguments)]
fn try_split_none_run(
    header: &str,
    minuses: &[&str],
    pluses: &[&str],
    sites: &[usize],
    new_start: usize,
    start: usize,
    end: usize,
    path: &str,
    pre: &str,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<EmittedHunk>, usize)> {
    let entry = used_rules.len();
    // Splits need both halves ≥ 2 (a lone pair's single already declined).
    if end - start < 4 {
        return None;
    }
    let split_title_len = header.split("@@").nth(2).map(str::len).unwrap_or(0);
    for mid in (start + 2)..=(end - 2) {
        used_rules.truncate(entry);
        let left_start = used_rules.len();
        let left_native = run_native_hunk(header, minuses, pluses, sites, new_start, start, mid);
        let left = crate::compress_balanced_pair_body(
            &minuses[start..mid],
            &pluses[start..mid],
            path,
            Some(pre),
            &sites[start..mid],
            normalized_hunk_len(&left_native) + split_title_len,
            used_rules,
            &mut Vec::new(),
        );
        let Some((left_body, left_detector)) = left else {
            continue;
        };
        let right_start = used_rules.len();
        let right_native = run_native_hunk(header, minuses, pluses, sites, new_start, mid, end);
        let right = crate::compress_balanced_pair_body(
            &minuses[mid..end],
            &pluses[mid..end],
            path,
            Some(pre),
            &sites[mid..end],
            normalized_hunk_len(&right_native) + split_title_len,
            used_rules,
            &mut Vec::new(),
        );
        let Some((right_body, right_detector)) = right else {
            continue;
        };
        return Some((
            vec![
                (left_body, left_detector, left_start),
                (right_body, right_detector, right_start),
            ],
            mid,
        ));
    }
    used_rules.truncate(entry);
    None
}

/// WI-5/WI-21 joint-territory veto: a `struct`/`enum` item-rename pair
/// beside a member line of that same container (proven by the `carrier`
/// exactly-one-container law over the pre-image) is a container joint or
/// its residue — never split. A coincidentally co-located non-member
/// (alias evolution, independent rename) still partitions.
fn joint_territory_veto(
    pre: &str,
    minuses: &[&str],
    pluses: &[&str],
    sites: &[usize],
    keys: &[Option<&'static str>],
) -> bool {
    for (i, key) in keys.iter().enumerate() {
        if *key != Some("item_rename") {
            continue;
        }
        let (kind, old_ident) = match item_kind_ident(minuses[i]) {
            Some((k, id)) if k == "struct" || k == "enum" => (k, id),
            _ => continue,
        };
        for (j, _) in minuses.iter().enumerate() {
            if i == j {
                continue;
            }
            // Member-shaped minus: `name: Type` (field) or bare/variant
            // payload (variant/discriminant) — alias/const shapes carry
            // their own head keyword and never count as members.
            if is_item_head(minuses[j]) || is_item_head(pluses[j]) {
                continue;
            }
            if kind == "struct" {
                if let Some(field) = field_name(minuses[j]) {
                    if let Some(carrier) =
                        super::carrier::struct_carrier(pre, &field, sites[j], false)
                    {
                        if carrier == old_ident {
                            return true;
                        }
                    }
                }
            } else if let Some(variant) = variant_name(minuses[j]) {
                if let Some(carrier) = super::carrier::enum_carrier(pre, &variant, sites[j], false)
                {
                    if carrier == old_ident {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// `(kind, ident)` for item-head lines (`struct Old {`, `const C …`),
/// or `None` for non-item lines.
fn item_kind_ident(line: &str) -> Option<(String, String)> {
    let (_, kind, ident, _) = super::split_item(line.trim_end())?;
    Some((kind, ident))
}

/// True for lines carrying their own head keyword (`struct`/`enum`/`fn`/
/// `type`/`const`/`static`/`mod`/`trait`/`impl`/`use`/`let`/`match` …) —
/// never member lines. `let` lines are statements, not items, but they
/// are likewise never container members, so they count as heads here
/// (a `let` beside a struct rename is no joint).
fn is_item_head(line: &str) -> bool {
    if super::split_item(line.trim_end()).is_some() {
        return true;
    }
    let t = line.trim_start();
    t.starts_with("use ")
        || t.starts_with("impl ")
        || t.starts_with("impl<")
        || t.starts_with("let ")
        || t.starts_with("match ")
}

/// `name` of a `name: Type,` field line, or `None`.
fn field_name(line: &str) -> Option<String> {
    let t = line.trim().trim_end_matches(',').trim();
    let (name, _) = t.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(name.to_string())
}

/// `name` of a variant/discriminant line (`A(u32)`, `B {…}`, `C = 3`,
/// `D,`), or `None`.
fn variant_name(line: &str) -> Option<String> {
    let t = line.trim().trim_end_matches(',').trim();
    if t.is_empty() {
        return None;
    }
    let head = t.split(['(', '{', '=', ' ']).next().unwrap_or("").trim();
    if head.is_empty() || !head.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    if super::split_item(line.trim_end()).is_some() {
        return None;
    }
    Some(head.to_string())
}

fn first_minus_pat<'a>(body: &'a str, pats: &mut Vec<&'a str>) {
    for line in body.lines() {
        if let Some(pat) = line.strip_prefix("- ") {
            pats.push(pat);
            break;
        }
    }
}

/// Rebuild one run's native hunk text with covering ranges.
fn run_native_hunk(
    _header: &str,
    minuses: &[&str],
    pluses: &[&str],
    sites: &[usize],
    new_start: usize,
    start: usize,
    end: usize,
) -> String {
    let len = end - start;
    let mut out = format!(
        "@@ -{},{} +{},{} @@\n",
        sites[start],
        len,
        new_start + start,
        len
    );
    for i in start..end {
        out.push('-');
        out.push_str(minuses[i]);
        out.push('\n');
        out.push('+');
        out.push_str(pluses[i]);
        out.push('\n');
    }
    out
}

/// Parse `@@ -o[,ol] +n[,nl] @@` into `(new_start, new_len)`.
/// Old-side values come from `changed_old_lines`; only the new side is
/// needed for remainder rebuilds.
fn parse_hunk_ranges(header: &str) -> Option<(usize, usize)> {
    let inner = header.strip_prefix("@@")?.split("@@").next()?.trim();
    let mut parts = inner.split_whitespace();
    let _old = parts.next()?;
    let new = parts.next()?;
    let new = new.strip_prefix('+')?;
    let mut it = new.split(',');
    let start: usize = it.next()?.parse().ok()?;
    let len: usize = match it.next() {
        Some(l) => l.parse().ok()?,
        None => 1,
    };
    Some((start, len))
}

/// D13 composed probe: applying the composed fragment to a `pre` clone
/// must reproduce the native hunk's post tree byte-identically.
fn composed_reproduces_native(path: &str, pre: &str, fragment: &str, hunk: &str) -> bool {
    let section_of = |body: &str| format!("{}\n{}", crate::git_header(path), body);
    let Ok(composed) = rsmpl_patch::parse_patch(&section_of(fragment)) else {
        return false;
    };
    let Ok(native) = rsmpl_patch::parse_patch(&section_of(hunk)) else {
        return false;
    };
    let mut got = std::collections::HashMap::from([(path.to_string(), pre.to_string())]);
    let mut exp = std::collections::HashMap::from([(path.to_string(), pre.to_string())]);
    if rsmpl_engine::apply_patch_to_tree(&mut got, &composed).is_err() {
        return false;
    }
    if rsmpl_engine::apply_patch_to_tree(&mut exp, &native).is_err() {
        return false;
    }
    got == exp
}

/// Ident-boundary containment (never substring).
fn contains_ident(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut token = String::new();
    for c in haystack.chars().chain(std::iter::once(' ')) {
        if c == '_' || c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if token == needle {
                return true;
            }
            token.clear();
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre_map(pre: &str) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())])
    }

    fn applied(pre: &std::collections::HashMap<String, String>, fragment: &str) -> String {
        let section = format!("{}\n{}", crate::git_header("f.rs"), fragment);
        let parsed = rsmpl_patch::parse_patch(&section).expect("fragment must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        files["f.rs"].clone()
    }

    fn native_post(pre: &str, hunk: &str) -> String {
        let section = format!("{}\n{}", crate::git_header("f.rs"), hunk);
        let parsed = rsmpl_patch::parse_patch(&section).expect("native must parse");
        let mut files = pre_map(pre);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("native must apply");
        files["f.rs"].clone()
    }

    #[test]
    fn two_class_hunk_raises_per_group() {
        // F4 shape: a call rewrite and a general-expression rewrite in one
        // hunk — two contiguous runs, each semantic, in document order.
        let pre = "fn m() {\n    foo(alpha);\n    y + 1;\n}\n";
        let hunk =
            "@@ -2,2 +2,2 @@\n-    foo(alpha);\n-    y + 1;\n+    bar(alpha);\n+    y + 2;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("two-class hunk must split");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert!(
            out.contains("- foo(E0)"),
            "call run must raise, got:\n{out}"
        );
        assert!(out.contains("- E0 + 1"), "expr run must raise, got:\n{out}");
        // Document order preserved (call hunk before expr hunk).
        assert!(
            out.find("foo(E0)").unwrap() < out.find("E0 + 1").unwrap(),
            "order must hold, got:\n{out}"
        );
        assert_eq!(prov.len(), 2, "one entry per group: {prov:?}");
        assert!(applied(&pre_map(pre), &out) == native_post(pre, hunk));
    }

    #[test]
    fn unclaimable_run_stays_ordinary_remainder() {
        // Two const renames plus a use rename in one hunk: the item runs
        // raise as singles, the use pair (no use single exists) stays
        // ordinary verbatim in place, D13 still holds.
        let pre = "const OLD_A: usize = 1001;\nconst OLD_B: usize = 1002;\nuse crate::old_mod;\n";
        let hunk = "@@ -1,3 +1,3 @@\n-const OLD_A: usize = 1001;\n-const OLD_B: usize = 1002;\n-use crate::old_mod;\n+const NEW_A: usize = 1001;\n+const NEW_B: usize = 1002;\n+use crate::new_mod;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("claimable runs must raise with ordinary remainder");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert!(
            out.contains("-use crate::old_mod;\n+use crate::new_mod;\n"),
            "remainder verbatim, got:\n{out}"
        );
        assert_eq!(prov.len(), 2, "remainder carries no entry: {prov:?}");
        assert_eq!(applied(&pre_map(pre), &out), native_post(pre, hunk));
    }

    #[test]
    fn interleave_vetoes_whole_hunk() {
        // A B A line order stays ordinary — order is never manufactured.
        let pre =
            "const OLD_A: usize = 1001;\ntype AliasName = Vec<u8>;\nconst OLD_B: usize = 1002;\n";
        let hunk = "@@ -1,3 +1,3 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n-type AliasName = Vec<u8>;\n+type AliasName = Box<[u8]>;\n-const OLD_B: usize = 1002;\n+const NEW_B: usize = 1002;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new()
        )
        .is_none());
        assert!(used.is_empty());
        assert!(prov.is_empty());
    }

    #[test]
    fn joint_territory_struct_field_stays_whole() {
        // 99's mixed hunk: struct rename + its field-type evolution in one
        // hunk stays ordinary (WI-5/WI-21 joint territory, never split).
        let pre = "struct OldA {\n    x: i32,\n}\n";
        let hunk = "@@ -1,2 +1,2 @@\n-struct OldA {\n+struct NewA {\n-    x: i32,\n+    x: u32,\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        );
        assert!(
            out.is_none(),
            "joint territory must stay whole, got: {out:?}"
        );
        assert!(used.is_empty());
    }

    #[test]
    fn duplicate_minus_vetoes() {
        let pre = "const OLD_A: usize = 1001;\nconst OLD_A2: usize = 1001;\n";
        let hunk = "@@ -1,2 +1,2 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new()
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn suppressed_callee_poisons_hunk() {
        // WI-4a: a vetoed joint's hunk never splits, even when its runs
        // would classify cleanly.
        let pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\n";
        let hunk = "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n-    x + 1\n+    x + 2\n";
        let suppressed: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::from([("f.rs".to_string(), "foo".to_string())]);
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(
            try_subhunk_partition(hunk, "f.rs", Some(pre), &mut used, &mut prov, &suppressed)
                .is_none()
        );
        assert!(used.is_empty());
    }

    #[test]
    fn none_pre_image_stays_ordinary() {
        let hunk = "@@ -1,2 +1,2 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n-const OLD_B: usize = 1002;\n+const NEW_B: usize = 1002;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(try_subhunk_partition(
            hunk,
            "f.rs",
            None,
            &mut used,
            &mut prov,
            &std::collections::HashSet::new()
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn same_class_hetero_pairs_raise_as_singles() {
        // One run, hetero deltas: the multi offer declines (distinct
        // deltas share no template) but every pair proves alone, so the
        // hunk still raises pair by pair — the per-pair posture the
        // section-level partition owns for multi-hunk sections.
        let pre = "const OLD_A: usize = 1001;\nconst OLD_B: usize = 1002;\n";
        let hunk = "@@ -1,2 +1,2 @@\n-const OLD_A: usize = 1001;\n+const NEW_A: usize = 1001;\n-const OLD_B: usize = 1002;\n+const NEW_B: usize = 1002;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("same-class hetero pairs must raise as singles");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert_eq!(applied(&pre_map(pre), &out), native_post(pre, hunk));
    }

    #[test]
    fn none_run_splits_into_firing_multis() {
        // No single pair proves alone (each template matches twice), but
        // the run is two homogeneous multi sides — each proves as a
        // sweep, so the run splits instead of staying ordinary.
        let pre = "fn first() {\n    foo(alpha);\n    foo(beta);\n    y + 1;\n    z + 1;\n}\n";
        let hunk = "@@ -2,4 +2,4 @@\n-    foo(alpha);\n-    foo(beta);\n-    y + 1;\n-    z + 1;\n+    bar(alpha);\n+    bar(beta);\n+    y + 2;\n+    z + 2;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("none-run with two firing multis must split");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert!(
            out.contains("- foo(E0)"),
            "call side must raise, got:\n{out}"
        );
        assert!(
            out.find("foo(E0)").unwrap() < out.find("+ bar(E0)").unwrap(),
            "call side first, got:\n{out}"
        );
        assert_eq!(applied(&pre_map(pre), &out), native_post(pre, hunk));
    }

    #[test]
    fn firing_interleave_vetoes_whole_hunk() {
        // Both classes prove alone, but the A B A order would need
        // regrouping to emit — stays ordinary, order never manufactured.
        let pre = "fn m() {\n    foo(a);\n    y + 1;\n    baz(b);\n}\n";
        let hunk = "@@ -2,3 +2,3 @@\n-    foo(a);\n-    y + 1;\n-    baz(b);\n+    bar(a);\n+    y + 2;\n+    qux(b);\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new()
        )
        .is_none());
        assert!(used.is_empty());
        assert!(prov.is_empty());
    }

    #[test]
    fn mixed_method_vis_and_literal_splits() {
        // Per-hunk method singles (WI-14 slice-3 residue): a method-vis
        // pair beside a body-literal pair in one hunk raises the method
        // run as a `sig_evolution` single (four-guard method-run price);
        // the literal pair (no N=1 shape) stays ordinary verbatim in
        // place. D13 holds (byte-identical round-trip).
        let pre = "impl Svc {\n    fn alpha(x: i32) {\n        1\n    }\n}\n";
        let hunk = "@@ -2,2 +2,2 @@ impl Svc {\n-    fn alpha(x: i32) {\n-        1\n+    pub fn alpha(x: i32) {\n+        2\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("method run must claim with ordinary remainder");
        assert_eq!(out.matches("rule=").count(), 1, "got:\n{out}");
        assert!(out.contains("-     fn alpha(...) {"), "got:\n{out}");
        assert!(out.contains("+     pub fn alpha(...) {"), "got:\n{out}");
        assert!(
            out.contains("-        1\n+        2\n"),
            "literal remainder verbatim, got:\n{out}"
        );
        assert!(
            prov.iter().any(|p| p.detector == "sig_evolution"),
            "prov: {prov:?}"
        );
        assert_eq!(applied(&pre_map(pre), &out), native_post(pre, hunk));
    }

    #[test]
    fn mixed_long_header_method_vis_and_literal_splits() {
        // P3 long-header item: the same mixed single hunk under a ~90-char
        // generic `impl` header declines at HEAD (the scaled guard prices
        // the measured short-header filter, not this one). Per-emission
        // pricing adds exactly the header surplus, so the method run
        // claims and the literal stays ordinary verbatim in place.
        let hdr = "impl<T: Clone + std::fmt::Debug + Send + Sync> Svc<T> where T: Clone {";
        let pre = format!("{hdr}\n    fn alpha(x: i32) {{\n        1\n    }}\n}}\n");
        let hunk = "@@ -2,2 +2,2 @@ impl Svc {\n-    fn alpha(x: i32) {\n-        1\n+    pub fn alpha(x: i32) {\n+        2\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(&pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("long-header method run must claim with ordinary remainder");
        assert_eq!(out.matches("rule=").count(), 1, "got:\n{out}");
        assert!(out.contains("-     fn alpha(...) {"), "got:\n{out}");
        assert!(out.contains("+     pub fn alpha(...) {"), "got:\n{out}");
        assert!(
            out.contains("-        1\n+        2\n"),
            "literal remainder verbatim, got:\n{out}"
        );
        assert!(out.contains(hdr), "filter header verbatim, got:\n{out}");
        assert_eq!(applied(&pre_map(&pre), &out), native_post(&pre, hunk));
    }
    #[test]
    fn hetero_double_delta_partitions_into_singles() {
        // WI-13 residue slice A: two double-delta pairs of different
        // classes (distinct minus templates, each unique file-wide)
        // partition into two positional allow-one singles; the tree
        // re-applies byte-exact. Multi-line composition is owned here,
        // not by the sweep (which needs one shared class).
        let pre = "fn m() {\n    let a = oldmod::foo(1);\n    let b = othermod::baz(2);\n}\n";
        let hunk = "@@ -2,2 +2,2 @@\n-    let a = oldmod::foo(1);\n-    let b = othermod::baz(2);\n+    let a = newmod::bar(1);\n+    let b = newmod::qux(2);\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("hetero double-delta hunk must partition");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert!(out.contains("- oldmod::foo(E0)\n"), "got:\n{out}");
        assert!(out.contains("+ newmod::bar(E0)\n"), "got:\n{out}");
        assert!(out.contains("- othermod::baz(E0)\n"), "got:\n{out}");
        assert!(out.contains("+ newmod::qux(E0)\n"), "got:\n{out}");
        assert!(
            out.find("oldmod::foo(E0)").unwrap() < out.find("othermod::baz(E0)").unwrap(),
            "document order, got:\n{out}"
        );
        assert_eq!(prov.len(), 2, "one entry per run: {prov:?}");
        assert_eq!(applied(&pre_map(pre), &out), native_post(pre, hunk));
    }

    #[test]
    fn hetero_double_delta_with_foreign_pair_claims_partial() {
        // WI-13 residue slice B: the typed-let line now fires its
        // shared-prefix joint (type + init call, two-hunk one-rule
        // factored) beside the provable double-delta neighbor — foreign
        // lines never veto firing neighbors, and joint lines are no
        // longer foreign.
        let pre =
            "fn m() {\n    let a = oldmod::foo(1);\n    let w: oldmod::Foo = oldmod::bar(2);\n}\n";
        let hunk = "@@ -2,2 +2,2 @@\n-    let a = oldmod::foo(1);\n-    let w: oldmod::Foo = oldmod::bar(2);\n+    let a = newmod::bar(1);\n+    let w: newmod::Foo = newmod::bar(2);\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("both runs must claim");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert!(out.contains("- oldmod::foo(E0)\n"), "got:\n{out}");
        assert!(
            out.contains("- oldmod::Foo\n"),
            "joint type half, got:\n{out}"
        );
        assert!(
            out.contains("- oldmod::bar(E0)\n"),
            "joint call half, got:\n{out}"
        );
        assert!(
            !out.contains("let w: oldmod::Foo"),
            "no ordinary remainder: {out}"
        );
        assert_eq!(applied(&pre_map(pre), &out), native_post(pre, hunk));
    }

    #[test]
    fn hetero_double_delta_shared_minus_falls_back_to_verbatim() {
        // Shared-minus-template hetero pairs veto each holed single by
        // occurrence totality (found 2 vs want 1), so the run raises
        // the whole-line verbatim singles instead — the literal floor
        // owns identical-template shapes, holed patterns never bypass
        // it. Byte-identical round-trip either way.
        let pre = "fn m() {\n    let a = oldmod::foo(1);\n    let b = oldmod::foo(2);\n}\n";
        let hunk = "@@ -2,2 +2,2 @@\n-    let a = oldmod::foo(1);\n-    let b = oldmod::foo(2);\n+    let a = newmod::bar(1);\n+    let b = newmod::baz(2);\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("shared-minus hetero must raise verbatim singles");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert!(
            out.contains("- let a = oldmod::foo(1);\n"),
            "framed literal floor, got:\n{out}"
        );
        assert!(
            out.contains("- let b = oldmod::foo(2);\n"),
            "framed literal floor, got:\n{out}"
        );
        assert!(!out.contains("foo(E0)"), "no holed bypass: {out}");
        assert_eq!(applied(&pre_map(pre), &out), native_post(pre, hunk));
    }

    #[test]
    fn hetero_double_delta_interleave_stays_ordinary() {
        // Same-key double-delta runs on both sides of a foreign line
        // (A B A) stay ordinary — order is never manufactured, even
        // though each flank proves alone. The order law owns this
        // shape, not a future detector. The middle line here is truly
        // foreign (Ty AND callee change — the slice-B joint declines
        // both), so the A-B-A key pattern (call_sweep, None,
        // call_sweep) still vetoes after slice B.
        let pre = "fn m() {\n    let a = oldmod::foo(1);\n    let w: oldmod::Foo = oldmod::bar(2);\n    let c = othermod::qux(3);\n}\n";
        let hunk = "@@ -2,3 +2,3 @@\n-    let a = oldmod::foo(1);\n-    let w: oldmod::Foo = oldmod::bar(2);\n-    let c = othermod::qux(3);\n+    let a = newmod::bar(1);\n+    let w: newmod::Qux = newmod::qux(2);\n+    let c = newmod::quux(3);\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(
            try_subhunk_partition(
                hunk,
                "f.rs",
                Some(pre),
                &mut used,
                &mut prov,
                &std::collections::HashSet::new()
            )
            .is_none(),
            "interleaved same-key runs must stay ordinary"
        );
        assert!(used.is_empty());
        assert!(prov.is_empty());
    }

    #[test]
    fn partition_takes_no_declined_by_construction() {
        // WI-15 P3: the partition driver takes no `declined` vec — the
        // single-pair/run classification probes are speculative offers
        // on a throwaway, not chain attempts (the whole-hunk attempt
        // that precedes the split already owns per-family evidence).
        // Pinned by signature: no caller vec can be polluted.
        let pre = "fn m() {\n    foo(alpha);\n    y + 1;\n}\n";
        let hunk =
            "@@ -2,2 +2,2 @@\n-    foo(alpha);\n-    y + 1;\n+    bar(alpha);\n+    y + 2;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("two-class hunk must split");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert_eq!(applied(&pre_map(pre), &out), native_post(pre, hunk));
    }
}
