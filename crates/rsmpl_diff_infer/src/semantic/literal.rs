//! Same-callee trailing-literal-delta call sweep (T448, split from
//! `super` for the 2000-line hygiene gate — peel.rs precedent) plus
//! its cross-hunk section driver (T449: singleton `-U0` hunks in an
//! unbalanced section group by literal class, peel precedent).

use super::{call_parts, flat_ws, parse_call_expr, unique_rule};

/// One hetero trailing-literal-delta class (T449): shared callee,
/// uniform arity `k + 1`, token-equal trailing old/new literals.
/// Leading args bind positionally per site (hetero values are the
/// point); the trailing pair replays verbatim.
#[derive(Debug, PartialEq, Eq)]
struct LiteralClass {
    cal: String,
    prefix: usize,
    old_lit: String,
    new_lit: String,
}

/// Parse one `-`/`+` line pair as a trailing-literal-delta call
/// migration: same callee, same arity `k + 1`, per-site leading `k`
/// args token-equal minus-vs-plus, trailing old/new token-equal with
/// a real delta. `let`-init sites unwrap to their call init (T449:
/// `parse_call_expr` handles the `Stmt::Local` form); the class key
/// carries only the shared parts.
fn parse_literal_pair(old_line: &str, new_line: &str) -> Option<LiteralClass> {
    use quote::ToTokens;
    let tok = |e: &syn::Expr| e.to_token_stream().to_string();
    let (o_cal, o_args) = call_parts(&parse_call_expr(old_line)?)?;
    let (n_cal, n_args) = call_parts(&parse_call_expr(new_line)?)?;
    if o_cal != n_cal || o_args.len() != n_args.len() || o_args.is_empty() {
        return None;
    }
    if !o_args[..o_args.len() - 1]
        .iter()
        .zip(n_args[..n_args.len() - 1].iter())
        .all(|(a, b)| tok(a) == tok(b))
    {
        return None;
    }
    let (o_last, n_last) = (
        tok(&o_args[o_args.len() - 1]),
        tok(&n_args[n_args.len() - 1]),
    );
    if o_last == n_last {
        return None;
    }
    // Trailing macros stay ordinary: the sweep inlines the literal as
    // TokenStream-normalized text, which the engine cannot match when a
    // macro hides inside (see `contains_macro_arg`).
    if contains_macro_arg(&o_args[o_args.len() - 1])
        || contains_macro_arg(&n_args[n_args.len() - 1])
    {
        return None;
    }
    Some(LiteralClass {
        cal: o_cal,
        prefix: o_args.len() - 1,
        old_lit: o_last,
        new_lit: n_last,
    })
}
/// True when an argument expression holds a macro invocation anywhere
/// inside: the literal sweep inlines the trailing literal as
/// TokenStream-normalized text, which the engine cannot match when a
/// `!` invocation hides inside (`wrap(old_mac ! (1))` matches nothing,
/// so diffrs would emit a patch that fails `--check` — WI-9b2-delim
/// probe, paren and bracket alike). Those classes stay ordinary for
/// the nested macro sweep; boolean `!`/`!=` never contain a macro node
/// and are unaffected.
fn contains_macro_arg(e: &syn::Expr) -> bool {
    struct Finder(bool);
    impl<'ast> syn::visit::Visit<'ast> for Finder {
        fn visit_expr_macro(&mut self, _: &'ast syn::ExprMacro) {
            self.0 = true;
        }
    }
    let mut f = Finder(false);
    syn::visit::Visit::visit_expr(&mut f, e);
    f.0
}

