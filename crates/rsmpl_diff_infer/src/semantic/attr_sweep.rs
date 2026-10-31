//! Attribute-delta raising (WI-10b): uniform attr-line deltas over hetero items.
//!
//! Pure-attr hunks — bare `+#[...]` insertions, bare `-#[...]` deletions,
//! and balanced attr-line replacements — share one uniform delta across
//! N ≥ 2 sites but carry no header text, so no pair template can name
//! them. This detector recovers owners from the pre-image (`syn` walk:
//! the innermost item containing the changed line, or the line after an
//! insertion point), groups hunks by (mode, attr text, owner family), and
//! emits one hunk per class in the WI-10a authored spelling: the fn
//! N-hole form (`- V Q fn N G (...) W {`, e2e-proven to match `-> T`
//! sites) or the bare-pair item form (`- struct N`, vis/generics
//! preserved like renames). Heterogeneous names ride `N`; uniform names
//! stay concrete. Methods, `impl` owners, mixed attr+code hunks, and
//! unparseable pre-images decline (v1 limits, honest per SPEC 2.6).
//!
//! Two entries share the class machinery: `try_attr_replace_sweep` for
//! the whole-balanced pair chain (homogeneous single-line replace
//! sections — bench 38's shape), and `try_attr_sweep_section` as a
//! chain link for the non-whole path (insert/delete/replace groups;
//! claimed hunks rewritten in place, everything else byte-identical for
//! verbatim + per-hunk to retry — the peel precedent). Mixed
//! whole-balanced sections stay ordinary (pair-chain homogeneity; the
//! per-hunk path cannot beat T375 on singles).
//!
//! Totality is the scratch-apply probe: the emitted hunk must apply to
//! the pre-image with `matches == class size`. An untouched same-state
//! twin matches and vetoes; an already-migrated site is skipped by the
//! engine's subset semantics and never vetoes — exactly the IMPROVE
//! totality rule. T375 guards the covered set; reservations truncate on
//! failure.

use super::peel::split_section_hunks;
use super::unique_rule;
use std::collections::HashMap;

/// Attribute-delta mode for one hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttrMode {
    Insert,
    Delete,
    Replace,
}

/// One owner family: free/block fns, or one bare-pair item kind
/// (`struct`, `enum`, `trait`, `union`, `type`, `const`, `static`, `mod`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum OwnerFamily {
    Fn,
    Item(String),
}

/// One class of pure-attr hunks: uniform mode + attr text + family.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AttrClass {
    mode: AttrMode,
    minus: Vec<String>,
    plus: Vec<String>,
    family: OwnerFamily,
}

/// One discovered owner: an ownable item in the pre-image.
struct Owner {
    family: OwnerFamily,
    name: String,
    start: usize,
    end: usize,
}

/// True for outer-attribute and doc-comment diff lines.
fn is_attr_line(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("#[") || t.starts_with("///")
}

/// Grouping key text for one attr diff line: trimmed raw text. Indent
/// never survives the `-`/`+` strip comparably (the engine matches
/// whitespace-insensitively and replays site indent), but within-line
/// spacing stays exact so the emission real-applies byte-identical.
fn key_attr(s: &str) -> String {
    s.trim().to_string()
}

/// Classify one hunk's minus/plus diff lines (already stripped of the
/// `-`/`+` prefix). `None` when the hunk is not a pure-attr delta:
/// mixed lines, blank lines, or no delta.
fn classify_hunk(
    minuses: &[String],
    pluses: &[String],
) -> Option<(AttrMode, Vec<String>, Vec<String>)> {
    if minuses
        .iter()
        .chain(pluses.iter())
        .any(|l| l.trim().is_empty())
    {
        return None;
    }
    if minuses
        .iter()
        .chain(pluses.iter())
        .any(|l| !is_attr_line(l))
    {
        return None;
    }
    let minus: Vec<String> = minuses.iter().map(|l| key_attr(l)).collect();
    let plus: Vec<String> = pluses.iter().map(|l| key_attr(l)).collect();
    if minus.is_empty() && !plus.is_empty() {
        return Some((AttrMode::Insert, minus, plus));
    }
    if !minus.is_empty() && plus.is_empty() {
        return Some((AttrMode::Delete, minus, plus));
    }
    if minus.len() == plus.len() && minus != plus {
        return Some((AttrMode::Replace, minus, plus));
    }
    None
}

