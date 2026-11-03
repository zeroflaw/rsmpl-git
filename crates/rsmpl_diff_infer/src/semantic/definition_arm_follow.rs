//! Definition-addition match-arm follow-through (WI-12d).
//!
//! Split from `definition_addition` per D5: the variant-addition
//! group detector plus its arm-classification helpers and tests. The
//! shared group machinery (`FollowHunk`, `joint_reproduces`,
//! `insertion_point`, the single-insertion emitter) stays in
//! `definition_addition` and is imported.
//!
//! A file section holding exactly one pure-insertion *variant*
//! addition plus K pure-insertion match-arm hunks naming the added
//! variant compresses to the definition hunk plus K per-site concrete
//! ranged arm hunks (see `try_variant_addition_follow`).

use super::definition_addition::{
    insertion_point, joint_reproduces, try_definition_addition, variant_ident, FollowHunk,
};
use super::unique_rule;

/// WI-12d: variant-addition match-arm follow-through.
///
/// A file section holding exactly one pure-insertion *variant*
/// addition (the WI-12b shape, `+ C` inside proven `enum E`) plus
/// K ≥ 1 pure-insertion arm hunks (`-M,0` with one `+ <arm>,` line)
/// whose arms name the added variant compresses to the definition
/// hunk plus K per-site concrete ranged arm hunks (`@@ -M,1 @@` +
/// `  match <scrut> {` + `+ <arm>,`). Each arm's path must name the
/// variant: a qualified prefix must equal the def enum (`E::C`), or
/// a bare `C` needs a `use …E::*` glob line in the pre-image
/// (unanchored bare arms decline the group). Hetero bodies ride
/// per-site concrete (never holes).
///
/// All-or-nothing over the section's hunks: any non-arm hunk
/// (balanced pair, second variant insertion, variant mismatch,
/// unanchored bare, unrelated insertion) declines the whole group
/// (`None` — the per-hunk fallback then still compresses the
/// definition insertion alone). Totality is the joint scratch-apply
/// byte-compare probe and the guard is the scaled precision guard
/// (shared with the WI-12c group).
pub(crate) fn try_variant_addition_follow(
    section: &str,
    path: &str,
    file_old: Option<&str>,
    used_rules: &mut Vec<String>,
) -> Option<Vec<FollowHunk>> {
    let pre = file_old?;
    // Split prelude from native hunks (same shape as the field group).
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(String::new());
        }
        if let Some(h) = current.as_mut() {
            h.push_str(line);
            h.push('\n');
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    if hunks.len() < 2 {
        return None;
    }
    // Every hunk must be pure-insertion; exactly one must be the
    // definition addition (an arm line never parses as a variant, so
    // the def is unambiguous — but prove it, don't assume position).
    let mut pairs: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    for h in &hunks {
        let mut minuses: Vec<String> = Vec::new();
        let mut pluses: Vec<String> = Vec::new();
        for line in h.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            }
        }
        if !minuses.is_empty() || pluses.len() != 1 {
            return None;
        }
        insertion_point(h)?;
        pairs.push((minuses, pluses));
    }
    let checkpoint = used_rules.len();
    // Find the def hunk: exactly one insertion must validate as a
    // member addition (reuses the container proof, the probe, and the
    // rule reservation).
    let mut def_idx: Option<usize> = None;
    let mut def_section: Option<String> = None;
    for (i, h) in hunks.iter().enumerate() {
        let probe_before = used_rules.len();
        match try_definition_addition(
            &[],
            &[pairs[i].1[0].as_str()],
            path,
            Some(pre),
            h,
            used_rules,
        ) {
            Some((s, _)) => {
                if def_idx.is_some() {
                    used_rules.truncate(checkpoint);
                    return None;
                }
                def_idx = Some(i);
                def_section = Some(s);
            }
            None => used_rules.truncate(probe_before),
        }
    }
    let (def_idx, def_section) = match (def_idx, def_section) {
        (Some(i), Some(s)) => (i, s),
        _ => {
            used_rules.truncate(checkpoint);
            return None;
        }
    };
    // WI-12d owns variant additions only: the def emission names its
    // container (`  enum E {` vs `  struct S {`). Field insertions
    // belong to the WI-12c group.
    let Some((enum_name, variant)) = def_container_variant(&def_section) else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let glob = has_glob_import(pre, &enum_name);
    // Every other hunk must add an arm for the new variant.
    let mut out: Vec<FollowHunk> = Vec::new();
    let def_body = def_section
        .split_once('\n')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_default();
    let def_rule = used_rules.last().cloned().unwrap_or_default();
    for (i, h) in hunks.iter().enumerate() {
        if i == def_idx {
            out.push(FollowHunk {
                emitted: def_body.clone(),
                native: h.clone(),
                detector: "definition_addition",
                changed: crate::totality::changed_old_lines(h),
                rule: def_rule.clone(),
            });
            continue;
        }
        let point = insertion_point(h).expect("checked above");
        let Some((arm, scrutinee)) =
            classify_arm_addition(&pairs[i].1[0], &enum_name, &variant, glob, pre, point)
        else {
            used_rules.truncate(checkpoint);
            return None;
        };
        let rule = unique_rule(used_rules, "definition_addition_follow");
        let emitted = format!("@@ -{point},1 @@ rule={rule}\n  match {scrutinee} {{\n+ {arm}\n",);
        out.push(FollowHunk {
            emitted,
            native: h.clone(),
            detector: "definition_addition_follow",
            changed: crate::totality::changed_old_lines(h),
            rule,
        });
    }
    // Scaled precision guard: each emitted hunk carries the same
    // 16-byte proof budget as the WI-12 single shape.
    let native_len: usize = hunks.iter().map(|h| super::normalized_hunk_len(h)).sum();
    let emitted_len: usize = out.iter().map(|h| h.emitted.len()).sum();
    if emitted_len > native_len + super::guard::precision_allowance(out.len(), 0) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Joint scratch-apply probe: every hunk matches once AND the post
    // equals the native post byte-identically.
    if !joint_reproduces(pre, path, section, &out) {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some(out)
}

