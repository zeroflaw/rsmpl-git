//! `macro_rules!` definition + invocation rename (WI-7b + WI-16, SPEC D.4).
//!
//! One `macro_rules! Old { … }` definition line plus N ≥ 1 bare
//! `Old!(…)` invocation lines sharing one ident delta compress to one
//! def hunk with `# follow=uses` (`- macro_rules! Old` /
//! `+ macro_rules! New`, WI-16 slice 1, byte-identical to the authored
//! form). The engine's `macro_paths` follow rewrites every invocation
//! by path identity; totality is the scratch line-equality probe over
//! the exact site set. When the follow probe declines (no pre-image,
//! opaque interior, qualified path), the detector falls back to the
//! WI-7b two-hunk combined form sharing `rule=rename_Old_to_New`:
//! the definition as a plain item-rename hunk plus the invocations as
//! one `# allow=many` `# macro=token_tree` verbatim sweep.
//! Cross-file def+use intents group
//! via the existing `item_rename` + `macro_rename` grouping; this
//! detector owns the same-file combined intent.
//!
//! v1 scope: bare invocation paths only (`crate::Old!` stays ordinary —
//! no qualified-macro-rules attribution without module resolution);
//! column-0 definitions only (block-level `macro_rules!` stays
//! ordinary); per-pair token-equal invocation bodies (the verbatim
//! `...` emission covers hetero bodies across sites, same as
//! `macro_rename`); exactly one definition pair (twin definitions stay
//! ordinary). Totality is pre-image-proven: one `macro_rules! Old`
//! definition covering its changed line plus `macro_holds` over the
//! invocation subset (untouched twins veto), and a scratch-apply probe
//! of the emitted form.

use syn::spanned::Spanned;

use super::macro_parts::{parse_macro_pair, split_macro_path};
use super::{item_emission_header, unique_rule};