/// Ownable items in the pre-image: free fns (any nesting — methods are
/// a different node type and never collected here) and bare-pair items.
/// `impl` blocks contribute nothing and are pruned (methods need
/// ImplContext spelling, a v1 non-goal); `trait` contributes itself but
/// is not descended into; inline `mod` contributes itself and is
/// descended into.
fn collect_owners(file: &syn::File) -> Vec<Owner> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;
    struct Walk {
        owners: Vec<Owner>,
    }
    impl Walk {
        fn push(&mut self, family: OwnerFamily, name: String, span: proc_macro2::Span) {
            self.owners.push(Owner {
                family,
                name,
                start: span.start().line.max(1),
                end: span.end().line.max(span.start().line),
            });
        }
    }
    impl<'ast> Visit<'ast> for Walk {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            self.push(OwnerFamily::Fn, node.sig.ident.to_string(), node.span());
            syn::visit::visit_item_fn(self, node);
        }
        fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
            self.push(
                OwnerFamily::Item("struct".to_string()),
                node.ident.to_string(),
                node.span(),
            );
            syn::visit::visit_item_struct(self, node);
        }
        fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
            self.push(
                OwnerFamily::Item("enum".to_string()),
                node.ident.to_string(),
                node.span(),
            );
            syn::visit::visit_item_enum(self, node);
        }
        fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
            self.push(
                OwnerFamily::Item("union".to_string()),
                node.ident.to_string(),
                node.span(),
            );
            syn::visit::visit_item_union(self, node);
        }
        fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
            self.push(
                OwnerFamily::Item("type".to_string()),
                node.ident.to_string(),
                node.span(),
            );
            syn::visit::visit_item_type(self, node);
        }
        fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
            self.push(
                OwnerFamily::Item("const".to_string()),
                node.ident.to_string(),
                node.span(),
            );
            syn::visit::visit_item_const(self, node);
        }
        fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
            self.push(
                OwnerFamily::Item("static".to_string()),
                node.ident.to_string(),
                node.span(),
            );
            syn::visit::visit_item_static(self, node);
        }
        fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
            self.push(
                OwnerFamily::Item("mod".to_string()),
                node.ident.to_string(),
                node.span(),
            );
            syn::visit::visit_item_mod(self, node);
        }
        fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
            // Itself an owner; never descended into (methods pruned).
            self.push(
                OwnerFamily::Item("trait".to_string()),
                node.ident.to_string(),
                node.span(),
            );
        }
        fn visit_item_impl(&mut self, _node: &'ast syn::ItemImpl) {
            // No owners, fully pruned (v1 non-goal).
        }
    }
    let mut walk = Walk { owners: Vec::new() };
    walk.visit_file(file);
    walk.owners
}

/// Innermost owner containing 1-based `line`, or `None`.
fn owner_at(owners: &[Owner], line: usize) -> Option<&Owner> {
    owners
        .iter()
        .filter(|o| o.start <= line && line <= o.end)
        .min_by_key(|o| o.end - o.start)
}

/// Old-side `@@ -start[,len]` start for one hunk (`None` when the header
/// does not parse — callers treat the hunk as unclaimable).
fn hunk_old_start(hunk: &str) -> Option<usize> {
    let header = hunk.lines().next()?;
    let first = header.split_whitespace().nth(1)?;
    first
        .trim_start_matches('-')
        .split(',')
        .next()?
        .parse::<usize>()
        .ok()
}

