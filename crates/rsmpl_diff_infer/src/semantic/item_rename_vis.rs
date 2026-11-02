//! WI-5-R2 vis-evolving item-rename probe (SPEC D.4 bare-item joint).
//!
//! Sibling of the intent pre-pass (D5 line gate): the vis-evolving twin of
//! `try_item_rename` plus its closed-vis gate. The classifier lives in
//! `item_joint_visqual`; the emission rides the same module. This file owns
//! only the probe the pre-pass runs before emitting.
//!
//! Bare-item kinds only (`fn` rides the fn joint, `mod` rides WI-15c,
//! `impl` / `macro_rules!` take no visibility).

use super::{contains_word, ident_len, item_emission_header, split_item, unique_rule};

/// WI-5-R2 vis-evolving `try_item_rename` twin: the vis slot may step once
/// within the WI-1 closed set while every other slot stays token-equal
/// and the names differ. Bare-item kinds only (`fn` rides the fn joint,
/// `mod` rides WI-15c, `impl` / `macro_rules!` take no visibility).
/// Emits the concrete evolved terse pair the engine vis arm applies.
pub(crate) fn try_item_rename_vis_joint(
    old_line: &str,
    new_line: &str,
    path: &str,
    file_old: Option<&str>,
    sites: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let old = old_line.trim_end();
    let new = new_line.trim_end();
    if old == new {
        return None;
    }
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(old)?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(new)?;
    if old_kind == "fn" || old_kind != new_kind {
        return None;
    }
    if matches!(old_kind.as_str(), "impl" | "macro_rules!" | "mod") {
        return None;
    }
    if old_ident.is_empty() || new_ident.is_empty() || old_ident == new_ident {
        return None;
    }
    if old_prefix == new_prefix || old_suffix != new_suffix {
        return None;
    }
    if !is_closed_vis_prefix(&old_prefix) || !is_closed_vis_prefix(&new_prefix) {
        return None;
    }
    // Self-referential vis is an evolution, never a joint.
    if contains_word(&old_prefix, &old_ident)
        || contains_word(&old_prefix, &new_ident)
        || contains_word(&new_prefix, &old_ident)
        || contains_word(&new_prefix, &new_ident)
    {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_ident}_to_{new_ident}"));
    let mut hunk = item_emission_header(path, &rule, file_old, sites);
    hunk.push_str(&format!(
        "- {old_prefix}{old_kind} {old_ident}\n+ {new_prefix}{new_kind} {new_ident}\n"
    ));
    Some(hunk)
}

/// WI-1 closed vis set for terse renames (`""`, `pub`, `pub(crate)`,
/// `pub(super)`, `pub(in <path>)`). Mirrors the item-intent gate (local
/// copy per the per-module convention).
fn is_closed_vis_prefix(prefix: &str) -> bool {
    if prefix.is_empty() || prefix == "pub " {
        return true;
    }
    if prefix == "pub(crate) " || prefix == "pub(super) " {
        return true;
    }
    let Some(inner) = prefix
        .strip_prefix("pub(in ")
        .and_then(|r| r.strip_suffix(") "))
    else {
        return false;
    };
    let mut segs = inner.split("::").peekable();
    let mut first = true;
    while let Some(seg) = segs.next() {
        if seg.is_empty() {
            return false;
        }
        let head_ok =
            first && matches!(seg, "crate" | "self" | "super" | "$crate") && segs.peek().is_some();
        if !head_ok && ident_len(seg) != Some(seg.len()) {
            return false;
        }
        first = false;
    }
    !first
}