/// Whole-section `macro_rules!` definition + invocation rename: exactly
/// one `macro_rules!` header pair plus N ≥ 1 invocation pairs sharing one
/// bare-ident delta. Returns the two-hunk combined section, or `None`
/// when the section is not exactly this shape.
pub(crate) fn try_macro_rules_rename(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return super::prov::Counted::Miss;
    }
    if changed.len() != minuses.len() {
        return super::prov::Counted::Miss;
    }
    // Partition pairs: exactly one `macro_rules!` definition, the rest
    // bare invocations. Anything else declines the whole section.
    let mut def_idx: Option<usize> = None;
    let mut inv_idx: Vec<usize> = Vec::new();
    for (i, (m, p)) in minuses.iter().zip(pluses.iter()).enumerate() {
        if is_macro_rules_def_pair(m, p).is_some() {
            if def_idx.is_some() {
                return super::prov::Counted::Miss; // twin definitions stay ordinary
            }
            def_idx = Some(i);
        } else if parse_macro_pair(m, p).is_some() {
            inv_idx.push(i);
        } else {
            return super::prov::Counted::Miss;
        }
    }
    let Some(def_idx) = def_idx else {
        return super::prov::Counted::Miss;
    };
    if inv_idx.is_empty() {
        return super::prov::Counted::Miss;
    }
    // Shared bare-ident delta across the definition and every invocation.
    let Some((def_old, def_new)) = is_macro_rules_def_pair(minuses[def_idx], pluses[def_idx])
    else {
        return super::prov::Counted::Miss;
    };
    if def_old == def_new || def_old.is_empty() || def_new.is_empty() {
        return super::prov::Counted::Miss;
    }
    for &i in &inv_idx {
        let Some((o, n)) = parse_macro_pair(minuses[i], pluses[i]) else {
            return super::prov::Counted::Miss;
        };
        let Some((o_pre, o_last)) = split_macro_path(&o) else {
            return super::prov::Counted::Miss;
        };
        let Some((n_pre, n_last)) = split_macro_path(&n) else {
            return super::prov::Counted::Miss;
        };
        // Bare only in v1; prefixes must be empty and last segments must
        // carry the definition delta.
        if !o_pre.is_empty() || !n_pre.is_empty() {
            return super::prov::Counted::Miss;
        }
        if o_last != def_old || n_last != def_new {
            return super::prov::Counted::Miss;
        }
    }
    // Pre-image totality.
    let pre = file_old;
    let def_site = changed[def_idx];
    if let Some(pre_text) = pre {
        if macro_rules_def_site(pre_text, &def_old) != Some(def_site) {
            return super::prov::Counted::Miss;
        }
        let inv_minuses: Vec<&str> = inv_idx.iter().map(|&i| minuses[i]).collect();
        let inv_changed: Vec<usize> = inv_idx.iter().map(|&i| changed[i]).collect();
        match crate::totality_counted::macro_verdict(
            file_old,
            &def_old,
            false,
            &inv_minuses,
            &inv_changed,
        ) {
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
    // WI-16 slice 1: the follow form wins when proven. The same
    // def+use shape raises as one def hunk with `# follow=uses`
    // (byte-identical to the authored fixture-45 hunk); the engine's
    // `macro_paths` follow rewrites every `Old!(…)` invocation by
    // path identity. The scratch line-equality probe below is
    // authoritative; the `parse_macro_pair` gate above only rejects
    // cheap non-shares.
    if let Some(pre_text) = pre {
        let probe = emit_follow(path, "probe", Some(pre_text), def_site, &def_old, &def_new);
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        if scratch_reproduces(pre_text, path, &probe, &pre_lines, changed, pluses) {
            let rule = unique_rule(used_rules, &format!("rename_{def_old}_to_{def_new}"));
            return super::prov::Counted::Fire(emit_follow(
                path,
                &rule,
                Some(pre_text),
                def_site,
                &def_old,
                &def_new,
            ));
        }
    }
    // Scratch-apply probe of the combined emission (T361 doctrine):
    // the exact bytes must apply cleanly with 1 + N matches.
    let probe_rule = "probe";
    let probe = emit(
        path, probe_rule, file_old, def_site, &inv_idx, changed, &def_old, &def_new,
    );
    if let Some(pre_text) = pre {
        if scratch_matches(pre_text, path, &probe) != Some(1 + inv_idx.len()) {
            match crate::totality_counted::scratch_line_verdict(Some(pre_text), minuses, changed) {
                crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
                    return super::prov::Counted::Totality { found, want };
                }
                crate::totality_counted::FamilyVerdict::OpaqueHit => {
                    return super::prov::Counted::Opaque;
                }
                _ => return super::prov::Counted::Miss,
            }
        }
    }
    let rule = unique_rule(used_rules, &format!("rename_{def_old}_to_{def_new}"));
    super::prov::Counted::Fire(emit(
        path, &rule, file_old, def_site, &inv_idx, changed, &def_old, &def_new,
    ))
}

/// A `-`/`+` line pair as a `macro_rules!` definition rename: both
/// column-0, same empty prefix and suffix, `macro_rules!` kind, idents
/// differ. Returns (old, new).
fn is_macro_rules_def_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let (o_pre, o_kind, o_ident, o_suf) = super::split_item(old_line)?;
    let (n_pre, n_kind, n_ident, n_suf) = super::split_item(new_line)?;
    if o_kind != "macro_rules!" || n_kind != "macro_rules!" {
        return None;
    }
    if o_pre != n_pre || o_suf != n_suf {
        return None;
    }
    if o_ident == n_ident || o_ident.is_empty() || n_ident.is_empty() {
        return None;
    }
    Some((o_ident, n_ident))
}