/// Cross-hunk literal driver for non-whole sections (T449): mirrors
/// the `try_call_peel_section` skeleton — group balanced-hunk pairs
/// by literal class, emit one hunk per passing hetero class, leave
/// everything else ordinary for verbatim + per-hunk to retry.
/// Literal bodies are `@@ -1,1 @@` single-pattern hunks, so the later
/// verbatim pass groups them at n=1 (skipped) and passes them through
/// unchanged.
pub(crate) fn try_call_literal_section(
    section: &str,
    _path: &str,
    used_rules: &mut Vec<String>,
    file_old: Option<&str>,
    fired: &mut Vec<(String, &'static str)>,
) -> Option<String> {
    let (prelude, hunks) = split_section_hunks(section);
    if hunks.is_empty() {
        return None;
    }
    let hunk_changed: Vec<Vec<usize>> = hunks
        .iter()
        .map(|h| crate::totality::changed_old_lines(h))
        .collect();
    // Balanced-hunk pairs in order; unbalanced hunks stay leftovers.
    let mut pairs: Vec<(usize, String, String)> = Vec::new();
    let mut pair_old: Vec<usize> = Vec::new();
    let mut hunk_balanced = vec![false; hunks.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        let mut minuses = Vec::new();
        let mut pluses = Vec::new();
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            }
        }
        if minuses.len() == pluses.len() && !minuses.is_empty() {
            hunk_balanced[i] = true;
            for (k, (m, p)) in minuses.into_iter().zip(pluses).enumerate() {
                let old_line = hunk_changed[i].get(k).copied().unwrap_or(0);
                pairs.push((i, m, p));
                pair_old.push(old_line);
            }
        }
    }
    // Group pair indices by literal class, first-occurrence order.
    let mut classes: Vec<(LiteralClass, Vec<usize>)> = Vec::new();
    for (idx, (.., m, p)) in pairs.iter().enumerate() {
        let Some(c) = parse_literal_pair(m, p) else {
            continue;
        };
        match classes.iter_mut().find(|(k, _)| *k == c) {
            Some((_, members)) => members.push(idx),
            None => classes.push((c, vec![idx])),
        }
    }
    let checkpoint = used_rules.len();
    // Fire at most one hunk per hetero class (all-identical classes
    // stay for verbatim, which is tighter there — T422 precedent).
    // Candidates reserve no rules: the atomicity fixed point below
    // drops violators first (macros.rs aa23c77 precedent), so dropped
    // classes burn no `rN` names and leave no stale `fired` records.
    struct Candidate {
        first: usize,
        tail: String,
        sites: Vec<usize>,
        members: Vec<usize>,
        label: String,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (class, members) in &classes {
        if members.len() < 2 {
            continue;
        }
        let first = &pairs[members[0]];
        if members.iter().all(|&idx| {
            flat_ws(&pairs[idx].1) == flat_ws(&first.1)
                && flat_ws(&pairs[idx].2) == flat_ws(&first.2)
        }) {
            continue;
        }
        let ms: Vec<&str> = members.iter().map(|&idx| pairs[idx].1.as_str()).collect();
        let mut group_changed: Vec<usize> = Vec::new();
        for &idx in members {
            for &l in &hunk_changed[pairs[idx].0] {
                if !group_changed.contains(&l) {
                    group_changed.push(l);
                }
            }
        }
        if !crate::totality::call_literal_holds(
            file_old,
            &class.cal,
            class.prefix + 1,
            &class.old_lit,
            &ms,
            &group_changed,
        ) {
            continue;
        }
        let mut vars = Vec::new();
        for i in 0..class.prefix {
            vars.push(format!("E{i}"));
        }
        let head = vars.join(", ");
        // WI-7b: uppercase same callee rides backticks (literal), else it
        // infers as an `expr` hole and over-matches.
        let lit = super::shape::literal_callee(&class.cal);
        let (minus_call, plus_call) = if head.is_empty() {
            (
                format!("{}({})", lit, class.old_lit),
                format!("{}({})", lit, class.new_lit),
            )
        } else {
            (
                format!("{}({head}, {})", lit, class.old_lit),
                format!("{}({head}, {})", lit, class.new_lit),
            )
        };
        let tail = format!("- {minus_call}\n+ {plus_call}\n");
        let sites: Vec<usize> = members.iter().map(|&idx| pair_old[idx]).collect();
        candidates.push(Candidate {
            first: members[0],
            tail,
            sites,
            members: members.clone(),
            label: format!("calls_{}_literal", class.cal),
        });
    }
    // Hunk-atomicity fixed point (macros.rs aa23c77): a class fires
    // only if every balanced hunk holding a member pair is fully
    // covered by passing classes — otherwise the class hunk and the
    // leftover ordinary hunk would rewrite the same line
    // (GIT_HUNK_CONFLICT overlapping edits), or — when the first
    // member sits in the uncovered hunk — the body would never emit
    // and covered hunks would silently drop. Drop violators to a
    // fixed point (cascade: dropping uncovers hunks).
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
        candidates.remove(gi);
    }
    if candidates.is_empty() {
        return None;
    }
    // Reserve rules + WI-2 detector records for survivors only.
    let mut passing: Vec<(usize, String, Vec<usize>, String)> = Vec::new();
    let mut covered_pair = vec![false; pairs.len()];
    for cand in candidates {
        let rule = unique_rule(used_rules, &cand.label);
        // WI-2: exact detector record (commit filters by presence).
        fired.push((rule.clone(), "call_literal_section"));
        for &idx in &cand.members {
            covered_pair[idx] = true;
        }
        passing.push((cand.first, cand.tail, cand.sites, rule));
    }
    // WI-1b: ranged bodies per group when spans are disjoint and ≤64;
    // overlapping groups share the sweep dummy (SPEC 10.6).
    let mut spans: Vec<Option<(usize, usize)>> = Vec::new();
    for (_, _, sites, _) in &passing {
        if sites.contains(&0) {
            spans.push(None);
            continue;
        }
        let min = sites.iter().min().copied().unwrap_or(0);
        let max = sites.iter().max().copied().unwrap_or(0);
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
        .map(|(gi, (_, tail, sites, rule))| {
            if overlap[gi] || spans[gi].is_none() {
                format!("@@ -1,1 @@ rule={rule}\n# sweep\n{tail}")
            } else {
                format!("{}{tail}", crate::emission_body(rule, sites))
            }
        })
        .collect();
    // A hunk is covered iff balanced and every pair sits in a passing
    // class; leftover hunks stay ordinary in place.
    let covered_hunk: Vec<bool> = hunk_balanced
        .iter()
        .enumerate()
        .map(|(i, b)| {
            *b && pairs
                .iter()
                .enumerate()
                .filter(|(_, (j, _, _))| *j == i)
                .all(|(idx, _)| covered_pair[idx])
        })
        .collect();
    // T375 over the literal-covered set: the leftover retry is
    // fallback-identical, so the section beats the fallback exactly
    // when the literal bodies beat the covered ordinary hunks.
    let covered_len: usize = hunks
        .iter()
        .zip(&covered_hunk)
        .filter(|(_, b)| **b)
        .map(|(h, _)| h.len())
        .sum();
    if bodies.iter().map(|b| b.len()).sum::<usize>() >= covered_len {
        used_rules.truncate(checkpoint);
        return None;
    }
    let passing_idx: Vec<(usize, String)> = passing
        .iter()
        .enumerate()
        .map(|(gi, (first, _, _, _))| (*first, bodies[gi].clone()))
        .collect();
    let mut out = prelude;
    let mut emitted = vec![false; passing_idx.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        if covered_hunk[i] {
            for (gi, (first_idx, body)) in passing_idx.iter().enumerate() {
                if !emitted[gi] && pairs[*first_idx].0 == i {
                    out.push_str(body);
                    emitted[gi] = true;
                }
            }
            continue;
        }
        out.push_str(hunk);
    }
    Some(out)
}