/// Read back the def emission's container + variant (`  enum E {` /
/// `+ C,…`). `None` for field additions (WI-12c) or malformed
/// emissions.
fn def_container_variant(def_section: &str) -> Option<(String, String)> {
    let mut container: Option<String> = None;
    let mut member: Option<String> = None;
    for line in def_section.lines().skip(1) {
        if line.starts_with("@@") || line.starts_with('#') {
            continue;
        } else if let Some(body) = line.strip_prefix("  ") {
            let t = body.trim();
            let name = t.strip_prefix("enum ")?;
            container = name.strip_suffix('{').map(|s| s.trim().to_string());
        } else if let Some(plus) = line.strip_prefix('+') {
            member = Some(plus.trim().to_string());
        }
    }
    let variant = variant_ident(member?.as_str())?;
    Some((container?, variant))
}

/// A native insertion plus line as an arm adding `variant` to
/// `enum_name`: parses as a single `syn::Arm` whose path names the
/// variant (qualified prefix must equal the enum; bare needs the
/// glob anchor). Returns the verbatim arm text plus the scrutinee of
/// the innermost pre-image `match` spanning the insertion point.
fn classify_arm_addition(
    plus_raw: &str,
    enum_name: &str,
    variant: &str,
    glob: bool,
    pre: &str,
    point: usize,
) -> Option<(String, String)> {
    let t = plus_raw.trim();
    if t.is_empty() {
        return None;
    }
    let arm: syn::Arm = syn::parse_str(t).ok()?;
    if arm_matches_other_variant(&arm, variant) {
        return None;
    }
    // Path ownership: qualified prefix must name the def enum, bare
    // needs the glob import in scope.
    let bare = is_bare_variant_path(&arm)?;
    if bare {
        if !glob {
            return None;
        }
    } else if !is_qualified_variant_path(&arm, enum_name, variant) {
        return None;
    }
    let scrutinee = innermost_match_scrutinee(pre, point)?;
    Some((t.to_string(), scrutinee))
}

