//! Enclosing-macro interior sweep (WI-7a). One level above the
//! whole-line interior sweep in `macros.rs`: changed lines that are not
//! themselves macro invocations but sit *inside* one shared macro
//! invocation compress to a single concrete token-tree hunk over the
//! enclosing macro text.
//!
//! Motivating shape (bench 17): three identical `header = OLD` lines,
//! each an interior line of a multi-line `format!(...)`, become one
//! `- format!(...)` / `+ format!(...)` hunk instead of three ordinary
//! hunks. The engine matches enclosing templates token-wise
//! (`fragments_equal` is whitespace-insensitive), so the raw
//! multi-line span text round-trips byte-identical.
//!
//! v1 scope (deliberately narrow): only *identical* enclosing pairs
//! (same path, same delimiter, byte-identical minus/plus full texts)
//! group; varying stables/deltas across sites stay ordinary (hetero
//! enclosing is future work — the whole-line hetero pass in
//! `macro_hetero.rs` owns that shape one level down). No stranded
//! joins, no `# sweep` authored form beyond the emitted dummy range
//! (emission is always file scope: the candidate enclosing span
//! exceeds any interior-line range, so a ranged hunk could never
//! contain it). Same soundness contract as every other detector:
//! per-group engine-count totality (`template_enclosing_holds`),
//! hunk atomicity (a group fires only if every hunk holding a member
//! pair is fully covered), and the section-global T375 byte guard.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use super::peel::split_section_hunks;
use super::unique_rule;

/// One macro invocation in the pre-image: exact byte span + spelling.
/// `macro_rules!` definitions are never collected (WI-7b owns them).
struct Enclosing {
    path: String,
    open: char,
    start: usize,
    end: usize,
}

struct SpanCollector {
    index: rsmpl_engine::LineIndex,
    out: Vec<Enclosing>,
}

impl SpanCollector {
    fn push_macro(&mut self, mac: &syn::Macro) {
        if mac.path.leading_colon.is_some() {
            return;
        }
        let mut segs = Vec::new();
        for seg in mac.path.segments.iter() {
            if !matches!(seg.arguments, syn::PathArguments::None) {
                return;
            }
            let name = seg.ident.to_string();
            if !super::ident_len(&name).is_some_and(|n| n == name.len()) {
                return;
            }
            segs.push(name);
        }
        if segs.is_empty() {
            return;
        }
        if segs.len() == 1 && segs[0] == "macro_rules" {
            return;
        }
        let open = match mac.delimiter {
            syn::MacroDelimiter::Paren(_) => '(',
            syn::MacroDelimiter::Bracket(_) => '[',
            syn::MacroDelimiter::Brace(_) => '{',
        };
        let span = mac.span();
        let (Some(start), Some(end)) = (
            self.index
                .offset(span.start().line as u32, span.start().column),
            self.index.offset(span.end().line as u32, span.end().column),
        ) else {
            return;
        };
        if start >= end {
            return;
        }
        self.out.push(Enclosing {
            path: segs.join("::"),
            open,
            start,
            end,
        });
    }
}

impl<'ast> Visit<'ast> for SpanCollector {
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.push_macro(node);
        visit::visit_macro(self, node);
    }
}

/// Byte range of the 1-based old-side line in `pre` (content only, no
/// trailing newline / carriage return). `None` past the file end.
fn line_byte_range(
    index: &rsmpl_engine::LineIndex,
    pre: &str,
    old_line: usize,
) -> Option<(usize, usize)> {
    let start = index.line_start(old_line as u32)?;
    let mut end = index.line_start(old_line as u32 + 1).unwrap_or(pre.len());
    if end > start && pre.as_bytes().get(end - 1) == Some(&b'\n') {
        end -= 1;
    }
    if end > start && pre.as_bytes().get(end - 1) == Some(&b'\r') {
        end -= 1;
    }
    Some((start, end.min(pre.len())))
}