/// N=1 bare-item vis single (WI-14 slice 3 residue): same kind + same
/// name + same suffix, vis differs within the closed set. Emits the
/// concrete directive-free terse pair (`- struct S` / `+ pub struct S`,
/// bare like renames — the engine preserves generics/body/RHS) under
/// default `one`. Totality is the scratch-apply byte-compare probe
/// (T361): `matches == 1` plus post-tree line equality, so an untouched
/// twin vetoes to ordinary. `fn` rides `sig_evolution`, `mod` rides
/// file-move intents, `impl`/`macro_rules!` take no visibility — all
/// decline here.
pub(crate) fn try_item_vis_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    let old = minuses[0].trim_end();
    let new = pluses[0].trim_end();
    if old == new {
        return None;
    }
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(old)?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(new)?;
    if old_kind != new_kind || old_ident != new_ident || old_suffix != new_suffix {
        return None;
    }
    if old_prefix == new_prefix {
        return None;
    }
    if matches!(old_kind.as_str(), "fn" | "impl" | "macro_rules!" | "mod") {
        return None;
    }
    if !is_closed_vis_prefix(&old_prefix) || !is_closed_vis_prefix(&new_prefix) {
        return None;
    }
    if contains_word(&old_prefix, &old_ident)
        || contains_word(&old_prefix, &new_ident)
        || contains_word(&new_prefix, &old_ident)
        || contains_word(&new_prefix, &new_ident)
    {
        return None;
    }
    // Attr lines are separate diff lines; a diff line carrying `#[`
    // text is not a bare-item vis shape.
    if old.contains("#[") || new.contains("#[") {
        return None;
    }
    let pre = file_old?;
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, &format!("vis_{old_ident}"));
    let mut hunk = item_emission_header(path, &rule, Some(pre), changed);
    hunk = hunk.replace("# allow=many\n", "");
    hunk.push_str(&format!(
        "- {old_prefix}{old_kind} {old_ident}\n+ {new_prefix}{new_kind} {new_ident}\n"
    ));
    if !scratch_reproduces(pre, path, &hunk, changed, pluses) {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some(hunk)
}

/// Scratch-apply byte-compare probe (T361): the emitted bare pair must
/// apply with `matches == changed.len()` and reproduce the native plus
/// lines at every changed site with every other line byte-identical.
fn scratch_reproduces(
    pre: &str,
    path: &str,
    section: &str,
    changed: &[usize],
    pluses: &[&str],
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    if report.rules.iter().map(|r| r.matches).sum::<usize>() != changed.len() {
        return false;
    }
    let post = &files[path];
    let pre_lines: Vec<&str> = pre.lines().collect();
    let post_lines: Vec<&str> = post.lines().collect();
    if post_lines.len() != pre_lines.len() {
        return false;
    }
    for (site, plus) in changed.iter().zip(pluses.iter()) {
        let Some(post_line) = post_lines.get(site.saturating_sub(1)) else {
            return false;
        };
        if post_line.trim() != plus.trim() {
            return false;
        }
    }
    for (idx, (a, b)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        if changed.contains(&(idx + 1)) {
            continue;
        }
        if a != b {
            return false;
        }
    }
    true
}