/// True when the arm's pattern names a *different* variant (last
/// path segment differs, or the arm is not a plain variant path at
/// all — bindings, wildcards, literals never join the group).
fn arm_matches_other_variant(arm: &syn::Arm, variant: &str) -> bool {
    let path = match &arm.pat {
        syn::Pat::Path(pp) if pp.qself.is_none() => &pp.path,
        syn::Pat::TupleStruct(ps) => &ps.path,
        syn::Pat::Struct(ps) => &ps.path,
        syn::Pat::Ident(pi)
            if pi.by_ref.is_none() && pi.mutability.is_none() && pi.subpat.is_none() =>
        {
            return pi.ident != variant;
        }
        _ => return true,
    };
    if path.leading_colon.is_some() {
        return true;
    }
    match path.segments.last() {
        Some(seg) => seg.ident != variant,
        None => true,
    }
}

/// True for a bare single-ident arm path (`C => …`, parsed as
/// `Pat::Ident` or a one-segment path). `None` for non-variant pats.
fn is_bare_variant_path(arm: &syn::Arm) -> Option<bool> {
    if let syn::Pat::Ident(pi) = &arm.pat {
        if pi.by_ref.is_none() && pi.mutability.is_none() && pi.subpat.is_none() {
            return Some(true);
        }
        return None;
    }
    let path = match &arm.pat {
        syn::Pat::Path(pp) if pp.qself.is_none() => &pp.path,
        syn::Pat::TupleStruct(ps) => &ps.path,
        syn::Pat::Struct(ps) => &ps.path,
        _ => return None,
    };
    if path.leading_colon.is_some() {
        return None;
    }
    Some(path.segments.len() == 1)
}

/// True for `E::C` (two segments, prefix == enum, last == variant).
fn is_qualified_variant_path(arm: &syn::Arm, enum_name: &str, variant: &str) -> bool {
    let path = match &arm.pat {
        syn::Pat::Path(pp) if pp.qself.is_none() => &pp.path,
        syn::Pat::TupleStruct(ps) => &ps.path,
        syn::Pat::Struct(ps) => &ps.path,
        _ => return false,
    };
    if path.leading_colon.is_some() || path.segments.len() != 2 {
        return false;
    }
    path.segments.first().is_some_and(|s| s.ident == enum_name)
        && path.segments.last().is_some_and(|s| s.ident == variant)
}

/// A `use …E::*` glob line in the pre-image (any module prefix).
fn has_glob_import(pre: &str, enum_name: &str) -> bool {
    pre.lines().any(|l| {
        let t = l.trim().strip_prefix("use ").unwrap_or("").trim();
        let t = t.strip_suffix(';').unwrap_or(t).trim();
        let mut segs: Vec<&str> = t.split("::").map(str::trim).collect();
        if segs.len() < 2 {
            return false;
        }
        segs.pop() == Some("*") && segs.pop() == Some(enum_name)
    })
}

/// Scrutinee (token-normalised) of the innermost pre-image `match`
/// strictly spanning the insertion point. `None` when no match spans
/// the point.
fn innermost_match_scrutinee(pre: &str, point: usize) -> Option<String> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;
    struct Probe {
        out: Vec<(u32, u32, String)>,
    }
    impl<'ast> Visit<'ast> for Probe {
        fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
            let e = &*node.expr;
            self.out.push((
                node.span().start().line as u32,
                node.span().end().line as u32,
                quote::quote!(#e)
                    .to_string()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            ));
            syn::visit::visit_expr_match(self, node);
        }
    }
    struct Walk {
        probe: Probe,
    }
    impl<'ast> Visit<'ast> for Walk {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Match(m) = node {
                self.probe.visit_expr_match(m);
            } else {
                syn::visit::visit_expr(self, node);
            }
        }
    }
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut walk = Walk {
        probe: Probe { out: Vec::new() },
    };
    for it in &file.items {
        walk.visit_item(it);
    }
    let mut best: Option<(u32, u32, String)> = None;
    for (s, e, sc) in walk.probe.out {
        if s <= (point as u32)
            && (point as u32) < e
            && best.as_ref().is_none_or(|(bs, _, _)| s >= *bs)
        {
            best = Some((s, e, sc));
        }
    }
    best.map(|(_, _, sc)| sc)
}