/// Smallest enclosing macro whose byte span fully contains the changed
/// line and whose line-spliced plus text differs from its minus text.
/// Byte containment (not line containment) skips same-line nested
/// macros whose span cannot cover the whole changed line.
fn enclosing_for(
    macros: &[Enclosing],
    pre: &str,
    index: &rsmpl_engine::LineIndex,
    old_line: usize,
    plus_line: &str,
) -> Option<(usize, String, String)> {
    let (lb, le) = line_byte_range(index, pre, old_line)?;
    let mut best: Option<(usize, usize, String, String)> = None;
    for (i, m) in macros.iter().enumerate() {
        if !(m.start <= lb && le <= m.end) {
            continue;
        }
        let minus_full = pre.get(m.start..m.end)?.to_string();
        let mut plus_full = String::with_capacity(minus_full.len() + plus_line.len());
        plus_full.push_str(pre.get(m.start..lb)?);
        plus_full.push_str(plus_line);
        plus_full.push_str(pre.get(le..m.end)?);
        if minus_full == plus_full {
            continue;
        }
        let len = m.end - m.start;
        if best
            .as_ref()
            .is_some_and(|(_, best_len, _, _)| *best_len <= len)
        {
            continue;
        }
        best = Some((i, len, minus_full, plus_full));
    }
    best.map(|(i, _, minus_full, plus_full)| (i, minus_full, plus_full))
}

/// Grouping key for identical enclosing pairs: the macro path, the
/// open delimiter, and the byte-identical minus/plus span texts.
type EnclosingKey = (String, char, String, String);

/// Enclosing-macro totality: the emitted multi-line enclosing macro
/// pair (raw pre-image span text, plus the line-spliced post text)
/// must match exactly `n` sites in the pre-image file. Counted by the
/// engine itself with the real pair spelling: a `# sweep` probe
/// carrying the K minus lines plus the K plus lines (bare `-`/`+`
/// prefixes — the span text carries its own indentation) is applied
/// to a scratch tree and its match count must equal `n`. An untouched
/// twin enclosing macro anywhere in the file counts `n + 1` and
/// vetoes. `None` pre-image keeps the legacy allow; probe parse/apply
/// failure vetoes. Always file scope (`# sweep`): the candidate
/// (enclosing span) exceeds any interior-line range, so a ranged hunk
/// could never contain it.
fn template_enclosing_holds(
    path: &str,
    file_old: Option<&str>,
    minus_full: &str,
    plus_full: &str,
    n: usize,
) -> bool {
    if n == 0 || minus_full.trim().is_empty() || plus_full.trim().is_empty() {
        return false;
    }
    let Some(pre) = file_old else {
        return true;
    };
    let mut probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n",
        crate::git_header(path)
    );
    for m in minus_full.lines() {
        probe.push_str(&format!("-{m}\n"));
    }
    for p in plus_full.lines() {
        probe.push_str(&format!("+{p}\n"));
    }
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
}

struct EnclosingGroup {
    minus: String,
    plus: String,
    members: Vec<usize>,
}