/// N≥2 bare-item vis sweep (vis-single residue): one uniform closed-set
/// vis step over N same-kind items with per-site preserved names,
/// emitted as a single N-holed terse pair (`- struct N` /
/// `+ pub struct N`) under `# allow=many`. The uniform delta is proved
/// by construction (one shared minus/plus prefix across all pairs);
/// totality is the scratch-apply byte-compare probe (`matches == N`
/// plus per-site post equality, so an in-scope twin vetoes to today's
/// singles). `fn` rides the sig/header sweeps, `mod` rides file-move
/// intents, `impl`/`macro_rules!` take no visibility — all decline here.
/// Detector name `item_rename` (shared with the single; the whole-section
/// and per-hunk callers keep their existing guards — strict T375, which
/// a one-hunk diet sweep beats by construction).
pub(crate) fn try_item_vis_sweep(
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
    let mut uniform: Option<(String, String, String)> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let old = m.trim_end();
        let new = p.trim_end();
        if old == new {
            return None;
        }
        let (old_prefix, old_kind, old_ident, old_suffix) = split_item(old)?;
        let (new_prefix, new_kind, new_ident, new_suffix) = split_item(new)?;
        if old_kind != new_kind || old_ident != new_ident || old_suffix != new_suffix {
            return None;
        }
        if old_prefix == new_prefix {
            return None;
        }
        if matches!(old_kind.as_str(), "fn" | "impl" | "macro_rules!" | "mod") {
            return None;
        }
        if !is_closed_vis_prefix(&old_prefix) || !is_closed_vis_prefix(&new_prefix) {
            return None;
        }
        if contains_word(&old_prefix, &old_ident)
            || contains_word(&old_prefix, &new_ident)
            || contains_word(&new_prefix, &old_ident)
            || contains_word(&new_prefix, &new_ident)
        {
            return None;
        }
        // Attr lines are separate diff lines; a diff line carrying `#[`
        // text is not a bare-item vis shape.
        if old.contains("#[") || new.contains("#[") {
            return None;
        }
        // One uniform delta: every pair shares the same minus and plus
        // vis (divergent plus sides stay ordinary, never half-claimed).
        match &uniform {
            None => uniform = Some((old_prefix, new_prefix, old_kind)),
            Some((fp, np, fk)) => {
                if *fp != old_prefix || *np != new_prefix || *fk != old_kind {
                    return None;
                }
            }
        }
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
    let (old_prefix, new_prefix, kind) = uniform?;
    let pre = file_old?;
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, "vis_sweep");
    let mut hunk = item_emission_header(path, &rule, Some(pre), changed);
    hunk.push_str(&format!("- {old_prefix}{kind} N\n+ {new_prefix}{kind} N\n"));
    if !scratch_reproduces(pre, path, &hunk, changed, pluses) {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some(hunk)
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
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

    fn applied_post(patch: &str, pre: &HashMap<String, String>) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        files
    }

    #[test]
    fn struct_vis_add_fires_concrete() {
        let raw = section("f.rs", "@@ -1,1 +1,1 @@\n-struct S;\n+pub struct S;\n");
        let pre = pre_map(&[("f.rs", "struct S;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("struct vis single must fire");
        assert!(out.contains("- struct S\n"), "got:\n{out}");
        assert!(out.contains("+ pub struct S\n"), "got:\n{out}");
        assert!(!out.contains("# allow=many"), "allow-one, got:\n{out}");
        assert_eq!(applied_post(&out, &pre)["f.rs"], "pub struct S;\n");
    }

    #[test]
    fn sweep_fires_hetero_and_round_trips() {
        // N≥2 uniform vis step over hetero names: one N-holed sweep
        // hunk (not two singles), real-apply byte-identical.
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-struct S;\n-struct T;\n+pub struct S;\n+pub struct T;\n",
        );
        let pre = pre_map(&[("f.rs", "struct S;\nstruct T;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("vis sweep must fire");
        assert_eq!(
            out.matches("rule=").count(),
            1,
            "one sweep hunk, got:\n{out}"
        );
        assert!(out.contains("# allow=many\n"), "got:\n{out}");
        assert!(out.contains("- struct N\n"), "got:\n{out}");
        assert!(out.contains("+ pub struct N\n"), "got:\n{out}");
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "pub struct S;\npub struct T;\n"
        );
    }

    #[test]
    fn sweep_removal_and_pub_crate_fire() {
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-pub struct S;\n-pub struct T;\n+struct S;\n+struct T;\n",
        );
        let pre = pre_map(&[("f.rs", "pub struct S;\npub struct T;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("vis-removal sweep must fire");
        assert!(out.contains("- pub struct N\n"), "got:\n{out}");
        assert!(out.contains("+ struct N\n"), "got:\n{out}");
        assert_eq!(applied_post(&out, &pre)["f.rs"], "struct S;\nstruct T;\n");
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-pub struct S;\n-pub struct T;\n+pub(crate) struct S;\n+pub(crate) struct T;\n",
        );
        let pre = pre_map(&[("f.rs", "pub struct S;\npub struct T;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("pub -> pub(crate) sweep must fire");
        assert!(out.contains("+ pub(crate) struct N\n"), "got:\n{out}");
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "pub(crate) struct S;\npub(crate) struct T;\n"
        );
    }

    #[test]
    fn struct_vis_removal_and_pub_crate_fire() {
        let raw = section("f.rs", "@@ -1,1 +1,1 @@\n-pub struct S;\n+struct S;\n");
        let pre = pre_map(&[("f.rs", "pub struct S;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("vis removal must fire");
        assert!(out.contains("- pub struct S\n"), "got:\n{out}");
        assert!(out.contains("+ struct S\n"), "got:\n{out}");
        assert_eq!(applied_post(&out, &pre)["f.rs"], "struct S;\n");
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-pub struct S;\n+pub(crate) struct S;\n",
        );
        let pre = pre_map(&[("f.rs", "pub struct S;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("pub -> pub(crate) must fire");
        assert!(out.contains("+ pub(crate) struct S\n"), "got:\n{out}");
        assert_eq!(applied_post(&out, &pre)["f.rs"], "pub(crate) struct S;\n");
    }

    #[test]
    fn enum_and_trait_vis_fire() {
        let raw = section("f.rs", "@@ -1,1 +1,1 @@\n-enum E {\n+pub enum E {\n");
        let pre = pre_map(&[("f.rs", "enum E {\n    A,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("enum vis single must fire");
        assert!(out.contains("- enum E\n"), "got:\n{out}");
        assert!(out.contains("+ pub enum E\n"), "got:\n{out}");
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "pub enum E {\n    A,\n}\n"
        );
        let raw = section("f.rs", "@@ -1,1 +1,1 @@\n-trait T {\n+pub trait T {\n");
        let pre = pre_map(&[("f.rs", "trait T {\n    fn f(&self);\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("trait vis single must fire");
        assert!(out.contains("+ pub trait T\n"), "got:\n{out}");
    }

    #[test]
    fn twin_claims_only_changed_site() {
        // Range-scoped item hunks rewrite only the covered site: a twin
        // elsewhere stays bare, which is exactly the native diff — so the
        // single fires (no fn-style twin veto for range-scoped items).
        let raw = section("f.rs", "@@ -1,1 +1,1 @@\n-struct S;\n+pub struct S;\n");
        let pre = pre_map(&[("f.rs", "struct S;\nstruct S;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("twin single must fire for the claimed site");
        assert!(out.contains("- struct S\n"), "got:\n{out}");
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "pub struct S;\nstruct S;\n"
        );
    }

    #[test]
    fn two_slot_and_excluded_kinds_decline() {
        // vis+RHS is two slots (WI-7 territory).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-type A = Vec<u8>;\n+pub type A = Box<[u8]>;\n",
        );
        let pre = pre_map(&[("f.rs", "type A = Vec<u8>;\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none(),
            "vis+RHS must stay ordinary"
        );
        // fn rides sig_evolution, mod rides file-move intents,
        // impl/macro_rules take no visibility.
        let mut used = Vec::new();
        for (m, p) in [
            ("fn foo() {", "pub fn foo() {"),
            ("mod foo;", "pub mod foo;"),
            ("macro_rules! foo", "pub macro_rules! foo"),
        ] {
            assert!(
                super::try_item_vis_single(&[m], &[p], "f.rs", Some("x\n"), &[1], &mut used)
                    .is_none(),
                "{m} / {p} must decline"
            );
        }
        // Same-vis is not an evolution.
        let mut used = Vec::new();
        assert!(
            super::try_item_vis_single(
                &["struct S;"],
                &["struct S;"],
                "f.rs",
                Some("struct S;\n"),
                &[1],
                &mut used
            )
            .is_none(),
            "same-vis must decline"
        );
    }

    #[test]
    fn sweep_enum_multiline_covering_span_fires() {
        // Multi-line items: the covering span wraps whole bodies, the
        // terse pair still rewrites headers only.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-enum E {\n+pub enum E {\n@@ -4,1 +4,1 @@\n-enum F {\n+pub enum F {\n",
        );
        let pre = pre_map(&[("f.rs", "enum E {\n    A,\n}\nenum F {\n    B,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("enum sweep must fire");
        assert_eq!(
            out.matches("rule=").count(),
            1,
            "one sweep hunk, got:\n{out}"
        );
        assert!(out.contains("- enum N\n"), "got:\n{out}");
        assert!(out.contains("+ pub enum N\n"), "got:\n{out}");
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "pub enum E {\n    A,\n}\npub enum F {\n    B,\n}\n"
        );
    }

    #[test]
    fn sweep_veto_matrix() {
        // N=1 belongs to the single (sweep gate is N≥2).
        let mut used = Vec::new();
        assert!(
            super::try_item_vis_sweep(
                &["struct S;"],
                &["pub struct S;"],
                "f.rs",
                Some("struct S;\n"),
                &[1],
                &mut used
            )
            .is_none(),
            "N=1 must decline the sweep"
        );
        assert!(used.is_empty());
        // Divergent plus sides: never half-claimed under one rule; the
        // pipeline degrades to two singles instead.
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-struct S;\n-struct T;\n+pub struct S;\n+pub(crate) struct T;\n",
        );
        let pre = pre_map(&[("f.rs", "struct S;\nstruct T;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("divergent pair must still raise as singles");
        assert!(
            !out.contains("struct N"),
            "no N-hole sweep on divergent plus, got:\n{out}"
        );
        assert_eq!(out.matches("rule=").count(), 2, "two singles, got:\n{out}");
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "pub struct S;\npub(crate) struct T;\n"
        );
        // Two-slot (vis+RHS) vetoes the sweep and stays ordinary: no
        // single can claim `type A` either (vis+RHS is two evolving
        // slots for the single too).
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-type A = Vec<u8>;\n-type B = Vec<u8>;\n+pub type A = Box<[u8]>;\n+pub type B = Box<[u8]>;\n",
        );
        let pre = pre_map(&[("f.rs", "type A = Vec<u8>;\ntype B = Vec<u8>;\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none(),
            "vis+RHS sweep must stay ordinary"
        );
        // Mixed kinds decline the sweep (one pattern is kind-specific);
        // the pipeline degrades to two singles.
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-struct S;\n-enum E {\n+pub struct S;\n+pub enum E {\n",
        );
        let pre = pre_map(&[("f.rs", "struct S;\nenum E {\n    A,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("mixed kinds must still raise as singles");
        assert!(
            !out.contains("struct N"),
            "no N-hole sweep on mixed kinds, got:\n{out}"
        );
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "pub struct S;\npub enum E {\n    A,\n}\n"
        );
        // Identical minus lines are verbatim territory.
        let mut used = Vec::new();
        assert!(
            super::try_item_vis_sweep(
                &["struct S;", "struct S;"],
                &["pub struct S;", "pub struct S;"],
                "f.rs",
                Some("struct S;\nstruct S;\n"),
                &[1, 2],
                &mut used
            )
            .is_none(),
            "duplicate minus lines must decline"
        );
        assert!(used.is_empty());
        // Excluded kinds decline the sweep directly.
        for (m0, m1, p0, p1) in [
            (
                "fn foo() {",
                "fn bar() {",
                "pub fn foo() {",
                "pub fn bar() {",
            ),
            ("mod a;", "mod b;", "pub mod a;", "pub mod b;"),
        ] {
            let mut used = Vec::new();
            assert!(
                super::try_item_vis_sweep(
                    &[m0, m1],
                    &[p0, p1],
                    "f.rs",
                    Some("x\n"),
                    &[1, 2],
                    &mut used
                )
                .is_none(),
                "{m0} / {m1} must decline"
            );
            assert!(used.is_empty());
        }
    }

    #[test]
    fn sweep_in_scope_twin_vetoes_to_singles() {
        // An untouched same-shape item inside the covering span matches
        // the N-hole minus too (`matches == 3 ≠ 2`), so the sweep
        // vetoes — but the two changed sites still raise as singles via
        // the per-hunk fallback (separate hunks here).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-struct S;\n+pub struct S;\n@@ -3,1 +3,1 @@\n-struct T;\n+pub struct T;\n",
        );
        let pre = pre_map(&[("f.rs", "struct S;\nstruct U;\nstruct T;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("twin-separated singles must fire");
        assert!(
            !out.contains("struct N"),
            "no N-hole sweep with an in-scope twin, got:\n{out}"
        );
        assert_eq!(out.matches("rule=").count(), 2, "two singles, got:\n{out}");
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "pub struct S;\nstruct U;\npub struct T;\n"
        );
    }
}