#[cfg(test)]
mod variant_follow_tests {
    use super::super::{infer_semantic_with_pre_images, unique_rule};
    use super::*;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const TWO_MATCH_PRE: &str = "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n\nfn g(e: E) -> u32 {\n    match e {\n        E::A => 10,\n    }\n}\n";
    const TWO_MATCH_POST: &str = "enum E {\n    A,\n    C,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n        E::C => 3,\n    }\n}\n\nfn g(e: E) -> u32 {\n    match e {\n        E::A => 10,\n        E::C => 30,\n    }\n}\n";

    fn two_match_section() -> String {
        section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    C,\n",
                "@@ -7,0 +8 @@\n",
                "+        E::C => 3,\n",
                "@@ -13,0 +14 @@\n",
                "+        E::C => 30,\n",
            ),
        )
    }

    #[test]
    fn hetero_two_arm_group_fires_and_round_trips() {
        let raw = two_match_section();
        let pre = pre_map(&[("f.rs", TWO_MATCH_PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("def + 2 hetero arms must compress");
        assert!(out.contains("  enum E {"), "def header, got:\n{out}");
        assert!(out.contains("+ C,"), "def member, got:\n{out}");
        assert!(out.contains("rule=r1"), "got:\n{out}");
        assert!(out.contains("rule=r2"), "got:\n{out}");
        assert!(out.contains("rule=r3"), "got:\n{out}");
        assert!(out.contains("  match e {"), "arm context, got:\n{out}");
        assert!(out.contains("+ E::C => 3,"), "got:\n{out}");
        assert!(out.contains("+ E::C => 30,"), "got:\n{out}");
        // Real-apply byte-compare: the joint probe's promise.
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], TWO_MATCH_POST);
    }

    #[test]
    fn glob_bare_arm_fires() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -4,0 +5 @@\n",
                "+    C,\n",
                "@@ -9,0 +10 @@\n",
                "+        C => 3,\n",
            ),
        );
        let pre_text =
            "use E::*;\n\nenum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        A => 1,\n    }\n}\n";
        let pre = pre_map(&[("f.rs", pre_text)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("glob-anchored bare arm must compress");
        assert!(out.contains("rule=r1"), "got:\n{out}");
        assert!(out.contains("rule=r2"), "got:\n{out}");
        assert!(out.contains("+ C => 3,"), "bare arm verbatim, got:\n{out}");
        assert!(!out.contains("E::C"), "stays bare, got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "use E::*;\n\nenum E {\n    A,\n    C,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        A => 1,\n        C => 3,\n    }\n}\n"
        );
    }

    #[test]
    fn unanchored_bare_declines_group_but_def_fires() {
        // Bare arm with no `use E::*` in scope: the group declines
        // (misattribution risk), the definition still fires alone.
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    C,\n",
                "@@ -7,0 +8 @@\n",
                "+        C => 3,\n",
            ),
        );
        let pre = "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";
        let mut used = Vec::new();
        assert!(
            try_variant_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "unanchored bare must decline the group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
        let mut used = Vec::new();
        assert!(
            try_definition_addition(
                &[],
                &["    C,"],
                "f.rs",
                Some(pre),
                "@@ -2,0 +3 @@\n+    C,\n",
                &mut used,
            )
            .is_some(),
            "definition still fires alone"
        );
    }

    #[test]
    fn middle_insertion_declines_group() {
        // The native arm lands between A and B, but emission always
        // appends before the closing brace: the joint probe fails, so
        // the group declines (def still fires alone).
        let raw = section(
            "f.rs",
            concat!(
                "@@ -3,0 +4 @@\n",
                "+    C,\n",
                "@@ -8,0 +9 @@\n",
                "+        E::C => 3,\n",
            ),
        );
        let pre = "enum E {\n    A,\n    B,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n        E::B => 2,\n    }\n}\n";
        let mut used = Vec::new();
        assert!(
            try_variant_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "middle insertion must decline the group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
    }

    #[test]
    fn variant_mismatch_declines_group() {
        // The arm names D while the definition adds C.
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    C,\n",
                "@@ -7,0 +8 @@\n",
                "+        E::D => 3,\n",
            ),
        );
        let pre = "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";
        let mut used = Vec::new();
        assert!(
            try_variant_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "variant mismatch must decline the group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
    }

    #[test]
    fn balanced_pair_declines_group() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    C,\n",
                "@@ -7,1 +7,1 @@\n",
                "-        E::A => 1,\n",
                "+        E::C => 3,\n",
            ),
        );
        let pre = "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";
        let mut used = Vec::new();
        assert!(
            try_variant_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "balanced pair must decline the group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
    }

    #[test]
    fn two_variant_insertions_decline_group() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    C,\n",
                "@@ -2,0 +4 @@\n",
                "+    D,\n",
            ),
        );
        let pre = "enum E {\n    A,\n}\n";
        let mut used = Vec::new();
        assert!(
            try_variant_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "two defs must decline the group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
    }

    #[test]
    fn unrelated_insertion_declines_group() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    C,\n",
                "@@ -7,0 +8 @@\n",
                "+        let y = 1;\n",
            ),
        );
        let pre = "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";
        let mut used = Vec::new();
        assert!(
            try_variant_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "non-arm insertion must decline the group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn scaled_guard_long_name_declines() {
        // A 60-char enum name prices the group out (same 16 × hunks
        // discipline as the field group).
        let long = "EWithAVeryLongNameThatExceedsThePrecisionGuardEasily1234";
        let pre_text = format!(
            "enum {long} {{\n    A,\n}}\n\nfn f(e: {long}) -> u32 {{\n    match e {{\n        {long}::A => 1,\n    }}\n}}\n"
        );
        let raw = section(
            "f.rs",
            &format!("@@ -3,0 +4 @@\n+    C,\n@@ -8,0 +9 @@\n+        {long}::C => 3,\n"),
        );
        let mut used = Vec::new();
        assert!(
            try_variant_addition_follow(&raw, "f.rs", Some(&pre_text), &mut used).is_none(),
            "over-guard group must decline"
        );
        assert!(used.is_empty(), "decline must burn no rules");
    }

    #[test]
    fn arm_classifier_matrix() {
        // Qualified fire, wrong-enum prefix declined, over-qualified
        // declined, bare needs the caller-side glob (classifier is
        // glob-blind: bare always classifies, the group gates).
        assert!(classify_arm_addition("E::C => 3,", "E", "C", false, "fn f() {}", 1).is_none());
        let pre = "fn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";
        assert!(classify_arm_addition("E::C => 3,", "E", "C", false, pre, 3).is_some());
        assert!(classify_arm_addition("F::C => 3,", "E", "C", false, pre, 3).is_none());
        assert!(classify_arm_addition("a::E::C => 3,", "E", "C", false, pre, 3).is_none());
        assert!(classify_arm_addition("E::D => 3,", "E", "C", false, pre, 3).is_none());
        // Bare without glob declines here (the anchor proof lives in
        // the group caller); with glob it fires.
        assert!(classify_arm_addition("C => 3,", "E", "C", false, pre, 3).is_none());
        let glob_pre = "use E::*;\nfn f(e: E) -> u32 {\n    match e {\n        A => 1,\n    }\n}\n";
        assert!(classify_arm_addition("C => 3,", "E", "C", true, glob_pre, 4).is_some());
        // Guards and payloads ride verbatim; bindings never join.
        assert!(classify_arm_addition("E::C if x > 1 => 2,", "E", "C", false, pre, 3).is_some());
        assert!(classify_arm_addition("E::C(..) => 0,", "E", "C", false, pre, 3).is_some());
        assert!(classify_arm_addition("x => 3,", "E", "C", true, glob_pre, 4).is_none());
        // Point outside any match declines.
        assert!(classify_arm_addition("E::C => 3,", "E", "C", false, pre, 1).is_none());
        let _ = unique_rule(&mut Vec::new(), "r1");
    }
}
