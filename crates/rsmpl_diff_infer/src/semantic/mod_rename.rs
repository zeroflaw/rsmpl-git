//! WI-15c `mod` + file-rename intent raising (SPEC D.4 `mod_rename`).
//!
//! One `mod old;` → `mod new;` declaration pair joined with N-1
//! same-delta `old::` path-use lines across files compresses to the
//! single definition hunk *with* `# follow=uses` (the engine follow
//! already rewrites `crate::old::` and bare `old::` `use` lines and
//! expression paths workspace-wide, probe-verified). An accompanying
//! headers-only `rename from/to` section (flat `old.rs`→`new.rs` or
//! `old/mod.rs`→`new/mod.rs` layouts) is preserved verbatim as the
//! native file-move prelude — it carries no pairs and never counts
//! toward T375.
//!
//! Totality is a cross-file scratch-apply probe (member-rename posture,
//! WI-15b): the authored emission applied to an in-memory clone of the
//! pre-image must reproduce exactly the native plus lines at the claimed
//! changed sites with every other line byte-identical (a missed site
//! fails the plus-leg, an overwritten twin fails the unchanged-leg).
//! File/section-pure partial: proven sections collapse to the emission
//! at the def section's slot, unproven sections pass through
//! byte-identical (they still reach the per-section `use_prefix`/sweep
//! chain downstream — the use-prefix follow-through leg, each hunk
//! independently totality-checked). Mixed sections (proven + unproven
//! pairs in one section) veto the whole intent (overlap safety).
//! `None` pre-image declines (the `mod` container is unprovable without
//! it); inline `mod old {` bodies decline (there is no file to join);
//! single-section two-pair shapes defer to the per-section `item_rename`
//! path (canonical stability).

use std::collections::{HashMap, HashSet};

use super::{split_item, try_item_rename, unique_rule};

/// One parsed eligible pair section.
struct SecPairs {
    idx: usize,
    path: String,
    minuses: Vec<String>,
    pluses: Vec<String>,
    changed: Vec<usize>,
}

/// One native pair at its old-side site.
struct Pair {
    sec: usize,
    path: String,
    minus: String,
    plus: String,
    site: usize,
}

