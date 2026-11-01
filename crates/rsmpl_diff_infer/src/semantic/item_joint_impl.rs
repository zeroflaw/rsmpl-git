//! WI-3 joint double-delta intent: `impl Trait for Type` renames as one.
//!
//! A trait rename composed with its self-type rename
//! (`trait Printable {}` → `trait Labeled {}` plus `struct Old;` →
//! `struct New;` plus the composed header `impl Printable for Old {` →
//! `impl Labeled for New {`) re-diffs wholly ordinary: both header sides
//! change, so no single-delta detector starts, and the item-intent
//! pre-pass needs exactly one definition delta. This pre-pass claims the
//! triple as one shared-rule intent (D13): two `@@ -1,1 @@` + `# sweep` +
//! `# follow=uses` hunks sharing one rule id (trait pair first, self-type
//! pair second — SPEC 14.8 def-before-member law). The composed `impl`
//! header is never stated: the two follows rewrite it (probe-verified),
//! exactly like the WI-5 field-add two-hunk emission states only the
//! def pair plus the addition.
//!
//! Slice law (WI-3): single section, exactly three balanced pairs, same
//! file, no extras. Any fourth pair, any unbalanced hunk, any hybrid
//! (pre-existing semantic hunks), or any changed line beyond the triple
//! declines — extra follow-use lines alongside the triple (WI-3b) and
//! cross-file trait/type/impl groups (WI-12) stay ordinary here, pinned.
//!
//! Totality is the D13 joint scratch-apply probe: the composed emission
//! applied to a `pre` clone must reproduce the native plus lines at
//! exactly the three claimed sites with every other line of every
//! pre-image file byte-identical (an off-diff same-name use rewritten by
//! either follow fails the unchanged leg — the uniqueness guard). The
//! T375 pure-section byte guard applies unchanged (diet emission must
//! beat the native section bytes).

use std::collections::HashMap;

use super::item_intent::{count_kind_defs, Pair};
use super::{split_item, unique_rule};