/// One class hunk's body lines (directives + `-`/`+` lines, no `@@`
/// header — callers prepend the sweep dummy). `names` are the owner
/// names in site order: hetero rides `N`, uniform stays concrete.
///
/// Two header tiers for fn classes: the minimal tier (`- fn N(...) W {`,
/// one directive) matches only plain fns but is much shorter — it wins
/// T375 on small uniform classes; the full tier (`V Q`/`G` wildcards)
/// covers hetero slots at four-directive cost. Item classes need no
/// tiers (bare pairs ignore vis/generics). Callers probe the minimal
/// tier first and fall back to full.
fn emit_class_lines(class: &AttrClass, names: &[String], full: u8) -> String {
    let hetero = {
        let mut sorted = names.to_vec();
        sorted.sort();
        sorted.dedup();
        sorted.len() > 1
    };
    let name = if hetero {
        "N".to_string()
    } else {
        names[0].clone()
    };
    let mut out = String::new();
    match &class.family {
        OwnerFamily::Fn => {
            // Header tiers: 0 = bare (`- fn N(...) {`, no directives —
            // all-plain classes only, T375-friendly), 1 = where wildcard,
            // 2 = full slot wildcards for hetero slots. Callers probe in
            // order; omitted slots require absence, so a tier only fires
            // on classes it describes exactly.
            let (header, directives): (String, &str) = match full {
                2 => (
                    format!("V Q fn {name} G (...) W {{"),
                    "# visibility V\n# qualifier_list Q\n# generic_params G\n# where_clause W\n",
                ),
                1 => (format!("fn {name}(...) W {{"), "# where_clause W\n"),
                _ => (format!("fn {name}(...) {{"), ""),
            };
            out.push_str(directives);
            for m in &class.minus {
                out.push_str(&format!("- {m}\n"));
            }
            out.push_str(&format!("- {header}\n"));
            for p in &class.plus {
                out.push_str(&format!("+ {p}\n"));
            }
            out.push_str(&format!("+ {header}\n"));
            out.push_str("      ...\n  }\n");
        }
        OwnerFamily::Item(kind) => {
            for m in &class.minus {
                out.push_str(&format!("- {m}\n"));
            }
            out.push_str(&format!("- {kind} {name}\n"));
            for p in &class.plus {
                out.push_str(&format!("+ {p}\n"));
            }
            out.push_str(&format!("+ {kind} {name}\n"));
        }
    }
    out
}

/// Scratch-apply probe (T361 doctrine: `--check` alone never proves a
/// sweep): the emitted hunk must apply to the pre-image with exactly `n`
/// matches *and* reproduce byte-identically the result of applying the
/// claimed ordinary hunks. Count equality alone can coincide wrongly (an
/// untouched twin matching while a claimed site skips); the byte-compare
/// proves the emission rewrites exactly the claimed sites. Any failure —
/// unparseable emission, match-count mismatch, byte mismatch, or a loud
/// postcondition — vetoes the class.
fn probe_reproduces(
    path: &str,
    pre: &str,
    ordinary_hunks: &str,
    body_lines: &str,
    n: usize,
) -> bool {
    let ordinary_section = format!("{}\n{}", crate::git_header(path), ordinary_hunks);
    let Ok(parsed) = rsmpl_patch::parse_patch(&ordinary_section) else {
        return false;
    };
    let mut expected = HashMap::new();
    expected.insert(path.to_string(), pre.to_string());
    if rsmpl_engine::apply_patch_to_tree(&mut expected, &parsed).is_err() {
        return false;
    }
    let probe_body = format!("@@ -1,1 @@ rule=probe\n# sweep\n{body_lines}");
    let probe_section = format!("{}\n{}", crate::git_header(path), probe_body);
    let Ok(parsed2) = rsmpl_patch::parse_patch(&probe_section) else {
        return false;
    };
    let mut got = HashMap::new();
    got.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut got, &parsed2) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
        && got.get(path) == expected.get(path)
}