/// Whole-patch `mod`-intent pre-pass: returns the rebuilt section list
/// (claimed pair sections replaced by the single semantic section at
/// the definition section's slot, rename sections preserved verbatim)
/// plus the definition path and rule for provenance, or `None` when the
/// patch holds no provable intent. Reserved rule names are released on
/// decline.
pub(crate) fn try_mod_rename_intent_sweep(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    // Partition: headers-only rename sections aside (preserved verbatim,
    // never pair sources), eligible `.rs` pair sections parsed.
    let mut rename_secs: Vec<usize> = Vec::new();
    let mut parsed: Vec<SecPairs> = Vec::new();
    let mut eligible: Vec<usize> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        let header = sec.lines().next().unwrap_or("");
        if !header.starts_with("diff --git ") || sec.contains('\r') {
            continue;
        }
        if super::is_rename_header(header) {
            // Content-carrying renames own the existing rename-content
            // path (per-hunk compression under the preserved prelude);
            // only headers-only file moves join the `mod` intent.
            if sec.lines().any(|l| l.starts_with("@@")) {
                return None;
            }
            if !sec.lines().any(|l| l.starts_with("rename from ")) {
                return None;
            }
            rename_secs.push(i);
            continue;
        }
        // WI-4d: pre-pass emissions never re-enter later pre-passes.
        if super::is_semantic_section(sec) {
            continue;
        }
        let rest = header.strip_prefix("diff --git ")?;
        let path = crate::b_side_path(rest)?;
        if !path.ends_with(".rs") {
            continue;
        }
        eligible.push(i);
        let (minuses, pluses, balanced) = section_pairs(sec);
        if !balanced || minuses.is_empty() {
            return None;
        }
        let changed = crate::totality::changed_old_lines(sec);
        if changed.len() != minuses.len() {
            return None;
        }
        parsed.push(SecPairs {
            idx: i,
            path,
            minuses,
            pluses,
            changed,
        });
    }
    if rename_secs.len() > 1 {
        return None;
    }
    if parsed.is_empty() {
        return None;
    }
    let mut pairs: Vec<Pair> = Vec::new();
    for sp in &parsed {
        for ((m, p), s) in sp
            .minuses
            .iter()
            .zip(sp.pluses.iter())
            .zip(sp.changed.iter())
        {
            pairs.push(Pair {
                sec: sp.idx,
                path: sp.path.clone(),
                minus: m.clone(),
                plus: p.clone(),
                site: *s,
            });
        }
    }
    if pairs.len() < 2 {
        return None;
    }
    // Exactly one `mod old;` declaration delta (diet-bare `split_item`
    // delta with a `;` suffix on both sides); every other pair is a use
    // line sharing the delta.
    let mut def_idx: Option<usize> = None;
    let mut old_ident = String::new();
    let mut new_ident = String::new();
    for (i, pair) in pairs.iter().enumerate() {
        if let Some((kind, old, new)) = split_mod_delta(&pair.minus, &pair.plus) {
            if kind != "mod" {
                continue;
            }
            if def_idx.is_some() {
                used_rules.truncate(checkpoint);
                return None;
            }
            def_idx = Some(i);
            old_ident = old;
            new_ident = new;
        }
    }
    let def_idx = def_idx?;
    if old_ident == new_ident || old_ident.is_empty() || new_ident.is_empty() {
        return None;
    }
    // More than one `mod`-shaped delta (e.g. two `mod` lines) declines:
    // `split_mod_delta` above claims the first; a second one anywhere
    // (even same ident) means the intent is ambiguous.
    let mut mod_deltas = 0usize;
    for pair in &pairs {
        if split_mod_delta(&pair.minus, &pair.plus).is_some() {
            mod_deltas += 1;
        }
    }
    if mod_deltas != 1 {
        used_rules.truncate(checkpoint);
        return None;
    }
    for (i, pair) in pairs.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        if pair.minus.trim() == pair.plus.trim() {
            return None;
        }
        if !contains_ident(&pair.minus, &old_ident) || !contains_ident(&pair.plus, &new_ident) {
            return None;
        }
    }
    for pair in &pairs {
        if !pre.contains_key(&pair.path) {
            return None;
        }
    }
    // Twin-`mod` veto (unique-definition rule, SPEC 14.8): exactly one
    // `mod old;` declaration across the involved pre-images.
    if count_mod_defs(pre, &pairs, &old_ident) != 1 {
        return None;
    }
    // Dedicated-position carve (same as `item_intent`): call callees and
    // macro heads naming `old` belong to their detectors.
    for (i, pair) in pairs.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        if is_dedicated_use_site(&pair.minus, &old_ident) {
            return None;
        }
    }
    // Rename gate: a present file move must name the `mod` delta
    // (flat or `mod.rs` layouts, dirs otherwise equal).
    if let Some(&ri) = rename_secs.first() {
        let (from, to) = rename_paths(&sections[ri])?;
        if !rename_matches(&old_ident, &new_ident, &from, &to) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    let def = &pairs[def_idx];
    // The def pair must be emittable as a lone item rename (diet
    // `- mod old` matches the `mod old;` line, probe-verified).
    let mut probe_used = vec![format!("probe{}", used_rules.len())];
    let def_pre = pre.get(&def.path).map(String::as_str);
    try_item_rename(
        &def.minus,
        &def.plus,
        &def.path,
        def_pre,
        std::slice::from_ref(&def.site),
        &mut probe_used,
    )?;
    // Single-section two-pair shapes stay with the per-section path
    // (that path owns the one-section win; canonical bytes stay stable).
    if parsed.len() == 1 {
        let sp = &parsed[0];
        if sp.minuses.len() == pairs.len() {
            return None;
        }
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_ident}_to_{new_ident}"));
    let emission = emit(&def.path, &rule, &old_ident, &new_ident);
    // Cross-file scratch probe: classify each pair as proven (post trim
    // == plus trim). Def must be proven; at least one use proven.
    let parsed_patch = rsmpl_patch::parse_patch(&emission).ok()?;
    let mut files = pre.clone();
    for section in &parsed_patch.files {
        files.entry(section.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed_patch).is_err() {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut proven = vec![false; pairs.len()];
    for (i, pair) in pairs.iter().enumerate() {
        let Some(post) = files.get(&pair.path) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        let post_lines: Vec<&str> = post.lines().collect();
        let Some(got) = post_lines.get(pair.site.saturating_sub(1)) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        proven[i] = got.trim() == pair.plus.trim();
    }
    if !proven[def_idx] {
        used_rules.truncate(checkpoint);
        return None;
    }
    if proven.iter().filter(|&&p| p).count() < 2 {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Section purity: every section holding a proven pair must be fully
    // proven (else the emission rewrites a line the ordinary remainder
    // also rewrites). Mixed sections veto the whole intent.
    let mut sec_all: HashMap<usize, bool> = HashMap::new();
    let mut sec_has: HashMap<usize, bool> = HashMap::new();
    for (i, pair) in pairs.iter().enumerate() {
        let e = sec_all.entry(pair.sec).or_insert(true);
        if !proven[i] {
            *e = false;
        }
        if proven[i] {
            sec_has.insert(pair.sec, true);
        }
    }
    for (sec, has) in &sec_has {
        if *has && !sec_all.get(sec).copied().unwrap_or(false) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    let claimed_secs: HashSet<usize> = sec_has
        .iter()
        .filter(|(_, h)| **h)
        .map(|(s, _)| *s)
        .collect();
    // Unclaimed files byte-identical under emission; claimed files clean
    // outside claimed sites.
    let claimed_files: HashSet<&str> = pairs
        .iter()
        .enumerate()
        .filter(|(i, _)| proven[*i])
        .map(|(_, p)| p.path.as_str())
        .collect();
    for (path, pre_text) in pre.iter() {
        let Some(post_text) = files.get(path) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        if !claimed_files.contains(path.as_str()) {
            if post_text != pre_text {
                used_rules.truncate(checkpoint);
                return None;
            }
            continue;
        }
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let post_lines: Vec<&str> = post_text.lines().collect();
        if post_lines.len() != pre_lines.len() {
            used_rules.truncate(checkpoint);
            return None;
        }
        let mut claimed_sites: HashSet<usize> = HashSet::new();
        for (i, pair) in pairs.iter().enumerate() {
            if proven[i] && pair.path == *path {
                claimed_sites.insert(pair.site);
            }
        }
        for (idx, (a, b)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
            if claimed_sites.contains(&(idx + 1)) {
                continue;
            }
            if a != b {
                used_rules.truncate(checkpoint);
                return None;
            }
        }
    }
    // T375 over the claimed pair sections only (the rename prelude is
    // preserved, not replaced, so its bytes never count).
    let claimed_bytes: usize = parsed
        .iter()
        .filter(|sp| claimed_secs.contains(&sp.idx))
        .map(|sp| sections[sp.idx].len())
        .sum();
    if emission.len() >= claimed_bytes {
        used_rules.truncate(checkpoint);
        return None;
    }
    let rename_set: HashSet<usize> = rename_secs.iter().copied().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        if rename_set.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        if !eligible.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        if !claimed_secs.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        if i == def.sec {
            out.push(emission.clone());
        }
    }
    Some((out, def.path.clone(), rule))
}

/// One section's minus/plus lines plus the balanced gate (same walk as
/// `chain.rs` / `item_intent.rs`: every hunk balanced, global counts
/// equal, non-empty).
fn section_pairs(section: &str) -> (Vec<String>, Vec<String>, bool) {
    let mut minuses: Vec<String> = Vec::new();
    let mut pluses: Vec<String> = Vec::new();
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
            if line.starts_with("old mode ") || line.starts_with("new mode ") {
                balanced = false;
            }
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            minuses.push(line[1..].to_string());
            hunk_minus += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            pluses.push(line[1..].to_string());
            hunk_plus += 1;
        }
    }
    if in_hunk && hunk_minus != hunk_plus {
        balanced = false;
    }
    let ok = balanced && saw_hunk && minuses.len() == pluses.len() && !minuses.is_empty();
    (minuses, pluses, ok)
}

/// A `mod old;` declaration delta, or `None` for use lines and inline
/// `mod old {` bodies (no file to join — the per-section path owns
/// those). Returns the (`kind`, `old`, `new`) triple.
fn split_mod_delta(minus: &str, plus: &str) -> Option<(String, String, String)> {
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(minus.trim_end())?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(plus.trim_end())?;
    if old_kind != "mod" || new_kind != "mod" {
        return None;
    }
    if old_prefix != new_prefix {
        return None;
    }
    // Declaration form only: `mod old;` (a trailing `;`). Inline
    // bodies (`mod old {`) have no file move to join.
    if old_suffix.trim() != ";" || new_suffix.trim() != ";" {
        return None;
    }
    if old_ident == new_ident {
        return None;
    }
    Some((old_kind, old_ident, new_ident))
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

/// True when the use line's changed subterm sits in a dedicated
/// detector's position (call callee or macro head naming `old`). Same
/// carve as `item_intent` — those classes keep their pinned shapes.
fn is_dedicated_use_site(minus: &str, old: &str) -> bool {
    let mut text = minus.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return false;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if is_dedicated_expr(&expr, old) {
            return true;
        }
    }
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(minus.trim()) {
        if let Some(init) = local.init {
            if is_dedicated_expr(&init.expr, old) {
                return true;
            }
        }
    }
    false
}

fn is_dedicated_expr(expr: &syn::Expr, old: &str) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                if path.qself.is_none() && path.path.segments.last().is_some_and(|s| s.ident == old)
                {
                    return true;
                }
            }
            false
        }
        syn::Expr::Macro(m) => m.mac.path.segments.last().is_some_and(|s| s.ident == old),
        _ => false,
    }
}