/// Same-name enclosing-macro sweep over leftover balanced pairs: every
/// pair whose changed line sits inside a shared enclosing macro
/// invocation groups by (path, delimiter, minus-full, plus-full); a
/// group fires only at N ≥ 2 with engine-count totality and full-hunk
/// coverage. Runs after `try_macro_interior_section` on its output:
/// already-semantic (`rule=`) hunks are never harvested, so claimed
/// pairs cannot double-fire.
pub(crate) fn try_macro_enclosing_section(
    section: &str,
    path: &str,
    used_rules: &mut Vec<String>,
    file_old: Option<&str>,
    fired: &mut Vec<(String, &'static str)>,
) -> Option<String> {
    let pre = file_old?;
    let (prelude, hunks) = split_section_hunks(section);
    if hunks.is_empty() {
        return None;
    }
    let parsed: syn::File = syn::parse_str(pre).ok()?;
    let mut collector = SpanCollector {
        index: rsmpl_engine::LineIndex::new(pre),
        out: Vec::new(),
    };
    collector.visit_file(&parsed);
    let macros = collector.out;
    if macros.is_empty() {
        return None;
    }
    let hunk_changed: Vec<Vec<usize>> = hunks
        .iter()
        .map(|h| crate::totality::changed_old_lines(h))
        .collect();
    // Harvest balanced whole-line pairs from ORDINARY hunks only.
    let mut pairs: Vec<(usize, String, String)> = Vec::new();
    let mut pair_old: Vec<usize> = Vec::new();
    let mut hunk_balanced = vec![false; hunks.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        if hunk.lines().next().is_some_and(|h| h.contains("rule=")) {
            continue;
        }
        let mut minuses = Vec::new();
        let mut pluses = Vec::new();
        let mut pure = true;
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            } else {
                pure = false;
            }
        }
        if !pure || minuses.len() != pluses.len() || minuses.is_empty() {
            continue;
        }
        hunk_balanced[i] = true;
        for (k, (m, p)) in minuses.into_iter().zip(pluses).enumerate() {
            if m.trim() == p.trim() {
                continue;
            }
            let old_line = hunk_changed[i].get(k).copied().unwrap_or(0);
            pairs.push((i, m, p));
            pair_old.push(old_line);
        }
    }
    if pairs.is_empty() {
        return None;
    }
    let index = rsmpl_engine::LineIndex::new(pre);
    // Group identical enclosing pairs, first-occurrence order.
    let mut classes: Vec<(EnclosingKey, Vec<usize>)> = Vec::new();
    for (idx, (.., m, p)) in pairs.iter().enumerate() {
        let Some(old_line) = pair_old.get(idx).copied().filter(|l| *l >= 1) else {
            continue;
        };
        // Whole-line macro pairs belong to the interior sweep one level
        // down (it declined this section, but the shape is still its
        // domain — hetero twins and divergent suffixes veto there by
        // totality, and re-grouping them here would only re-veto).
        if super::macro_parts::parse_macro_interior_pair(m, p).is_some() {
            continue;
        }
        let Some((mi, minus_full, plus_full)) = enclosing_for(&macros, pre, &index, old_line, p)
        else {
            continue;
        };
        let path_open = macros
            .get(mi)
            .map(|e| (e.path.clone(), e.open))
            .unwrap_or_default();
        let key = (path_open.0, path_open.1, minus_full, plus_full);
        match classes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(idx),
            None => classes.push((key, vec![idx])),
        }
    }
    let checkpoint = used_rules.len();
    let mut passing: Vec<EnclosingGroup> = Vec::new();
    for ((_, _, minus, plus), members) in &classes {
        if members.len() < 2 {
            continue;
        }
        if !template_enclosing_holds(path, file_old, minus, plus, members.len()) {
            continue;
        }
        passing.push(EnclosingGroup {
            minus: minus.clone(),
            plus: plus.clone(),
            members: members.clone(),
        });
    }
    // Hunk atomicity (interior-sweep fixed point): a group fires only if
    // every balanced hunk holding a member pair is fully covered.
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
        passing.remove(gi);
    }
    if passing.is_empty() {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut fired_groups: Vec<(EnclosingGroup, String)> = Vec::new();
    for g in passing {
        let rule = unique_rule(used_rules, "macro_enclosing");
        fired.push((rule.clone(), "macro_enclosing"));
        fired_groups.push((g, rule));
    }
    let passing = fired_groups;
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
    // T375 over the covered set (no stranded leftovers in v1).
    // Bare `-`/`+` prefixes (no added space): the span text carries
    // its own indentation and the parser strips exactly one prefix
    // char, so `- ` would phantom-indent every indented line by one.
    let bodies: Vec<String> = passing
        .iter()
        .map(|(g, rule)| {
            let mut h = format!("@@ -1,1 @@ rule={rule}\n# sweep\n");
            for line in g.minus.lines() {
                h.push_str(&format!("-{line}\n"));
            }
            for line in g.plus.lines() {
                h.push_str(&format!("+{line}\n"));
            }
            h
        })
        .collect();
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
    let mut out = prelude;
    let mut emitted = vec![false; passing.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        if covered_hunk[i] {
            for (gi, g) in passing.iter().enumerate() {
                if !emitted[gi]
                    && pairs
                        .iter()
                        .enumerate()
                        .any(|(jj, (j, _, _))| *j == i && g.0.members.contains(&jj))
                {
                    out.push_str(&bodies[gi]);
                    emitted[gi] = true;
                }
            }
            continue;
        }
        out.push_str(hunk);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic, infer_semantic_with_pre_images};
    use super::try_macro_enclosing_section;
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

    // Bench-17-like long interior lines: the ordinary hunks are big
    // enough that one enclosing template wins T375 (two tiny sites
    // would — correctly — stay ordinary; see the T375 pin below).
    fn pre_three_site() -> HashMap<String, String> {
        pre_map(&[(
            "f.rs",
            concat!(
                "fn m() {\n",
                "    let a = wrap!(\n",
                "        \"header-text-value\",\n",
                "        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n",
                "    );\n",
                "    let b = wrap!(\n",
                "        \"header-text-value\",\n",
                "        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n",
                "    );\n",
                "    let c = wrap!(\n",
                "        \"header-text-value\",\n",
                "        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n",
                "    );\n",
                "}\n",
            ),
        )])
    }

    fn section_three_site() -> String {
        section(
            "f.rs",
            concat!(
                "@@ -4 +4 @@\n",
                "-        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n",
                "+        key = other::module::render(path)\n",
                "@@ -8 +8 @@\n",
                "-        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n",
                "+        key = other::module::render(path)\n",
                "@@ -12 +12 @@\n",
                "-        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n",
                "+        key = other::module::render(path)\n",
            ),
        )
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
    fn enclosing_fire_three_identical_sites() {
        let raw = section_three_site();
        let pre = pre_three_site();
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("enclosing must fire");
        assert!(out.contains("# sweep\n"), "got: {out}");
        assert!(out.contains("-wrap!("), "got: {out}");
        assert!(
            out.contains("key = other::module::render(path)"),
            "got: {out}"
        );
        validated_live(&out, &pre);
        // The applied tree equals the expected post-image (the nested
        // `format!` inside the removed delta is fine — the concrete
        // template compares token-wise, never descending).
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        let post = files["f.rs"].replace(
            "key = some::module::transform(&format!(\"payload-{x}-suffix\"))",
            "key = other::module::render(path)",
        );
        let expected = pre["f.rs"].replace(
            "key = some::module::transform(&format!(\"payload-{x}-suffix\"))",
            "key = other::module::render(path)",
        );
        assert_eq!(post, expected, "got: {post}");
    }

    #[test]
    fn enclosing_two_tiny_sites_stay_ordinary() {
        // T375 pin: two short interior lines cost less as ordinary hunks
        // than as one enclosing template — the guard (not the matcher)
        // keeps them ordinary.
        let raw = section(
            "f.rs",
            "@@ -4 +4 @@\n-        key = old_val\n+        key = new_val\n@@ -8 +8 @@\n-        key = old_val\n+        key = new_val\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = wrap!(\n        \"x\",\n        key = old_val\n    );\n    let b = wrap!(\n        \"x\",\n        key = old_val\n    );\n}\n",
        )]);
        let mut used = Vec::new();
        let mut fired = Vec::new();
        assert!(
            try_macro_enclosing_section(&raw, "f.rs", &mut used, Some(&pre["f.rs"]), &mut fired)
                .is_none(),
            "T375 must decline the tiny template"
        );
    }

    #[test]
    fn enclosing_single_site_stays_ordinary() {
        // One enclosing pair (N=1) never groups.
        let raw = section(
            "f.rs",
            "@@ -4 +4 @@\n-        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n+        key = other::module::render(path)\n",
        );
        let pre = pre_three_site();
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none()
                || !infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                    .unwrap()
                    .contains("-wrap!("),
            "single site must stay ordinary"
        );
    }

    #[test]
    fn enclosing_hetero_stays_ordinary() {
        // Same enclosing path but different full texts across sites
        // (varying stable) — not this slice (hetero enclosing is future
        // work); the enclosing detector stays out.
        let raw = section(
            "f.rs",
            "@@ -4 +4 @@\n-        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n+        key = other::module::render(path)\n@@ -8 +8 @@\n-        key = some::module::transform(&format!(\"payload-{y}-suffix\"))\n+        key = other::module::render(path)\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            concat!(
                "fn m() {\n",
                "    let a = wrap!(\n",
                "        \"header-text-value\",\n",
                "        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n",
                "    );\n",
                "    let b = wrap!(\n",
                "        \"header-text-value\",\n",
                "        key = some::module::transform(&format!(\"payload-{y}-suffix\"))\n",
                "    );\n",
                "}\n",
            ),
        )]);
        let mut used = Vec::new();
        let mut fired = Vec::new();
        assert!(
            try_macro_enclosing_section(&raw, "f.rs", &mut used, Some(&pre["f.rs"]), &mut fired)
                .is_none(),
            "hetero enclosing must stay ordinary"
        );
    }

    #[test]
    fn enclosing_untouched_twin_vetoes() {
        // A fourth identical enclosing macro untouched by the diff counts
        // 4 ≠ 3 in the engine probe, so the group stays ordinary.
        let raw = section_three_site();
        let mut twin = pre_three_site()["f.rs"].clone();
        twin = twin.replace(
            "}\n",
            "    let d = wrap!(\n        \"header-text-value\",\n        key = some::module::transform(&format!(\"payload-{x}-suffix\"))\n    );\n}\n",
        );
        let pre = pre_map(&[("f.rs", &twin)]);
        let mut used = Vec::new();
        let mut fired = Vec::new();
        assert!(
            try_macro_enclosing_section(&raw, "f.rs", &mut used, Some(&pre["f.rs"]), &mut fired)
                .is_none(),
            "untouched twin must veto the enclosing group"
        );
    }

    #[test]
    fn enclosing_skips_semantic_hunks() {
        // Already-semantic (`rule=`) hunks are never re-harvested: one
        // semantic hunk plus one lone ordinary pair fires nothing.
        let raw = section(
            "f.rs",
            "@@ -1,1 @@ rule=r1\n# sweep\n- wrap!(\"x\", key = old_val)\n+ wrap!(\"x\", key = new_val)\n@@ -8 +8 @@\n-        key = old_val\n+        key = new_val\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = wrap!(\n        \"x\",\n        key = old_val\n    );\n}\n",
        )]);
        let mut used = vec!["r1".to_string()];
        let mut fired = Vec::new();
        let out =
            try_macro_enclosing_section(&raw, "f.rs", &mut used, Some(&pre["f.rs"]), &mut fired);
        assert!(
            out.is_none(),
            "lone ordinary pair must not fire, got: {out:?}"
        );
    }

    #[test]
    fn enclosing_unparseable_pre_image_declines() {
        let raw = section_three_site();
        let pre = pre_map(&[("f.rs", "fn m( broken (((\n")]);
        let mut used = Vec::new();
        let mut fired = Vec::new();
        assert!(try_macro_enclosing_section(
            &raw,
            "f.rs",
            &mut used,
            Some(&pre["f.rs"]),
            &mut fired
        )
        .is_none());
    }

    #[test]
    fn enclosing_whole_line_macro_pairs_belong_to_interior() {
        // Changed lines that are themselves whole-line macro invocations
        // are the interior sweep's domain — the enclosing pass skips
        // them even when the interior sweep declined.
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    log!(alpha, 1);\n+    log!(alpha, 2);\n@@ -3 +3 @@\n-    log!(beta, 1);\n+    log!(beta, 2);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    log!(alpha, 1);\n    log!(beta, 1);\n}\n",
        )]);
        let mut used = Vec::new();
        let mut fired = Vec::new();
        // The hetero interior pass (not this detector) owns these; the
        // enclosing detector alone must not claim them.
        let out =
            try_macro_enclosing_section(&raw, "f.rs", &mut used, Some(&pre["f.rs"]), &mut fired);
        assert!(
            out.is_none(),
            "whole-line pairs must not fire enclosing, got: {out:?}"
        );
        // Sanity: the full chain still compresses them (hetero owns it).
        let full = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre);
        assert!(full.is_some(), "hetero interior should fire");
    }

    #[test]
    fn enclosing_no_pre_image_declines() {
        let raw = section_three_site();
        let mut used = Vec::new();
        let mut fired = Vec::new();
        assert!(try_macro_enclosing_section(&raw, "f.rs", &mut used, None, &mut fired).is_none());
        // Without pre-images the section driver also declines (legacy
        // allow lives in whole-line probes, not here).
        assert!(
            infer_semantic(std::slice::from_ref(&raw)).is_none()
                || !infer_semantic(std::slice::from_ref(&raw))
                    .unwrap()
                    .contains("-wrap!(")
        );
    }
}