/// The 1-based start line of the unique `macro_rules! Old` definition in
/// `pre`, or `None` when there is not exactly one.
fn macro_rules_def_site(pre: &str, old: &str) -> Option<usize> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut sites: Vec<usize> = Vec::new();
    for it in &file.items {
        if let syn::Item::Macro(m) = it {
            let is_rules = m.mac.path.leading_colon.is_none()
                && m.mac.path.segments.len() == 1
                && m.mac.path.segments[0].ident == "macro_rules"
                && matches!(m.mac.path.segments[0].arguments, syn::PathArguments::None);
            if !is_rules {
                continue;
            }
            if m.ident.as_ref().is_some_and(|id| id == old) {
                sites.push(m.span().start().line);
            }
        }
    }
    if sites.len() == 1 {
        sites.into_iter().next()
    } else {
        None
    }
}

/// WI-16 slice 1: the follow emission — one def hunk with
/// `# follow=uses` (byte-identical to the authored fixture-45 hunk).
/// The engine rewrites every `Old!(…)` invocation by path identity;
/// the `scratch_reproduces` probe below proves the exact site set.
fn emit_follow(
    path: &str,
    rule: &str,
    file_old: Option<&str>,
    def_site: usize,
    old: &str,
    new: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    let mut head = item_emission_header(path, rule, file_old, &[def_site]);
    // The def-only spelling carries `# allow=many`; the follow hunk
    // is the single-proven-site spelling (59/77 doctrine).
    head = head.replace("# allow=many\n", "");
    out.push_str(strip_git_header(&head));
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- macro_rules! {old}\n+ macro_rules! {new}\n"));
    out
}

/// WI-16 slice 1 totality (member.rs line-equality precedent): the
/// follow emission applied to `pre` must reproduce exactly the
/// native plus lines at the changed sites with every other line
/// byte-identical. An untouched twin the follow would rewrite fails
/// the unchanged leg; a missed site fails the plus leg.
fn scratch_reproduces(
    pre: &str,
    path: &str,
    section: &str,
    pre_lines: &[&str],
    changed: &[usize],
    pluses: &[&str],
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(_) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    let post = &files[path];
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
        let line_no = idx + 1;
        if changed_set.contains(&line_no) {
            continue;
        }
        if pre_line != post_line {
            return false;
        }
    }
    true
}

/// Emit the combined two-hunk section sharing `rule`: the definition as
/// a plain item hunk plus the invocations as one verbatim token-tree
/// sweep.
#[allow(clippy::too_many_arguments)]
fn emit(
    path: &str,
    rule: &str,
    file_old: Option<&str>,
    def_site: usize,
    inv_idx: &[usize],
    changed: &[usize],
    old: &str,
    new: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    // Definition hunk: covering item span when provable, else the sweep
    // dummy (the multi-line item can never fit a 1-line range).
    let def_sec = item_emission_header(path, rule, file_old, &[def_site]);
    out.push_str(strip_git_header(&def_sec));
    out.push_str(&format!("- macro_rules! {old}\n+ macro_rules! {new}\n"));
    // Invocation hunk: tight range over invocation sites when narrow,
    // else the sweep dummy.
    let inv_sites: Vec<usize> = inv_idx.iter().map(|&i| changed[i]).collect();
    let inv_head = crate::emission_header(path, rule, &inv_sites);
    out.push_str(strip_git_header(&inv_head));
    // `emission_header` already ends with `# allow=many` or `# sweep`.
    out.push_str("# macro=token_tree\n");
    out.push_str(&format!("- {old}!(...)\n+ {new}!(...)\n"));
    out
}

/// Strip the leading `diff --git` line from an emission helper's output,
/// leaving the `@@` hunk header (plus any directive lines) for splicing
/// into a multi-hunk section.
fn strip_git_header(section: &str) -> &str {
    match section.find('\n') {
        Some(i) => &section[i + 1..],
        None => section,
    }
}