/// Split a section into its prelude lines and native hunks (the same
/// split the per-hunk fallback uses; each part keeps trailing newlines).
fn split_section_hunks(section: &str) -> (String, Vec<String>) {
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

/// N-site (N >= 2) same-callee same-arity trailing-literal-delta call
/// sweep (T448): every pair parses as a bare single-seg no-turbofish
/// call with one shared callee and uniform arity `k + 1`; each site's
/// leading `k` args token-equal minus-vs-plus (bound as `E0…`, hetero
/// across sites allowed); the trailing arg is token-equal concrete
/// text across ALL sites on each side with a real old/new delta
/// (`false` -> `"100644"`, never a plus-only metavar — T442-safe by
/// construction). Pre-image totality keys on the trailing old literal
/// (`call_literal_holds`, T420-style): same-arity sites with a
/// different trailing literal (7025a14's `true` site next to the 2x
/// `false` prize group) never match the pattern and must not veto.
pub(crate) fn try_call_literal_sweep(
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
    use quote::ToTokens;
    let tok = |e: &syn::Expr| e.to_token_stream().to_string();
    let mut cal: Option<String> = None;
    let mut prefix: Option<usize> = None;
    let mut old_lit: Option<String> = None;
    let mut new_lit: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o_cal, o_args) = call_parts(&parse_call_expr(m)?)?;
        let (n_cal, n_args) = call_parts(&parse_call_expr(p)?)?;
        if o_cal != n_cal || o_args.len() != n_args.len() || o_args.is_empty() {
            return None;
        }
        if cal.as_ref().is_some_and(|c| *c != o_cal) {
            return None;
        }
        if prefix.is_some_and(|k| k + 1 != o_args.len()) {
            return None;
        }
        if !o_args[..o_args.len() - 1]
            .iter()
            .zip(n_args[..n_args.len() - 1].iter())
            .all(|(a, b)| tok(a) == tok(b))
        {
            return None;
        }
        let (o_last, n_last) = (
            tok(&o_args[o_args.len() - 1]),
            tok(&n_args[n_args.len() - 1]),
        );
        if old_lit.as_ref().is_some_and(|l| *l != o_last) {
            return None;
        }
        if new_lit.as_ref().is_some_and(|l| *l != n_last) {
            return None;
        }
        // Trailing macros stay ordinary (see `contains_macro_arg`):
        // TokenStream-normalized macro text is unmatchable inlining.
        if contains_macro_arg(&o_args[o_args.len() - 1])
            || contains_macro_arg(&n_args[n_args.len() - 1])
        {
            return None;
        }
        cal = Some(o_cal);
        prefix = Some(o_args.len() - 1);
        old_lit = Some(o_last);
        new_lit = Some(n_last);
    }
    let (cal, k, old_lit, new_lit) = (cal?, prefix?, old_lit?, new_lit?);
    if old_lit == new_lit {
        return None;
    }
    // Hetero required (peel precedent): an all-identical class compresses
    // tighter as a verbatim hunk (no `# expr` directives, shorter rule
    // name — T422 closed this shape for verbatim), so leave it there.
    if minuses
        .iter()
        .zip(pluses.iter())
        .all(|(m, p)| flat_ws(m) == flat_ws(minuses[0]) && flat_ws(p) == flat_ws(pluses[0]))
    {
        return None;
    }
    if !crate::totality::call_literal_holds(file_old, &cal, k + 1, &old_lit, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{cal}_literal"));
    let mut hunk = crate::emission_header(path, &rule, changed);
    let mut vars = Vec::new();
    for i in 0..k {
        let v = format!("E{i}");
        vars.push(v);
    }
    let head = vars.join(", ");
    // WI-7b: uppercase same callee rides backticks (literal).
    let lit = super::shape::literal_callee(&cal);
    let (minus_call, plus_call) = if head.is_empty() {
        (format!("{lit}({old_lit})"), format!("{lit}({new_lit})"))
    } else {
        (
            format!("{lit}({head}, {old_lit})"),
            format!("{lit}({head}, {new_lit})"),
        )
    };
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    Some(hunk)
}