/// Whole-patch joint-`impl` pre-pass: returns the rebuilt section list
/// (the claimed section replaced by the single shared-rule emission) plus
/// the definition path and rule for provenance, or `None` when the patch
/// holds no provable double-delta intent. Reserved rule names are
/// released on decline.
pub(crate) fn try_joint_impl_rename(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    // Single-section slice: exactly one section holding exactly three
    // balanced pairs (same file). Any unbalanced hunk or hybrid section
    // (pre-existing semantic hunks) declines the whole pre-pass.
    struct Cand {
        sec: usize,
        path: String,
        section: String,
        pairs: Vec<(String, String, usize)>,
    }
    let mut cand: Option<Cand> = None;
    for (i, sec) in sections.iter().enumerate() {
        let header = sec.lines().next().unwrap_or("");
        if !header.starts_with("diff --git ") || sec.contains('\r') {
            continue;
        }
        if super::is_rename_header(header) {
            continue;
        }
        let rest = header.strip_prefix("diff --git ")?;
        let path = crate::b_side_path(rest)?;
        if !path.ends_with(".rs") {
            continue;
        }
        let (_prelude, semantic, native_view) = super::split_semantic_native(sec);
        if !semantic.is_empty() {
            continue;
        }
        let Some(native_sec) = native_view else {
            continue;
        };
        let hunks = super::split_native_hunks(&native_sec);
        if hunks.iter().any(|h| !h.balanced) {
            used_rules.truncate(checkpoint);
            return None;
        }
        let mut pairs: Vec<(String, String, usize)> = Vec::new();
        for h in &hunks {
            if h.minuses.len() != h.sites.len() || h.minuses.len() != h.pluses.len() {
                used_rules.truncate(checkpoint);
                return None;
            }
            for ((m, p), s) in h.minuses.iter().zip(h.pluses.iter()).zip(h.sites.iter()) {
                pairs.push((m.clone(), p.clone(), *s));
            }
        }
        if pairs.is_empty() {
            continue;
        }
        if pairs.len() != 3 {
            continue;
        }
        if cand.is_some() {
            // Two 3-pair sections: each is a separate triple only if both
            // composed — this slice claims at most one triple per patch
            // (the second stays for a later pass; it cannot run twice in
            // one chain walk, so decline to keep the probe total).
            used_rules.truncate(checkpoint);
            return None;
        }
        cand = Some(Cand {
            sec: i,
            path: path.to_string(),
            section: sec.clone(),
            pairs,
        });
    }
    let cand = cand?;
    // Role assignment: exactly one trait def pair, one self-type def pair,
    // one composed impl double-delta — each role exactly once.
    let mut trait_leg: Option<(String, String, String, usize)> = None;
    let mut ty_leg: Option<(String, String, String, String, usize)> = None;
    let mut impl_leg: Option<(String, String, String, String, usize)> = None;
    for (m, p, site) in &cand.pairs {
        if let Some((prefix, kind, old, new)) = split_def_pair(m, p) {
            if kind == "trait" {
                if trait_leg.is_some() {
                    used_rules.truncate(checkpoint);
                    return None;
                }
                trait_leg = Some((prefix, old, new, *site));
                continue;
            }
            if matches!(kind.as_str(), "struct" | "enum" | "union") {
                if ty_leg.is_some() {
                    used_rules.truncate(checkpoint);
                    return None;
                }
                ty_leg = Some((prefix, kind, old, new, *site));
                continue;
            }
            used_rules.truncate(checkpoint);
            return None;
        }
        if let Some((ot, nt, oy, ny)) = parse_impl_double_delta(m, p) {
            if impl_leg.is_some() {
                used_rules.truncate(checkpoint);
                return None;
            }
            impl_leg = Some((ot, nt, oy, ny, *site));
            continue;
        }
        used_rules.truncate(checkpoint);
        return None;
    }
    let Some((tr_prefix, ot, nt, tr_site)) = trait_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let Some((ty_prefix, ty_kind, oy, ny, ty_site)) = ty_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let Some((iot, int_, ioy, iny, impl_site)) = impl_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    // Composition: the impl header must spell exactly the two renames.
    if iot != ot || int_ != nt || ioy != oy || iny != ny {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Twin-definition veto (unique-definition rule, SPEC 14.8): exactly
    // one `trait ot` and one `{kind} oy` definition across the claimed
    // file's pre-image.
    let Some(pre_text) = pre.get(&cand.path) else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let probes = [Pair {
        sec: cand.sec,
        path: cand.path.clone(),
        minus: String::new(),
        plus: String::new(),
        site: tr_site,
    }];
    if count_kind_defs(pre, &probes, "trait", &ot) != 1 {
        used_rules.truncate(checkpoint);
        return None;
    }
    if count_kind_defs(pre, &probes, &ty_kind, &oy) != 1 {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Pre-image alignment: each claimed site must show its minus line.
    let pre_lines: Vec<&str> = pre_text.lines().collect();
    let at = |site: usize| pre_lines.get(site.saturating_sub(1)).copied().unwrap_or("");
    // Map each role back to its raw minus text for the alignment check.
    let minus_of = |site: usize| {
        cand.pairs
            .iter()
            .find(|(_, _, s)| *s == site)
            .map(|(m, _, _)| m.as_str())
            .unwrap_or("")
    };
    for site in [tr_site, ty_site, impl_site] {
        if at(site).trim() != minus_of(site).trim() {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    // Build the emission: trait follow hunk first, self-type second, one
    // shared rule. Concrete prefixes ride literally (WI-1 law).
    let rule = unique_rule(used_rules, &format!("rename_{ot}_to_{nt}"));
    let mut emission = crate::git_header(&cand.path);
    emission.push('\n');
    emission.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    emission.push_str("# sweep\n");
    emission.push_str("# follow=uses\n");
    emission.push_str(&format!(
        "- {tr_prefix}trait {ot}\n+ {tr_prefix}trait {nt}\n"
    ));
    emission.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    emission.push_str("# sweep\n");
    emission.push_str("# follow=uses\n");
    emission.push_str(&format!(
        "- {ty_prefix}{ty_kind} {oy}\n+ {ty_prefix}{ty_kind} {ny}\n"
    ));
    // D13 joint probe: composed emission on a `pre` clone must equal pre
    // with exactly the three minus lines replaced by the plus lines, in
    // every pre-image file (off-diff same-name uses rewritten by either
    // follow fail the unchanged leg).
    if !probe_joint(pre, &emission, &cand.pairs) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // T375 pure-section byte guard (diet must beat native).
    if emission.len() >= cand.section.len() {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(sections.len());
    for (i, sec) in sections.iter().enumerate() {
        if i == cand.sec {
            out.push(emission.clone());
        } else {
            out.push(sec.clone());
        }
    }
    Some((out, cand.path.clone(), rule))
}

/// A `split_item` definition delta, or `None` for use/impl lines. Mirrors
/// `item_intent`'s private `split_delta` (this module must not grow that
/// 1912-line file per D5).
fn split_def_pair(minus: &str, plus: &str) -> Option<(String, String, String, String)> {
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(minus.trim_end())?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(plus.trim_end())?;
    if old_kind != new_kind || old_prefix != new_prefix || old_suffix != new_suffix {
        return None;
    }
    if old_ident == new_ident {
        return None;
    }
    Some((old_prefix, old_kind, old_ident, new_ident))
}

/// Parse the composed double-delta header
/// `impl OldTr for OldTy<sfx>` → `impl NewTr for NewTy<sfx>`, or `None`
/// for anything else (single-delta legs belong to `impl_single`; qualified
/// trait paths, `impl<…>` generics, and `dyn`/`!` heads decline).
fn parse_impl_double_delta(minus: &str, plus: &str) -> Option<(String, String, String, String)> {
    let old = minus.trim();
    let new = plus.trim();
    let old_rest = old
        .strip_prefix("impl ")
        .or_else(|| old.strip_prefix("impl\t"))?;
    let new_rest = new
        .strip_prefix("impl ")
        .or_else(|| new.strip_prefix("impl\t"))?;
    if old_rest.starts_with('<') || new_rest.starts_with('<') {
        return None;
    }
    let (old_tr, old_ty) = split_for(old_rest)?;
    let (new_tr, new_ty) = split_for(new_rest)?;
    if !is_bare_ident(old_tr) || !is_bare_ident(new_tr) {
        return None;
    }
    if old_tr == new_tr {
        return None;
    }
    let (oy, old_sfx) = split_self_ty(old_ty)?;
    let (ny, new_sfx) = split_self_ty(new_ty)?;
    if oy == ny || old_sfx != new_sfx {
        return None;
    }
    Some((
        old_tr.to_string(),
        new_tr.to_string(),
        oy.to_string(),
        ny.to_string(),
    ))
}

/// Split an `impl`-rest at the single `for` word: `Tr for Ty …` →
/// `Some(("Tr", "Ty …"))`. Same word-boundary discipline as
/// `impl_single` (its helper is private; `Format`/`before` never split).
fn split_for(rest: &str) -> Option<(&str, &str)> {
    let bytes = rest.as_bytes();
    let mut hits: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i + 3 <= bytes.len() {
        let boundary_before = i == 0 || {
            let c = bytes[i - 1] as char;
            !(c.is_ascii_alphanumeric() || c == '_')
        };
        let boundary_after = i + 3 >= bytes.len() || {
            let c = bytes[i + 3] as char;
            !(c.is_ascii_alphanumeric() || c == '_')
        };
        if rest[i..].starts_with("for") && boundary_before && boundary_after {
            hits.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    if hits.len() != 1 {
        return None;
    }
    let at = hits[0];
    Some((rest[..at].trim_end(), rest[at + 3..].trim_start()))
}

/// True for a single unqualified ident (no paths, no `!`, no generics).
fn is_bare_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
        && super::ident_len(s) == Some(s.len())
}

/// Split a self-ty side into (leading ident, byte suffix). The suffix
/// (generics tail, `where`, `{`) must compare byte-equal across the pair.
fn split_self_ty(s: &str) -> Option<(&str, &str)> {
    let len = super::ident_len(s)?;
    if len == 0 || len == s.len() {
        return None;
    }
    Some((&s[..len], &s[len..]))
}

/// D13 joint probe for the triple: the composed emission applied to a
/// `pre` clone must show each native plus line at its claimed site with
/// every other line of every pre-image file byte-identical.
fn probe_joint(
    pre: &HashMap<String, String>,
    emission: &str,
    pairs: &[(String, String, usize)],
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(emission) else {
        return false;
    };
    let mut files = pre.clone();
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    // The emission touches exactly one path (single-section slice).
    let path = &parsed.files[0].target_path;
    let Some(pre_text) = pre.get(path) else {
        return false;
    };
    let Some(post_text) = files.get(path) else {
        return false;
    };
    let pre_lines: Vec<&str> = pre_text.lines().collect();
    let post_lines: Vec<&str> = post_text.lines().collect();
    if pre_lines.len() != post_lines.len() {
        return false;
    }
    let mut claims: HashMap<usize, &str> = HashMap::new();
    for (_, plus, site) in pairs {
        claims.insert(*site, plus.trim());
    }
    for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        let line_no = idx + 1;
        if let Some(want) = claims.get(&line_no) {
            if post_line.trim() != *want {
                return false;
            }
        } else if pre_line != post_line {
            return false;
        }
    }
    // Every other pre-image file byte-identical (workspace follow must
    // not leak into untouched files).
    for (p, pre_file) in pre {
        if p == path {
            continue;
        }
        if files.get(p) != Some(pre_file) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const PRE: &str = "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\n";

    fn triple_section() -> String {
        section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n",
        )
    }

    fn fire(sections: &[String], pre: &HashMap<String, String>) -> Option<String> {
        let mut used = Vec::new();
        try_joint_impl_rename(sections, pre, &mut used).map(|(out, _, _)| out.join(""))
    }

    #[test]
    fn joint_impl_triple_fires_verified() {
        let pre = pre_map(&[("f.rs", PRE)]);
        let out = fire(&[triple_section()], &pre).expect("triple must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("- trait Printable\n"), "got: {out}");
        assert!(out.contains("+ trait Labeled\n"), "got: {out}");
        assert!(out.contains("- struct Old\n"), "got: {out}");
        assert!(out.contains("+ struct New\n"), "got: {out}");
        // One shared rule across both hunks, trait first.
        assert_eq!(out.matches("rule=r1\n").count(), 2, "got: {out}");
        assert!(
            out.find("- trait Printable").unwrap() < out.find("- struct Old").unwrap(),
            "got: {out}"
        );
        // Real-apply byte-identical.
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "trait Labeled {}\nstruct New;\nimpl Labeled for New {\n}\n"
        );
    }

    #[test]
    fn joint_impl_enum_and_union_carriers_fire() {
        for (kind, pre, minus, plus, body) in [
            (
                "enum",
                "trait Printable {}\nenum Old {\n    A,\n}\nimpl Printable for Old {\n}\n",
                "enum Old {",
                "enum New {",
                "@@ -1,6 +1,6 @@\n-trait Printable {}\n-enum Old {\n+trait Labeled {}\n+enum New {\n     A,\n }\n-impl Printable for Old {\n+impl Labeled for New {\n }\n",
            ),
            (
                "union",
                "trait Printable {}\nunion Old {\n    x: i32,\n}\nimpl Printable for Old {\n}\n",
                "union Old {",
                "union New {",
                "@@ -1,6 +1,6 @@\n-trait Printable {}\n-union Old {\n+trait Labeled {}\n+union New {\n     x: i32,\n }\n-impl Printable for Old {\n+impl Labeled for New {\n }\n",
            ),
        ] {
            let _ = (minus, plus);
            let native = section("f.rs", body);
            let pre = pre_map(&[("f.rs", pre)]);
            let out = fire(&[native], &pre).expect("{kind} carrier must fire");
            assert!(out.contains(&format!("- {kind} Old\n")), "got: {out}");
            let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
            let mut files = pre.clone();
            rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
            assert!(files["f.rs"].contains("impl Labeled for New {"), "got: {}", files["f.rs"]);
        }
    }

    #[test]
    fn joint_impl_pub_prefix_fires_concrete() {
        let pre = "pub trait Printable {}\npub struct Old;\nimpl Printable for Old {\n}\n";
        let native = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-pub trait Printable {}\n-pub struct Old;\n-impl Printable for Old {\n+pub trait Labeled {}\n+pub struct New;\n+impl Labeled for New {\n",
        );
        let pre = pre_map(&[("f.rs", pre)]);
        let out = fire(&[native], &pre).expect("pub triple must fire");
        assert!(out.contains("- pub trait Printable\n"), "got: {out}");
        assert!(out.contains("- pub struct Old\n"), "got: {out}");
        assert!(!out.contains(" V "), "never a hole, got: {out}");
    }

    #[test]
    fn joint_impl_veto_matrix() {
        let pre = pre_map(&[("f.rs", PRE)]);
        // Third rename in the section stays ordinary.
        let four = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n-fn helper() {}\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n+fn ren_helper() {}\n }\n",
        );
        assert!(fire(&[four], &pre).is_none());
        // Mismatched composition (impl names a third ident) declines.
        let mismatch = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for Other {\n",
        );
        assert!(fire(&[mismatch], &pre).is_none());
        // Qualified trait path declines.
        let qpre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl crate::Printable for Old {\n}\n",
        )]);
        let qualified = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl crate::Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl crate::Labeled for New {\n",
        );
        assert!(fire(&[qualified], &qpre).is_none());
        // Generic impl header declines.
        let gpre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl<T> Printable for Old {\n}\n",
        )]);
        let generic = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl<T> Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl<T> Labeled for New {\n",
        );
        assert!(fire(&[generic], &gpre).is_none());
        // Single-delta legs are not triples.
        let two = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-trait Printable {}\n-struct Old;\n+trait Labeled {}\n+struct New;\n",
        );
        assert!(fire(&[two], &pre).is_none());
        // Identical names decline.
        let same = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Printable {}\n+struct Old;\n+impl Printable for Old {\n",
        );
        assert!(fire(&[same], &pre).is_none());
    }

    #[test]
    fn joint_impl_twin_def_vetoes() {
        // Two `struct Old` definitions in the pre-image: the type rename
        // is no longer unique-definition, so the joint must decline.
        let twin_pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\nmod m {\n    pub struct Old;\n}\n",
        )]);
        let native = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n",
        );
        assert!(fire(&[native], &twin_pre).is_none());
    }

    #[test]
    fn joint_impl_misaligned_pre_image_declines() {
        let pre = pre_map(&[("f.rs", PRE)]);
        // Claimed site shows a different line in pre.
        let native = section(
            "f.rs",
            "@@ -2,4 +2,4 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n",
        );
        assert!(fire(&[native], &pre).is_none());
    }

    #[test]
    fn joint_impl_off_diff_use_vetoes() {
        // An off-diff `Old` use elsewhere in the file: the self-type
        // follow rewrites it, failing the unchanged leg.
        let pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\nfn f(x: Old) -> Old {\n    x\n}\n",
        )]);
        assert!(fire(&[triple_section()], &pre).is_none());
    }

    #[test]
    fn joint_impl_parse_impl_double_delta_unit() {
        assert_eq!(
            parse_impl_double_delta("impl Printable for Old {", "impl Labeled for New {"),
            Some((
                "Printable".to_string(),
                "Labeled".to_string(),
                "Old".to_string(),
                "New".to_string()
            ))
        );
        // Suffix must be byte-equal.
        assert!(parse_impl_double_delta("impl A for B {", "impl C for D<T> {").is_none());
        // Single-delta legs decline here (owned by `impl_single`).
        assert!(parse_impl_double_delta("impl A for B {", "impl C for B {").is_none());
        assert!(parse_impl_double_delta("impl A for B {", "impl A for D {").is_none());
        assert!(parse_impl_double_delta("impl A for B {", "impl A for B {").is_none());
        // `Format` never splits a `for` word.
        assert!(parse_impl_double_delta("impl Format for B {", "impl Label for D {").is_some());
        assert!(parse_impl_double_delta("impl A for Before {", "impl C for D {").is_some());
    }
}