/// WI-19b def-only single (fixture 90): exactly one `macro_rules!`
/// definition pair, no invocations, no follow. Same def-site proof +
/// scratch probe as the combined path, definition hunk only.
pub(crate) fn try_macro_rules_def_only(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    let (old, new) = is_macro_rules_def_pair(minuses[0], pluses[0])?;
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    let pre = file_old?;
    if macro_rules_def_site(pre, &old) != Some(changed[0]) {
        return None;
    }
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, &format!("rename_{old}_to_{new}"));
    // WI-19b: concrete def-only hunk, no directives (59/77 doctrine:
    // default `allow=one` matches the one proven site).
    let def_sec =
        item_emission_header(path, &rule, Some(pre), changed).replace("# allow=many\n", "");
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(strip_git_header(&def_sec));
    out.push_str(&format!("- macro_rules! {old}\n+ macro_rules! {new}\n"));
    if scratch_matches(pre, path, &out) != Some(1) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Post line must equal the native plus (the count probe alone
    // cannot tell a same-count miss from a hit).
    let parsed = rsmpl_patch::parse_patch(&out).ok()?;
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).ok()?;
    let post_lines: Vec<&str> = files[path].lines().collect();
    if post_lines
        .get(changed[0].saturating_sub(1))
        .map(|l| l.trim())
        != Some(pluses[0].trim())
    {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some((out, "macro_rules_rename"))
}