/// Tiered emission probe: bare tier first (shorter, tighter), then
/// where-wildcard, then full slot wildcards. Returns the winning tier's
/// body lines, or `None` when none reproduces the class exactly.
fn probe_tiered(
    path: &str,
    pre: &str,
    ordinary_hunks: &str,
    class: &AttrClass,
    names: &[String],
    n: usize,
) -> Option<String> {
    let tiers: &[u8] = if matches!(class.family, OwnerFamily::Fn) {
        &[0, 1, 2]
    } else {
        // Item classes have no tiers; the flag is inert for them.
        &[0]
    };
    for full in tiers {
        let lines = emit_class_lines(class, names, *full);
        if probe_reproduces(path, pre, ordinary_hunks, &lines, n) {
            return Some(lines);
        }
    }
    None
}

/// Whole-balanced pair-chain entry: homogeneous single-line attr replace
/// sections (bench 38's shape). Every pair must share one uniform delta
/// with per-pair owners in one family; hetero names ride `N`.
pub(crate) fn try_attr_replace_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if changed.len() != minuses.len() {
        return None;
    }
    let pre = file_old?;
    let Ok(file) = syn::parse_str::<syn::File>(pre) else {
        return None;
    };
    let owners = collect_owners(&file);
    // Single-line attr pairs only (multi-line spellings never match the
    // engine's per-attr entries — the probe would veto, so decline fast).
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(minuses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        if !is_attr_line(m) || !is_attr_line(p) {
            return None;
        }
        pairs.push((key_attr(m), key_attr(p)));
    }
    let (m0, p0) = pairs[0].clone();
    if m0 == p0 {
        return None;
    }
    if pairs.iter().any(|(m, p)| m != &m0 || p != &p0) {
        return None;
    }
    // One owner per pair, one family for the class.
    let mut family: Option<OwnerFamily> = None;
    let mut names: Vec<String> = Vec::with_capacity(pairs.len());
    for line in changed.iter().copied() {
        let owner = owner_at(&owners, line)?;
        if family.as_ref().is_some_and(|f| *f != owner.family) {
            return None;
        }
        family = Some(owner.family.clone());
        names.push(owner.name.clone());
    }
    let class = AttrClass {
        mode: AttrMode::Replace,
        minus: vec![m0],
        plus: vec![p0],
        family: family?,
    };
    // Candidates reserve no rules: the probe gate below drops violators
    // first, so dropped classes burn no `rN` names (peel precedent).
    // Ordinary-hunk reconstruction for the byte-compare probe: each pair
    // cites its changed old line (replace pairs are balanced, so the new
    // side starts at the same line).
    let mut ordinary = String::new();
    for (i, (m, p)) in pairs.iter().enumerate() {
        let l = changed[i];
        ordinary.push_str(&format!("@@ -{l},1 +{l},1 @@\n-{m}\n+{p}\n"));
    }
    let lines = probe_tiered(path, pre, &ordinary, &class, &names, pairs.len())?;
    let rule = unique_rule(used_rules, "attr_sweep");
    let mut out = crate::emission_header(path, &rule, &[]);
    out.push_str(&lines);
    Some((out, "attr_sweep"))
}

