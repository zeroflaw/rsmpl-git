//! `macro_rules!` definition + invocation rename (WI-7b, SPEC D.4).
//!
//! One `macro_rules! Old { … }` definition line plus N ≥ 1 bare
//! `Old!(…)` invocation lines sharing one ident delta compress to two
//! hunks sharing `rule=rename_Old_to_New`: the definition as a plain
//! item-rename hunk (`- macro_rules! Old` / `+ macro_rules! New`) and
//! the invocations as one `# allow=many` `# macro=token_tree` verbatim
//! sweep (`- Old!(…)` / `+ New!(…)`). Cross-file def+use intents group
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
//! of the combined emission.

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
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if changed.len() != minuses.len() {
        return None;
    }
    // Partition pairs: exactly one `macro_rules!` definition, the rest
    // bare invocations. Anything else declines the whole section.
    let mut def_idx: Option<usize> = None;
    let mut inv_idx: Vec<usize> = Vec::new();
    for (i, (m, p)) in minuses.iter().zip(pluses.iter()).enumerate() {
        if is_macro_rules_def_pair(m, p).is_some() {
            if def_idx.is_some() {
                return None; // twin definitions stay ordinary
            }
            def_idx = Some(i);
        } else if parse_macro_pair(m, p).is_some() {
            inv_idx.push(i);
        } else {
            return None;
        }
    }
    let def_idx = def_idx?;
    if inv_idx.is_empty() {
        return None;
    }
    // Shared bare-ident delta across the definition and every invocation.
    let (def_old, def_new) = is_macro_rules_def_pair(minuses[def_idx], pluses[def_idx])?;
    if def_old == def_new || def_old.is_empty() || def_new.is_empty() {
        return None;
    }
    for &i in &inv_idx {
        let (o, n) = parse_macro_pair(minuses[i], pluses[i])?;
        let (o_pre, o_last) = split_macro_path(&o)?;
        let (n_pre, n_last) = split_macro_path(&n)?;
        // Bare only in v1; prefixes must be empty and last segments must
        // carry the definition delta.
        if !o_pre.is_empty() || !n_pre.is_empty() {
            return None;
        }
        if o_last != def_old || n_last != def_new {
            return None;
        }
    }
    // Pre-image totality.
    let pre = file_old;
    let def_site = changed[def_idx];
    if let Some(pre_text) = pre {
        if macro_rules_def_site(pre_text, &def_old) != Some(def_site) {
            return None;
        }
        let inv_minuses: Vec<&str> = inv_idx.iter().map(|&i| minuses[i]).collect();
        let inv_changed: Vec<usize> = inv_idx.iter().map(|&i| changed[i]).collect();
        if !crate::totality::macro_holds(file_old, &def_old, &inv_minuses, &inv_changed) {
            return None;
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
            return None;
        }
    }
    let rule = unique_rule(used_rules, &format!("rename_{def_old}_to_{def_new}"));
    Some(emit(
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
        let raw = raw_section();
        let pre = pre_map(&[("f.rs", PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("macro_rules rename must compress");
        assert_eq!(
            out.matches("rule=rename_old_widget_to_new_widget").count(),
            2,
            "got:\n{out}"
        );
        assert!(out.contains("- macro_rules! old_widget\n"), "got:\n{out}");
        assert!(out.contains("- old_widget!(...)\n"), "got:\n{out}");
        assert!(out.contains("# macro=token_tree\n"), "got:\n{out}");
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
            .is_none(),
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
                .is_none(),
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
            try_macro_rules_rename(
                &minuses,
                &pluses,
                "f.rs",
                Some(&twin_pre),
                &[1, 5, 6],
                &mut used,
            )
            .is_none(),
            "untouched twin must veto"
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
            try_macro_rules_rename(&minuses, &pluses, "f.rs", Some(PRE), &[1], &mut used).is_none()
        );
        let _ = unique_rule(&mut used, "r1");
    }
}