/// Apply `section` (one `diff --git` section) to `pre` in memory and
/// return the total match count, or `None` when the patch does not parse
/// or apply cleanly.
fn scratch_matches(pre: &str, path: &str, section: &str) -> Option<usize> {
    let parsed = rsmpl_patch::parse_patch(section).ok()?;
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).ok()?;
    Some(report.rules.iter().map(|r| r.matches).sum())
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, unique_rule};
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

    const PRE: &str = "macro_rules! old_widget {\n    ($x:expr) => { $x + 1 };\n}\nfn f() {\n    let a = old_widget!(1);\n    let b = old_widget!(2, 3);\n    let c = old_widget! { 4 };\n    let d = old_widget!(5);\n    let e = other_widget!(6);\n}\n";

    fn raw_section() -> String {
        section(
            "f.rs",
            concat!(
                "@@ -1 +1 @@\n",
                "-macro_rules! old_widget {\n",
                "+macro_rules! new_widget {\n",
                "@@ -5 +5 @@\n",
                "-    let a = old_widget!(1);\n",
                "+    let a = new_widget!(1);\n",
                "@@ -6 +6 @@\n",
                "-    let b = old_widget!(2, 3);\n",
                "+    let b = new_widget!(2, 3);\n",
                "@@ -7 +7 @@\n",
                "-    let c = old_widget! { 4 };\n",
                "+    let c = new_widget! { 4 };\n",
                "@@ -8 +8 @@\n",
                "-    let d = old_widget!(5);\n",
                "+    let d = new_widget!(5);\n",
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
    fn def_pair_extracts_delta() {
        assert_eq!(
            is_macro_rules_def_pair("macro_rules! Old {", "macro_rules! New {"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            is_macro_rules_def_pair("macro_rules! Old {", "macro_rules! Old {"),
            None
        );
        assert_eq!(
            is_macro_rules_def_pair("macro_rules! Old {", "struct New {"),
            None
        );
        // Indented (block-level) definitions never classify.
        assert_eq!(
            is_macro_rules_def_pair("    macro_rules! Old {", "    macro_rules! New {"),
            None
        );
    }

    #[test]
    fn def_site_counts_unique_definition() {
        assert_eq!(macro_rules_def_site(PRE, "old_widget"), Some(1));
        assert_eq!(macro_rules_def_site(PRE, "missing"), None);
        let twin = PRE.replace(
            "fn f()",
            "macro_rules! old_widget {\n    () => {};\n}\nfn f()",
        );
        assert_eq!(macro_rules_def_site(&twin, "old_widget"), None);
    }

    #[test]
    fn fires_one_def_plus_four_uses() {
        // WI-16 slice 1: def + clean invocations raises as one follow
        // hunk (not the two-hunk verbatim combined form).
        let raw = raw_section();
        let pre = pre_map(&[("f.rs", PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("macro_rules rename must compress");
        assert_eq!(
            out.matches("rule=rename_old_widget_to_new_widget").count(),
            1,
            "got:\n{out}"
        );
        assert!(out.contains("- macro_rules! old_widget\n"), "got:\n{out}");
        assert!(out.contains("+ macro_rules! new_widget\n"), "got:\n{out}");
        assert!(out.contains("# follow=uses\n"), "got:\n{out}");
        assert!(!out.contains("# macro=token_tree\n"), "got:\n{out}");
        validated_live(&out, &pre);
        // Real-apply byte-identical: the post tree renames all five.
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        let post = &files["f.rs"];
        assert!(post.contains("macro_rules! new_widget {"), "got: {post}");
        assert_eq!(post.matches("new_widget!").count(), 4, "got: {post}");
        assert!(!post.contains("old_widget"), "got: {post}");
        // The distractor survives.
        assert!(post.contains("other_widget!(6)"), "got: {post}");
    }

    #[test]
    fn fixture_45_shape_fires_one_follow_hunk() {
        // WI-16 slice 1: the fixture-45 corpus form (def + single
        // `old_m!(41)` use) raises to the authored follow hunk.
        let pre = "macro_rules! old_m {\n    ($x:expr) => { $x + 1 };\n}\nfn f() -> i32 {\n    old_m!(41)\n}\n";
        let raw = section(
            "f.rs",
            "@@ -1 +1 @@\n-macro_rules! old_m {\n+macro_rules! new_m {\n@@ -5 +5 @@\n-    old_m!(41)\n+    new_m!(41)\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("fixture-45 shape must compress");
        assert_eq!(
            out.matches("rule=rename_old_m_to_new_m").count(),
            1,
            "got:\n{out}"
        );
        assert!(out.contains("# follow=uses\n"), "got:\n{out}");
        assert!(out.contains("- macro_rules! old_m\n"), "got:\n{out}");
        assert!(out.contains("+ macro_rules! new_m\n"), "got:\n{out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(files["f.rs"].contains("macro_rules! new_m {"));
        assert!(files["f.rs"].contains("new_m!(41)"));
        assert!(!files["f.rs"].contains("old_m"));
    }

    #[test]
    fn invocation_only_declines_without_def() {
        // WI-16 slice 1 boundary: a bare invocation pair with no def
        // pair is WI-3's single territory, never the follow intent.
        let minuses = ["    let a = old_widget!(1);"];
        let pluses = ["    let a = new_widget!(1);"];
        let mut used = Vec::new();
        assert!(
            try_macro_rules_rename(&minuses, &pluses, "f.rs", Some(PRE), &[5], &mut used).is_miss()
        );
    }

    #[test]
    fn nested_inside_other_macro_stays_ordinary() {
        // WI-16 slice 1 veto: an invocation nested inside another
        // macro's body (`outer!(old_widget!(1))`) names `outer`, not
        // `old_widget`, so the section declines wholly ordinary.
        let pre = "macro_rules! old_widget {\n    ($x:expr) => { $x };\n}\nmacro_rules! outer {\n    ($x:expr) => { $x };\n}\nfn f() {\n    outer!(old_widget!(1));\n}\n";
        let raw = section(
            "f.rs",
            "@@ -1 +1 @@\n-macro_rules! old_widget {\n+macro_rules! new_widget {\n@@ -8 +8 @@\n-    outer!(old_widget!(1));\n+    outer!(new_widget!(1));\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).is_none()
                || !infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                    .unwrap()
                    .contains("rename_old_widget_to_new_widget")
        );
    }

    #[test]
    fn twin_definitions_stay_ordinary() {
        let twin_pre = PRE.replace(
            "fn f()",
            "macro_rules! old_widget {\n    () => {};\n}\nfn f()",
        );
        let raw = raw_section();
        let pre = pre_map(&[("f.rs", &twin_pre)]);
        let mut used = Vec::new();
        let minuses: Vec<&str> = vec![
            "macro_rules! old_widget {",
            "    let a = old_widget!(1);",
            "    let b = old_widget!(2, 3);",
            "    let c = old_widget! { 4 };",
            "    let d = old_widget!(5);",
        ];
        let pluses: Vec<&str> = vec![
            "macro_rules! new_widget {",
            "    let a = new_widget!(1);",
            "    let b = new_widget!(2, 3);",
            "    let c = new_widget! { 4 };",
            "    let d = new_widget!(5);",
        ];
        assert!(
            try_macro_rules_rename(
                &minuses,
                &pluses,
                "f.rs",
                Some(&twin_pre),
                &[1, 8, 9, 10, 11],
                &mut used,
            )
            .is_miss(),
            "twin definitions must decline"
        );
        let _ = (raw, pre);
    }

    #[test]
    fn qualified_invocations_stay_ordinary() {
        let pre = "macro_rules! old_widget {\n    ($x:expr) => { $x };\n}\nfn f() {\n    let a = crate::old_widget!(1);\n}\n";
        let minuses = [
            "macro_rules! old_widget {",
            "    let a = crate::old_widget!(1);",
        ];
        let pluses = [
            "macro_rules! new_widget {",
            "    let a = crate::new_widget!(1);",
        ];
        let mut used = Vec::new();
        assert!(
            try_macro_rules_rename(&minuses, &pluses, "f.rs", Some(pre), &[1, 5], &mut used)
                .is_miss(),
            "qualified invocations must decline in v1"
        );
    }

    #[test]
    fn untouched_invocation_twin_vetoes() {
        // A fifth `old_widget!` untouched by the diff counts N+1 in the
        // engine probe, so the group stays ordinary.
        let twin_pre = PRE.replace(
            "    let e = other_widget!(6);",
            "    let e = other_widget!(6);\n    let f = old_widget!(7);",
        );
        let minuses = [
            "macro_rules! old_widget {",
            "    let a = old_widget!(1);",
            "    let b = old_widget!(2, 3);",
        ];
        let pluses = [
            "macro_rules! new_widget {",
            "    let a = new_widget!(1);",
            "    let b = new_widget!(2, 3);",
        ];
        let mut used = Vec::new();
        assert!(
            matches!(
                try_macro_rules_rename(
                    &minuses,
                    &pluses,
                    "f.rs",
                    Some(&twin_pre),
                    &[1, 5, 6],
                    &mut used,
                ),
                crate::semantic::prov::Counted::Totality { found: 5, want: 2 }
            ),
            "untouched twin must veto with its counts (5 invocation sites vs 2 claimed)"
        );
    }

    #[test]
    fn def_only_single_fires_and_round_trips() {
        // WI-19b: a lone `macro_rules!` definition (use untouched)
        // compresses to the def-only hunk with no follow.
        let pre = "macro_rules! old_m {\n    ($x:expr) => { $x + 1 };\n}\nfn f() -> i32 {\n    old_m!(41)\n}\n";
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-macro_rules! old_m {\n+macro_rules! new_m {\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("macro def-only must fire");
        assert!(out.contains("- macro_rules! old_m\n"), "got:\n{out}");
        assert!(out.contains("+ macro_rules! new_m\n"), "got:\n{out}");
        assert!(!out.contains("follow=uses"), "got:\n{out}");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(files["f.rs"].contains("macro_rules! new_m {"));
        assert!(
            files["f.rs"].contains("old_m!(41)"),
            "use stays, got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn lone_definition_declines_to_item_rename() {
        // One def pair alone is the `item_rename` single's domain, not
        // this detector's (which needs ≥1 invocation).
        let minuses = ["macro_rules! old_widget {"];
        let pluses = ["macro_rules! new_widget {"];
        let mut used = Vec::new();
        assert!(
            try_macro_rules_rename(&minuses, &pluses, "f.rs", Some(PRE), &[1], &mut used).is_miss()
        );
        let _ = unique_rule(&mut used, "r1");
    }
}