/// Chain-link section entry for the non-whole path: insert/delete/
/// replace groups over pure-attr hunks. Claimed hunks rewrite in place;
/// everything else stays byte-identical for verbatim + per-hunk to
/// retry. `None` when no class passes (caller proceeds unchanged).
pub(crate) fn try_attr_sweep_section(
    section: &str,
    path: &str,
    used_rules: &mut Vec<String>,
    file_old: Option<&str>,
    fired: &mut Vec<(String, &'static str)>,
) -> Option<String> {
    let (prelude, hunks) = split_section_hunks(section);
    if hunks.is_empty() {
        return None;
    }
    let pre = file_old?;
    let Ok(file) = syn::parse_str::<syn::File>(pre) else {
        return None;
    };
    let owners = collect_owners(&file);
    // Harvest pure-attr hunks with their owner names.
    struct Site {
        hunk: usize,
        class: AttrClass,
        names: Vec<String>,
    }
    let mut sites: Vec<Site> = Vec::new();
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
                // Context, blank, or marker lines: not -U0-pure, never
                // claimed (peel stranded-join precedent).
                pure = false;
            }
        }
        if !pure {
            continue;
        }
        let Some((mode, minus, plus)) = classify_hunk(&minuses, &pluses) else {
            continue;
        };
        // One owner per changed line; all lines of the hunk must belong
        // to the same owner (an attr block never spans two items).
        let lines: Vec<usize> = match mode {
            AttrMode::Insert => {
                let Some(start) = hunk_old_start(hunk) else {
                    continue;
                };
                vec![start + 1]
            }
            AttrMode::Delete | AttrMode::Replace => crate::totality::changed_old_lines(hunk),
        };
        if lines.is_empty() {
            continue;
        }
        let mut owner_names: Vec<String> = Vec::new();
        let mut owner_key: Option<(usize, usize, String)> = None;
        let mut family: Option<OwnerFamily> = None;
        let mut ok = true;
        for line in lines {
            let Some(owner) = owner_at(&owners, line) else {
                ok = false;
                break;
            };
            let key = (owner.start, owner.end, owner.name.clone());
            if owner_key.as_ref().is_some_and(|k| *k != key) {
                ok = false;
                break;
            }
            owner_key = Some(key);
            if family.as_ref().is_some_and(|f| *f != owner.family) {
                ok = false;
                break;
            }
            family = Some(owner.family.clone());
            owner_names.push(owner.name.clone());
        }
        if !ok {
            continue;
        }
        // Multi-line hunks repeat the owner per changed line; the class
        // emission needs one name per hunk.
        let Some(key) = owner_key else {
            continue;
        };
        sites.push(Site {
            hunk: i,
            class: AttrClass {
                mode,
                minus,
                plus,
                family: family?,
            },
            names: vec![key.2],
        });
    }
    // Group hunks by class, first-occurrence order; singletons decline.
    let mut classes: Vec<(AttrClass, Vec<usize>)> = Vec::new();
    for (idx, site) in sites.iter().enumerate() {
        match classes.iter_mut().find(|(k, _)| *k == site.class) {
            Some((_, members)) => members.push(idx),
            None => classes.push((site.class.clone(), vec![idx])),
        }
    }
    // Candidates reserve no rules: survivors reserve below, so dropped
    // classes burn no `rN` names (peel precedent).
    struct Candidate {
        first_hunk: usize,
        body: String,
        members: Vec<usize>,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (class, members) in &classes {
        if members.len() < 2 {
            continue;
        }
        let names: Vec<String> = members
            .iter()
            .map(|&idx| sites[idx].names[0].clone())
            .collect();
        // Ordinary-hunk reconstruction for the byte-compare probe: the
        // claimed hunks verbatim (headers cite original coordinates, so
        // they apply cleanly to the pre-image).
        let mut ordinary = String::new();
        for &idx in members {
            ordinary.push_str(&hunks[sites[idx].hunk]);
        }
        let Some(lines) = probe_tiered(path, pre, &ordinary, class, &names, members.len()) else {
            continue;
        };
        // Per-class T375: a class fires iff its body beats its covered
        // ordinary hunks. A losing class declines alone — it never vetoes
        // a winning class (partial compression over all-or-nothing).
        let hunk_len: usize = members
            .iter()
            .map(|&idx| hunks[sites[idx].hunk].len())
            .sum();
        if lines.len() >= hunk_len {
            continue;
        }
        candidates.push(Candidate {
            first_hunk: sites[members[0]].hunk,
            body: lines,
            members: members.clone(),
        });
    }
    if candidates.is_empty() {
        return None;
    }
    // Reserve rules + detector records for survivors only.
    let mut bodies: Vec<(usize, String)> = Vec::new();
    for cand in &candidates {
        let rule = unique_rule(used_rules, "attr_sweep");
        fired.push((rule.clone(), "attr_sweep"));
        bodies.push((
            cand.first_hunk,
            format!("{}{}", crate::emission_body(&rule, &[]), cand.body),
        ));
    }
    // Rewrite claimed hunks in place (first occurrence per class carries
    // the body); everything else stays byte-identical.
    let mut by_hunk: HashMap<usize, &String> = HashMap::new();
    for (first, body) in &bodies {
        by_hunk.insert(*first, body);
    }
    let claimed: std::collections::HashSet<usize> = candidates
        .iter()
        .flat_map(|c| c.members.iter().map(|&idx| sites[idx].hunk))
        .collect();
    let mut out = prelude;
    for (i, hunk) in hunks.iter().enumerate() {
        if let Some(body) = by_hunk.get(&i) {
            out.push_str(body);
        } else if claimed.contains(&i) {
            // Non-first member of a claimed class: folded into the
            // class body, dropped here.
            continue;
        } else {
            out.push_str(hunk);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRE: &str =
        "#[derive(Debug)]\nstruct A {\n    x: i32,\n}\nstruct B {\n    y: i32,\n}\nfn solo() {}\n";

    fn owners() -> Vec<Owner> {
        collect_owners(&syn::parse_str(PRE).unwrap())
    }

    #[test]
    fn owner_discovery_maps_lines_to_innermost_item() {
        let o = owners();
        // Line 1 (`#[derive]`) belongs to `A`; line 5 to `B`; line 8 to `solo`.
        assert_eq!(owner_at(&o, 1).unwrap().name, "A");
        assert_eq!(owner_at(&o, 5).unwrap().name, "B");
        assert_eq!(owner_at(&o, 8).unwrap().name, "solo");
        assert!(matches!(
            owner_at(&o, 1).unwrap().family,
            OwnerFamily::Item(_)
        ));
        assert!(matches!(owner_at(&o, 8).unwrap().family, OwnerFamily::Fn));
        // Past EOF: no owner.
        assert!(owner_at(&o, 99).is_none());
    }

    #[test]
    fn owner_discovery_prunes_impl_blocks() {
        let file: syn::File =
            syn::parse_str("struct S;\nimpl S {\n    #[inline]\n    fn m(&self) {}\n}\n").unwrap();
        let o = collect_owners(&file);
        // The method's attr line maps to nothing (pruned); the struct does.
        assert!(owner_at(&o, 3).is_none());
        assert_eq!(owner_at(&o, 1).unwrap().name, "S");
    }

    #[test]
    fn classify_hunk_modes() {
        let (mode, minus, plus) = classify_hunk(&[], &["#[inline]".to_string()]).expect("insert");
        assert_eq!(mode, AttrMode::Insert);
        assert!(minus.is_empty());
        assert_eq!(plus, vec!["#[inline]".to_string()]);
        let (mode, _, _) = classify_hunk(&["#[a]".to_string()], &[]).expect("delete");
        assert_eq!(mode, AttrMode::Delete);
        let (mode, _, _) = classify_hunk(
            &["#[derive(Debug)]".to_string()],
            &["#[derive(Debug, Clone)]".to_string()],
        )
        .expect("replace");
        assert_eq!(mode, AttrMode::Replace);
        // Mixed, blank, no-delta, and differing lengths decline.
        assert!(classify_hunk(&["fn f() {}".to_string()], &["#[a]".to_string()]).is_none());
        assert!(classify_hunk(&["".to_string()], &["#[a]".to_string()]).is_none());
        assert!(classify_hunk(&["#[a]".to_string()], &["#[a]".to_string()]).is_none());
        assert!(classify_hunk(
            &["#[a]".to_string()],
            &["#[b]".to_string(), "#[c]".to_string()]
        )
        .is_none());
    }

    #[test]
    fn section_entry_fires_on_insert_class() {
        // Four pure `+#[inline]` insertions over plain free fns compress
        // to one minimal-tier N-holed hunk and real-apply byte-identical
        // (F9 probe shape; two sites alone lose T375 to the fixed hunk
        // cost, so N=4 is the honest fire pin).
        let pre = "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -0,0 +1 @@\n+#[inline]\n@@ -1,0 +3 @@\n+#[inline]\n@@ -2,0 +5 @@\n+#[inline]\n@@ -3,0 +7 @@\n+#[inline]\n";
        let mut used = Vec::new();
        let mut fired = Vec::new();
        let out = try_attr_sweep_section(section, "f.rs", &mut used, Some(pre), &mut fired)
            .expect("insert class must fire");
        assert!(out.contains("- fn N(...) {\n"), "got:\n{out}");
        assert!(out.contains("+ #[inline]\n"), "got:\n{out}");
        assert_eq!(fired, vec![("r1".to_string(), "attr_sweep")]);
        // Round-trip: the emission applies to the pre-image byte-identical
        // to the ordinary section.
        let parsed = rsmpl_patch::parse_patch(&out).expect("emission must parse");
        let mut files = HashMap::new();
        files.insert("f.rs".to_string(), pre.to_string());
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "#[inline]\nfn one() {}\n#[inline]\nfn two() {}\n#[inline]\nfn three() {}\n#[inline]\nfn four() {}\n",
            "got:\n{}",
            files["f.rs"]
        );
    }

    #[test]
    fn emission_names_hetero_n_and_uniform_concrete() {
        let class = AttrClass {
            mode: AttrMode::Insert,
            minus: vec![],
            plus: vec!["#[inline]".to_string()],
            family: OwnerFamily::Fn,
        };
        let hetero = emit_class_lines(&class, &["a".to_string(), "b".to_string()], 0);
        assert!(hetero.contains("- fn N(...) {\n"), "got:\n{hetero}");
        assert!(hetero.contains("+ #[inline]\n"), "got:\n{hetero}");
        assert!(!hetero.contains("# where"), "got:\n{hetero}");
        let w = emit_class_lines(&class, &["a".to_string(), "b".to_string()], 1);
        assert!(w.contains("- fn N(...) W {\n"), "got:\n{w}");
        assert!(w.contains("# where_clause W\n"), "got:\n{w}");
        let full = emit_class_lines(&class, &["a".to_string(), "b".to_string()], 2);
        assert!(full.contains("- V Q fn N G (...) W {\n"), "got:\n{full}");
        let uni = emit_class_lines(&class, &["a".to_string(), "a".to_string()], 0);
        assert!(uni.contains("fn a(...)"), "got:\n{uni}");
        let item = AttrClass {
            mode: AttrMode::Replace,
            minus: vec!["#[derive(Debug)]".to_string()],
            plus: vec!["#[derive(Debug,Clone)]".to_string()],
            family: OwnerFamily::Item("struct".to_string()),
        };
        let out = emit_class_lines(&item, &["A".to_string(), "B".to_string()], 0);
        assert!(out.contains("- struct N\n"), "got:\n{out}");
        assert!(out.contains("+ #[derive(Debug,Clone)]\n"), "got:\n{out}");
    }

    #[test]
    fn pair_chain_entry_fires_on_homogeneous_replace() {
        // Whole-balanced replace sections route through the pair chain
        // (bench 38's shape): one N-holed hunk, probe-verified.
        let pre = "#[derive(Debug)]\nstruct A {}\n#[derive(Debug)]\nstruct B {}\n";
        let minuses = ["#[derive(Debug)]", "#[derive(Debug)]"];
        let pluses = ["#[derive(Debug, Clone)]", "#[derive(Debug, Clone)]"];
        let mut used = Vec::new();
        let (out, detector) =
            try_attr_replace_sweep(&minuses, &pluses, "f.rs", Some(pre), &[1, 3], &mut used)
                .expect("homogeneous replace must fire");
        assert_eq!(detector, "attr_sweep");
        assert!(out.contains("- struct N\n"), "got:\n{out}");
        assert!(out.contains("+ #[derive(Debug, Clone)]\n"), "got:\n{out}");
        // Round-trip byte-identical.
        let parsed = rsmpl_patch::parse_patch(&out).expect("emission must parse");
        let mut files = HashMap::new();
        files.insert("f.rs".to_string(), pre.to_string());
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "#[derive(Debug, Clone)]\nstruct A {}\n#[derive(Debug, Clone)]\nstruct B {}\n",
            "got:\n{}",
            files["f.rs"]
        );
    }

    #[test]
    fn pair_chain_entry_vetoes_matrix() {
        let pre = "#[derive(Debug)]\nstruct A {}\n#[derive(Debug)]\nstruct B {}\n";
        // Singleton: N=1 never compresses.
        let mut used = Vec::new();
        assert!(try_attr_replace_sweep(
            &["#[derive(Debug)]"],
            &["#[derive(Debug, Clone)]"],
            "f.rs",
            Some(pre),
            &[1],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
        // Hetero deltas decline (no template).
        let mut used = Vec::new();
        assert!(try_attr_replace_sweep(
            &["#[derive(Debug)]", "#[derive(Debug)]"],
            &["#[derive(Debug, Clone)]", "#[derive(Copy)]"],
            "f.rs",
            Some(pre),
            &[1, 3],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
        // Mixed families decline (fn + struct share no emission).
        let mixed_pre = "#[derive(Debug)]\nstruct A {}\n#[derive(Debug)]\nfn b() {}\n";
        let mut used = Vec::new();
        assert!(try_attr_replace_sweep(
            &["#[derive(Debug)]", "#[derive(Debug)]"],
            &["#[derive(Debug, Clone)]", "#[derive(Debug, Clone)]"],
            "f.rs",
            Some(mixed_pre),
            &[1, 3],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
        // No pre-image: honest decline (owners unprovable).
        let mut used = Vec::new();
        assert!(try_attr_replace_sweep(
            &["#[derive(Debug)]", "#[derive(Debug)]"],
            &["#[derive(Debug, Clone)]", "#[derive(Debug, Clone)]"],
            "f.rs",
            None,
            &[1, 3],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn untouched_twin_vetoes_but_migrated_site_does_not() {
        // An untouched same-state twin matches the probe and vetoes.
        let twin_pre = "#[derive(Debug)]\nstruct A {}\n#[derive(Debug)]\nstruct B {}\n#[derive(Debug)]\nstruct C {}\n";
        let mut used = Vec::new();
        assert!(
            try_attr_replace_sweep(
                &["#[derive(Debug)]", "#[derive(Debug)]"],
                &["#[derive(Debug, Clone)]", "#[derive(Debug, Clone)]"],
                "f.rs",
                Some(twin_pre),
                &[1, 3],
                &mut used,
            )
            .is_none(),
            "twin at line 5 must veto"
        );
        assert!(used.is_empty());
        // An already-migrated site elsewhere in the file is skipped by
        // subset semantics and never vetoes an add (it cannot match the
        // minus, and the byte-compare proves the sweep touches only the
        // claimed sites).
        let migrated_pre = "#[inline]\nfn done() {}\nfn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n";
        let section = "diff --git f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,0 +4 @@\n+#[inline]\n@@ -3,0 +6 @@\n+#[inline]\n@@ -4,0 +8 @@\n+#[inline]\n@@ -5,0 +10 @@\n+#[inline]\n";
        let mut used = Vec::new();
        let mut fired = Vec::new();
        let out =
            try_attr_sweep_section(section, "f.rs", &mut used, Some(migrated_pre), &mut fired)
                .expect("migrated site must not veto");
        assert!(out.contains("fn N("), "got:\n{out}");
    }
}