#[cfg(test)]
mod tests {
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
    fn call_literal_two_sites_fire() {
        // T448: hetero leading bound as E-vars, homo trailing literal delta.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n}\n",
        )]);
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,2 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(out.contains("- make_widget(E0, E1, false)\n"), "got: {out}");
        assert!(
            out.contains("+ make_widget(E0, E1, \"100644\")\n"),
            "got: {out}"
        );
    }

    #[test]
    fn call_literal_all_identical_stays_for_verbatim() {
        // T448 + T422: an all-identical class (here zero-prefix) compresses
        // tighter without metavars, so the literal sweep declines and the
        // verbatim sweep claims it.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    make_widget(false);\n-    make_widget(false);\n+    make_widget(\"100644\");\n+    make_widget(\"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(false);\n    make_widget(false);\n}\n",
        )]);
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(!out.contains("calls_make_widget_literal"), "got: {out}");
    }

    #[test]
    fn call_literal_hetero_trailing_vetoes() {
        // T448: `false` vs `true` trailing is unexpressible (plus-side
        // metavars would bind nothing, T442).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, false);\n+    make_widget(beta, 2, true);\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_literal_changed_prefix_vetoes() {
        // T448: a leading arg rewritten in transit is not a literal delta.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 9, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_literal_extra_same_literal_vetoes() {
        // T448 tuple totality: an unchanged third same-trailing call in
        // the pre-image over-counts (3 != 2).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n    make_widget(gamma, 3, false);\n}\n",
        )]);
        assert!(
            crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn call_literal_extra_different_trailing_fires() {
        // T448 point: a same-arity site with a DIFFERENT trailing literal
        // (`true`, the 7025a14 shape) never matches the `false` pattern
        // and must not veto.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n    make_widget(gamma, 3, true);\n}\n",
        )]);
        let out = crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
    }

    #[test]
    fn call_literal_single_site_vetoes() {
        // T448: lone literal deltas keep the ordinary hunk (no token win).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n+    make_widget(alpha, 1, \"100644\");\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_literal_macro_arg_falls_through_to_nested() {
        // WI-9b2-delim: a trailing literal hiding a macro invocation is
        // not a literal — TokenStream-normalized macro text is
        // unmatchable, so the literal sweep declines and the nested macro
        // sweep claims the pair (validated_live proves the fall-through
        // applies cleanly).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = wrap(old_mac!(1));\n-    let second_value = wrap(old_mac!(1));\n+    let first_value = wrap(old_mac!(2));\n+    let second_value = wrap(old_mac!(2));\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_value = wrap(old_mac!(1));\n    let second_value = wrap(old_mac!(1));\n}\n",
        )]);
        let out = crate::semantic::infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            !out.contains("calls_wrap_literal"),
            "literal must not claim, got: {out}"
        );
        assert!(
            out.contains("- old_mac!(1)\n"),
            "nested must fire, got: {out}"
        );
        assert!(out.contains("+ old_mac!(2)\n"), "got: {out}");
    }

    #[test]
    fn call_literal_same_literal_vetoes() {
        // T448: no old/new delta (identical lines) is not a sweep.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, false);\n+    make_widget(beta, 2, false);\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_literal_let_init_whole_fires() {
        // T449: call sites inside `let` inits take the whole-section
        // path once `parse_call_expr` unwraps `Stmt::Local`; the
        // emission stays a bare-call pattern and the engine rewrites
        // the call sub-expression in place.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let a = make_widget(alpha, 1, false);\n-    let b = make_widget(beta, 2, false);\n+    let a = make_widget(alpha, 1, \"100644\");\n+    let b = make_widget(beta, 2, \"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = make_widget(alpha, 1, false);\n    let b = make_widget(beta, 2, false);\n}\n",
        )]);
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,2 @@ rule=r1\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("- make_widget(E0, E1, false)\n"), "got: {out}");
        assert!(
            out.contains("+ make_widget(E0, E1, \"100644\")\n"),
            "got: {out}"
        );
    }

    #[test]
    fn call_literal_let_init_cross_hunk_fires() {
        // T449 prize miniature: two singleton `-U0` hunks (spaced
        // `let` sites) plus a pure-addition hunk that vetoes
        // whole-section balance — the section driver groups the
        // singletons into one sweep; the addition stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let a = make_widget(alpha, 1, false);\n+    let a = make_widget(alpha, 1, \"100644\");\n@@ -9 +9 @@\n-    let b = make_widget(beta, 2, false);\n+    let b = make_widget(beta, 2, \"100644\");\n@@ -20,0 +21,1 @@\n+    let cached = extra();\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = make_widget(alpha, 1, false);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    let b = make_widget(beta, 2, false);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n}\n",
        )]);
        let out = crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("- make_widget(E0, E1, false)\n"), "got: {out}");
        assert!(
            out.contains("+ make_widget(E0, E1, \"100644\")\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+    let cached = extra();\n"),
            "addition stays, got: {out}"
        );
        assert!(
            !out.contains("-    let a = make_widget(alpha, 1, false);"),
            "native minus consumed, got: {out}"
        );
        assert!(out.contains("@@ -2,8 @@ rule=r1\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn call_literal_cross_hunk_single_member_stays_ordinary() {
        // T449: a lone singleton class (N < 2) has no token win — the
        // section stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let a = make_widget(alpha, 1, false);\n+    let a = make_widget(alpha, 1, \"100644\");\n@@ -20,0 +21,1 @@\n+    let cached = extra();\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_literal_cross_hunk_line_hetero_call_homo_fires() {
        // T449: `let a` vs `let b` lines are hetero, so verbatim can
        // never group them — but the call migration is one homo
        // class, and the zero-prefix literal sweep (no `# expr`
        // directives) rewrites both inits in place. Engine-checked.
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let a = make_widget(false);\n+    let a = make_widget(\"100644\");\n@@ -9 +9 @@\n-    let b = make_widget(false);\n+    let b = make_widget(\"100644\");\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = make_widget(false);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    let b = make_widget(false);\n}\n",
        )]);
        let out = crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,8 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("- make_widget(false)\n"), "got: {out}");
        assert!(out.contains("+ make_widget(\"100644\")\n"), "got: {out}");
    }

    #[test]
    fn call_literal_cross_hunk_all_identical_stays_for_verbatim() {
        // T449 + T422: a line-identical cross-hunk class compresses
        // tighter without metavars (`rule=r1` beats the longer
        // literal rule on byte-identical bodies), so the section
        // driver declines and verbatim claims it.
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    make_widget(false);\n+    make_widget(\"100644\");\n@@ -9 +9 @@\n-    make_widget(false);\n+    make_widget(\"100644\");\n",
        );
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(!out.contains("calls_make_widget_literal"), "got: {out}");
    }

    #[test]
    fn call_literal_cross_hunk_extra_same_literal_vetoes() {
        // T449 tuple totality across hunks: an unchanged third
        // same-trailing call in the pre-image over-counts (3 != 2).
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let a = make_widget(alpha, 1, false);\n+    let a = make_widget(alpha, 1, \"100644\");\n@@ -9 +9 @@\n-    let b = make_widget(beta, 2, false);\n+    let b = make_widget(beta, 2, \"100644\");\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = make_widget(alpha, 1, false);\n    let b = make_widget(beta, 2, false);\n    let c = make_widget(gamma, 3, false);\n}\n",
        )]);
        assert!(
            crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn call_let_init_changed_prefix_vetoes() {
        // T449 guard: a leading arg rewritten in transit is not a
        // literal delta, through the `let` unwrap too.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let a = make_widget(alpha, 1, false);\n-    let b = make_widget(beta, 2, false);\n+    let a = make_widget(alpha, 9, \"100644\");\n+    let b = make_widget(beta, 2, \"100644\");\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn literal_mixed_hunk_atomicity_drops_class() {
        // WI-9e: H1 holds a literal-delta pair plus a singleton; H2..H5
        // hold the same hetero literal class. The class must drop (its
        // first member sits in uncovered H1, so the body could never
        // emit there and covered hunks would silently drop) — inference
        // declines and the section falls through to ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@ pub fn a() {\n-    check(c1, \"a\");\n-    ping(1);\n+    check(c1, \"b\");\n+    ping(2);\n@@ -6 +6 @@ pub fn b() {\n-    check(c2, \"a\");\n+    check(c2, \"b\");\n@@ -9 +9 @@ pub fn c() {\n-    check(c3, \"a\");\n+    check(c3, \"b\");\n@@ -12 +12 @@ pub fn d() {\n-    check(c4, \"a\");\n+    check(c4, \"b\");\n@@ -15 +15 @@ pub fn e() {\n-    check(c5, \"a\");\n+    check(c5, \"b\");\n@@ -21 +20,0 @@ pub fn gone() {\n-    drop();\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "pub fn a() {\n    check(c1, \"a\");\n    ping(1);\n}\npub fn b() {\n    check(c2, \"a\");\n}\npub fn c() {\n    check(c3, \"a\");\n}\npub fn d() {\n    check(c4, \"a\");\n}\npub fn e() {\n    check(c5, \"a\");\n}\npub fn filler() {\n    keep();\n}\npub fn gone() {\n    drop();\n}\n",
        )]);
        assert!(
            crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn literal_fully_covered_class_fires() {
        // WI-9e control: every member hunk is fully covered — the fixed
        // point drops nothing and the class emits.
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@ pub fn a() {\n-    check(c1, \"a\");\n+    check(c1, \"b\");\n@@ -5 +5 @@ pub fn b() {\n-    check(c2, \"a\");\n+    check(c2, \"b\");\n@@ -8 +8 @@ pub fn c() {\n-    check(c3, \"a\");\n+    check(c3, \"b\");\n@@ -14 +13,0 @@ pub fn gone() {\n-    drop();\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "pub fn a() {\n    check(c1, \"a\");\n}\npub fn b() {\n    check(c2, \"a\");\n}\npub fn c() {\n    check(c3, \"a\");\n}\npub fn filler() {\n    keep();\n}\npub fn gone() {\n    drop();\n}\n",
        )]);
        let out = crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("check(E0,"), "got: {out}");
        assert!(
            !out.contains("check(c1, \"a\");"),
            "covered lines must be consumed, got: {out}"
        );
    }
}