/// Count `mod old;` declarations across the involved pre-images.
fn count_mod_defs(pre: &HashMap<String, String>, pairs: &[Pair], ident: &str) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    let mut count = 0usize;
    for pair in pairs {
        if !seen.insert(pair.path.clone()) {
            continue;
        }
        if let Some(text) = pre.get(&pair.path) {
            for line in text.lines() {
                if let Some((_, kind, id, suffix)) = split_item(line) {
                    if kind == "mod" && id == ident && suffix.trim() == ";" {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// The `rename from` / `rename to` values of a headers-only rename
/// section (decoded, unprefixed), or `None` when absent.
fn rename_paths(section: &str) -> Option<(String, String)> {
    let mut from: Option<String> = None;
    let mut to: Option<String> = None;
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("rename from ") {
            from = Some(rsmpl_patch::decode_git_path_token(rest.trim()));
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            to = Some(rsmpl_patch::decode_git_path_token(rest.trim()));
        }
    }
    Some((from?, to?))
}

/// True when the file move names the `mod` delta: flat (`old.rs` →
/// `new.rs`) or `mod.rs` (`old/mod.rs` → `new/mod.rs`) layouts with
/// otherwise equal dirs and the same layout on both sides. Anything
/// else (moved dirs, layout switches, second renames) declines the join.
fn rename_matches(old: &str, new: &str, from: &str, to: &str) -> bool {
    fn split(path: &str) -> (String, String, bool) {
        // (dir, mod_name, is_mod_rs): `a/old.rs` → (`a`, `old`, false);
        // `a/old/mod.rs` → (`a`, `old`, true); `old.rs` → (``, `old`, false).
        let path = path.trim();
        if let Some(dir) = path.strip_suffix("/mod.rs") {
            if let Some((parent, name)) = dir.rsplit_once('/') {
                return (parent.to_string(), name.to_string(), true);
            }
            return (String::new(), dir.to_string(), true);
        }
        if let Some(stem) = path.strip_suffix(".rs") {
            if let Some((dir, file)) = stem.rsplit_once('/') {
                return (dir.to_string(), file.to_string(), false);
            }
            return (String::new(), stem.to_string(), false);
        }
        (String::new(), path.to_string(), false)
    }
    let (from_dir, from_mod, from_mod_rs) = split(from);
    let (to_dir, to_mod, to_mod_rs) = split(to);
    from_mod == old && to_mod == new && from_dir == to_dir && from_mod_rs == to_mod_rs
}

/// Emit the single-hunk follow emission under `rule`.
fn emit(path: &str, rule: &str, old: &str, new: &str) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- mod {old}\n+ mod {new}\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
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

    const LIB_PRE: &str = "mod old;\nuse crate::old::f;\nfn m() {\n    crate::old::f();\n}\n";
    const USE_PRE: &str = "use crate::old::g;\nfn n() {\n    crate::old::g();\n}\n";

    fn mod_sections() -> Vec<String> {
        vec![
            section(
                "src/lib.rs",
                "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n@@ -2,1 +2,1 @@\n-use crate::old::f;\n+use crate::new::f;\n@@ -4,1 +4,1 @@\n-    crate::old::f();\n+    crate::new::f();\n",
            ),
            section(
                "src/other.rs",
                "@@ -1,1 +1,1 @@\n-use crate::old::g;\n+use crate::new::g;\n@@ -3,1 +3,1 @@\n-    crate::old::g();\n+    crate::new::g();\n",
            ),
        ]
    }

    fn rename_section() -> String {
        "diff --git a/src/old.rs b/src/new.rs\nsimilarity index 100%\nrename from src/old.rs\nrename to src/new.rs\n"
            .to_string()
    }

    #[test]
    fn mod_delta_extracts_declaration_only() {
        assert!(split_mod_delta("mod old;", "mod new;").is_some());
        assert!(split_mod_delta("mod old;", "mod old;").is_none());
        assert!(split_mod_delta("mod old {", "mod new {").is_none());
        assert!(split_mod_delta("struct Old {", "struct New {").is_none());
        assert!(split_mod_delta("use crate::old;", "use crate::new;").is_none());
    }

    #[test]
    fn rename_gate_accepts_flat_and_mod_layouts() {
        assert!(rename_matches("old", "new", "src/old.rs", "src/new.rs"));
        assert!(rename_matches("old", "new", "old.rs", "new.rs"));
        assert!(rename_matches(
            "old",
            "new",
            "src/old/mod.rs",
            "src/new/mod.rs"
        ));
        assert!(!rename_matches("old", "new", "src/old.rs", "src/other.rs"));
        assert!(!rename_matches("old", "new", "a/old.rs", "b/new.rs"));
        assert!(!rename_matches(
            "old",
            "new",
            "src/old.rs",
            "src/new/mod.rs"
        ));
    }

    #[test]
    fn two_file_mod_intent_fires_and_round_trips() {
        let sections = mod_sections();
        let pre = pre_map(&[("src/lib.rs", LIB_PRE), ("src/other.rs", USE_PRE)]);
        let mut used = Vec::new();
        let (out, path, rule) =
            try_mod_rename_intent_sweep(&sections, &pre, &mut used).expect("intent must fire");
        assert_eq!(path, "src/lib.rs");
        assert_eq!(out.len(), 1, "one intent section, got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[0].contains("- mod old"), "got:\n{}", out[0]);
        assert!(!rule.is_empty());
        validated_live(&out[0], &pre);
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            !files["src/lib.rs"].contains("old"),
            "got: {}",
            files["src/lib.rs"]
        );
        assert!(
            files["src/lib.rs"].contains("mod new;"),
            "got: {}",
            files["src/lib.rs"]
        );
        assert!(
            files["src/other.rs"].contains("crate::new::g"),
            "got: {}",
            files["src/other.rs"]
        );
        assert!(
            !files["src/other.rs"].contains("old"),
            "got: {}",
            files["src/other.rs"]
        );
    }

    #[test]
    fn rename_plus_mod_plus_uses_fire_with_rename_preserved() {
        let mut sections = vec![rename_section()];
        sections.extend(mod_sections());
        let pre = pre_map(&[
            ("src/lib.rs", LIB_PRE),
            ("src/other.rs", USE_PRE),
            ("src/new.rs", "pub fn f() {}\n"),
        ]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_mod_rename_intent_sweep(&sections, &pre, &mut used).expect("intent must fire");
        assert_eq!(path, "src/lib.rs");
        assert_eq!(out.len(), 2, "rename + intent, got: {out:?}");
        assert!(
            out[0].contains("rename from src/old.rs"),
            "got:\n{}",
            out[0]
        );
        assert!(out[0].contains("rename to src/new.rs"), "got:\n{}", out[0]);
        assert!(out[1].contains("# follow=uses"), "got:\n{}", out[1]);
        validated_live(&out[1], &pre);
    }

    #[test]
    fn rename_mismatch_declines() {
        let mut sections = vec![
            "diff --git a/src/other.rs b/src/renamed.rs\nsimilarity index 100%\nrename from src/other.rs\nrename to src/renamed.rs\n"
                .to_string(),
        ];
        sections.extend(mod_sections());
        let pre = pre_map(&[("src/lib.rs", LIB_PRE), ("src/other.rs", USE_PRE)]);
        let mut used = Vec::new();
        assert!(try_mod_rename_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn twin_mod_definitions_decline() {
        let sections = mod_sections();
        let pre = pre_map(&[
            ("src/lib.rs", LIB_PRE),
            ("src/other.rs", "mod old;\nuse crate::old::g;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_mod_rename_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn unproven_receiver_partial_fires() {
        // `c.rs` use spells `crate::old` through a path the follow
        // cannot prove (a same-name `mod old` twin in another file
        // would veto, so here: a use line whose plus side the probe
        // cannot reproduce because the minus line is not a path the
        // follow claims — a plain string literal mentioning old).
        // The intent claims the proven files, `c.rs` stays ordinary,
        // and the joint patch round-trips byte-identical.
        let lib = "mod old;\nuse crate::old::f;\nfn m() {\n    crate::old::f();\n}\n";
        let use_b = "use crate::old::f;\nfn m() {\n    crate::old::f();\n}\n";
        let use_c = "fn h() -> &'static str {\n    \"old\"\n}\n";
        let sections = vec![
            section(
                "src/lib.rs",
                "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n@@ -2,1 +2,1 @@\n-use crate::old::f;\n+use crate::new::f;\n@@ -4,1 +4,1 @@\n-    crate::old::f();\n+    crate::new::f();\n",
            ),
            section(
                "src/b.rs",
                "@@ -1,1 +1,1 @@\n-use crate::old::f;\n+use crate::new::f;\n@@ -3,1 +3,1 @@\n-    crate::old::f();\n+    crate::new::f();\n",
            ),
            section(
                "src/c.rs",
                "@@ -2,1 +2,1 @@\n-    \"old\"\n+    \"new\"\n",
            ),
        ];
        let pre = pre_map(&[
            ("src/lib.rs", lib),
            ("src/b.rs", use_b),
            ("src/c.rs", use_c),
        ]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_mod_rename_intent_sweep(&sections, &pre, &mut used).expect("partial must fire");
        assert_eq!(path, "src/lib.rs");
        assert_eq!(out.len(), 2, "intent + ordinary remainder, got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[1].contains("\"new\""), "got:\n{}", out[1]);
        let joint = out.join("");
        validated_live(&joint, &pre);
    }

    #[test]
    fn opaque_macro_use_declines() {
        // A `use` line hiding inside a macro body is opaque to the
        // engine: the follow vetoes without fallback, the probe apply
        // errors, and the whole intent declines (stays ordinary, never
        // half-claimed).
        let lib = "mod old;\nmacro_rules! m {\n    () => { use crate::old::f; };\n}\n";
        let sections = vec![
            section("src/lib.rs", "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n"),
            section(
                "src/b.rs",
                "@@ -1,1 +1,1 @@\n-use crate::old::f;\n+use crate::new::f;\n",
            ),
        ];
        let pre = pre_map(&[("src/lib.rs", lib), ("src/b.rs", "use crate::old::f;\n")]);
        let mut used = Vec::new();
        assert!(try_mod_rename_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn full_pipeline_compresses() {
        let sections = mod_sections();
        let pre = pre_map(&[("src/lib.rs", LIB_PRE), ("src/other.rs", USE_PRE)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- mod old"), "got:\n{out}");
        validated_live(&out, &pre);
    }
}
