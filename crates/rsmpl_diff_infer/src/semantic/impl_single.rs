//! WI-19c self-ty `impl` header singles (SPEC D.4 partition family).
//!
//! Exactly one `impl Old …` → `impl New …` definition pair (no `for`
//! trait side), or exactly one trait-side pair (`impl OldTr for SameTy`
//! → `impl NewTr for SameTy`, self-ty byte-equal), compresses to the
//! authored bare-pair hunk (`- impl Old` / `+ impl New`) under the
//! precision guard. The double-delta line (`impl OldTr for OldTy` →
//! `impl NewTr for NewTy`, both sides change) is never a single — it
//! needs a joint trait+self-ty intent, owned by item-intent follow
//! work. Totality is the scratch-apply line-equality probe
//! (member-def-only discipline): the emitted hunk applied to an
//! in-memory pre-image clone must show exactly the native plus line at
//! the claimed site with every other line byte-identical. A same-name
//! header elsewhere in the file is `orphans/off-target` evidence at
//! apply time, never a veto — the whole-file sweep count would veto
//! every such single, so it is not consulted here.

use super::{ident_len, item_emission_header, unique_rule};

/// One `impl` header pair, or `None` when the lines are not exactly a
/// single-delta shape. Returns the renamed ident (self-ty or
/// trait-side). Both lines trim-compared; everything around the one
/// ident delta must be byte-equal.
pub(crate) fn parse_impl_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let old = old_line.trim();
    let new = new_line.trim();
    // Shared joint head discipline: `impl Foo`, `impl\tFoo`, and
    // `impl<T> Foo` admit; anything else declines.
    let old_rest = super::item_joint_impl::strip_impl_head(old)?;
    let new_rest = super::item_joint_impl::strip_impl_head(new)?;
    // Generic prefix (when present) must compare byte-equal across the
    // pair (proven, never invented); unclosed prefixes decline.
    let (old_gen, old_rest) = super::item_joint_impl::strip_impl_generics(old_rest)?;
    let (new_gen, new_rest) = super::item_joint_impl::strip_impl_generics(new_rest)?;
    if old_gen != new_gen {
        return None;
    }
    if contains_word(old_rest, "for") || contains_word(new_rest, "for") {
        parse_impl_trait_side(old_rest, new_rest)
    } else {
        parse_impl_self_ty(old_rest, new_rest)
    }
}

/// Self-ty shape: `Old …` → `New …` with the remainder byte-equal.
fn parse_impl_self_ty(old_rest: &str, new_rest: &str) -> Option<(String, String)> {
    let old_len = ident_len(old_rest)?;
    let new_len = ident_len(new_rest)?;
    let old_ident = &old_rest[..old_len];
    let new_ident = &new_rest[..new_len];
    if old_ident.is_empty() || new_ident.is_empty() || old_ident == new_ident {
        return None;
    }
    if old_rest[old_len..] != new_rest[new_len..] {
        return None;
    }
    // Suffix must be non-empty (there is always a `{`/generic tail —
    // a bare `impl Old` line never appears in a native diff).
    if old_rest[old_len..].is_empty() {
        return None;
    }
    Some((old_ident.to_string(), new_ident.to_string()))
}

/// Trait-side shape: `OldTr for SameTy …` → `NewTr for SameTy …` with
/// the self-ty and every other byte equal. The double-delta line
/// (self-ty also changes) declines here — never a single.
fn parse_impl_trait_side(old_rest: &str, new_rest: &str) -> Option<(String, String)> {
    let (old_tr, old_ty) = split_for(old_rest)?;
    let (new_tr, new_ty) = split_for(new_rest)?;
    if old_ty != new_ty {
        return None;
    }
    // Shared joint tail discipline: bare idents or qualified paths with
    // a byte-equal prefix and a renamed tail (`!`/generic tails decline
    // inside the helper — the follow path's last-segment law).
    let (old_ident, new_ident) = super::item_joint_impl::split_qualified_tail(old_tr, new_tr)?;
    if old_ident.is_empty() || new_ident.is_empty() || old_ident == new_ident {
        return None;
    }
    Some((old_ident, new_ident))
}

/// Split an `impl`-rest at the single `for` word: `Tr for Ty …` →
/// `Some(("Tr", "Ty …"))`. `None` when there is not exactly one `for`
/// word.
fn split_for(rest: &str) -> Option<(&str, &str)> {
    // Word-boundary split on `for` (the `contains_word` discipline:
    // `Format`/`before` never split).
    let mut hits: Vec<usize> = Vec::new();
    let bytes = rest.as_bytes();
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

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w == word)
}

/// WI-19c: single-delta `impl` header rename under the precision
/// guard (self-ty or trait-side, never both). Returns the
/// covering-range hunk (keeps `# allow=many`, the partition-canonical
/// spelling the 14 authored form carries) with the `item_rename`
/// detector name so grouping/provenance vocabulary is unchanged.
pub(crate) fn try_impl_header_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<(String, &'static str)> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return super::prov::Counted::Miss;
    }
    let Some((old, new)) = parse_impl_pair(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    // Pre-image alignment: the claimed line must be the diff's minus.
    let pre_lines: Vec<&str> = pre.lines().collect();
    let Some(pre_line) = pre_lines.get(changed[0].saturating_sub(1)) else {
        return super::prov::Counted::Miss;
    };
    if pre_line.trim() != minuses[0].trim() {
        return super::prov::Counted::Miss;
    }
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, &format!("rename_{old}_to_{new}"));
    let mut out = item_emission_header(path, &rule, Some(pre), changed);
    out.push_str(&format!("- impl {old}\n+ impl {new}\n"));
    if !scratch_reproduces(pre, path, &out, &pre_lines, changed, pluses) {
        used_rules.truncate(checkpoint);
        match crate::totality_counted::scratch_line_verdict(Some(pre), minuses, changed) {
            crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
                return super::prov::Counted::Totality { found, want };
            }
            crate::totality_counted::FamilyVerdict::OpaqueHit => {
                return super::prov::Counted::Opaque;
            }
            _ => return super::prov::Counted::Miss,
        }
    }
    super::prov::Counted::Fire((out, "item_rename"))
}

/// Apply `section` to `pre` in memory and require exact line equality:
/// the claimed post line trims to the native plus, every other line is
/// byte-identical to `pre` (an out-of-range same-name twin is simply
/// untouched — orphans evidence at apply time, never a veto here).
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
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn section_body(
        path: &str,
        file_old: Option<&str>,
        minus: &str,
        plus: &str,
        changed: &[usize],
        used: &mut Vec<String>,
    ) -> crate::semantic::prov::Counted<String> {
        match try_impl_header_single(&[minus], &[plus], path, file_old, changed, used) {
            crate::semantic::prov::Counted::Fire((s, _)) => crate::semantic::prov::Counted::Fire(s),
            crate::semantic::prov::Counted::Totality { found, want } => {
                crate::semantic::prov::Counted::Totality { found, want }
            }
            crate::semantic::prov::Counted::Opaque => crate::semantic::prov::Counted::Opaque,
            crate::semantic::prov::Counted::Veto(name) => {
                crate::semantic::prov::Counted::Veto(name)
            }
            crate::semantic::prov::Counted::Miss => crate::semantic::prov::Counted::Miss,
        }
    }

    const PRE: &str = "struct Old;\nstruct Foo;\nimpl Old {\n    fn f(&self) {}\n}\nimpl Printable for Foo {\n}\nimpl Printable for Old {\n}\n";

    #[test]
    fn impl_self_ty_single_fires_with_twin_as_orphan() {
        // `impl Old {` at line 3 renames; the self-ty twin at line 8
        // stays untouched (orphan at apply, never a veto here).
        let mut used = Vec::new();
        let out = section_body(
            "f.rs",
            Some(PRE),
            "impl Old {",
            "impl New {",
            &[3],
            &mut used,
        )
        .unwrap_fire("self-ty single must fire");
        assert!(out.contains("- impl Old\n"), "got:\n{out}");
        assert!(out.contains("+ impl New\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), PRE.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("impl New {"));
        // Twin line untouched by the ranged hunk.
        assert!(files["f.rs"].contains("impl Printable for Old {"));
    }

    #[test]
    fn impl_trait_side_single_fires_self_ty_equal_only() {
        // Trait-side single with an unchanged self-ty claims the line;
        // the double-delta line (self-ty also changes) is never a single.
        let mut used = Vec::new();
        let out = section_body(
            "f.rs",
            Some(PRE),
            "impl Printable for Foo {",
            "impl Labeled for Foo {",
            &[6],
            &mut used,
        )
        .unwrap_fire("trait-side single must fire");
        assert!(out.contains("- impl Printable\n"), "got:\n{out}");
        assert!(out.contains("+ impl Labeled\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), PRE.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("impl Labeled for Foo {"));
        // Double-delta line untouched by the ranged hunk.
        assert!(files["f.rs"].contains("impl Printable for Old {"));
    }

    #[test]
    fn impl_double_delta_and_identical_decline() {
        let mut used = Vec::new();
        // Double-delta: both sides change.
        assert!(section_body(
            "f.rs",
            Some(PRE),
            "impl Printable for Old {",
            "impl Labeled for New {",
            &[8],
            &mut used
        )
        .is_miss());
        // Identical names decline.
        assert!(section_body(
            "f.rs",
            Some(PRE),
            "impl Old {",
            "impl Old {",
            &[3],
            &mut used
        )
        .is_miss());
        assert!(used.is_empty());
    }

    #[test]
    fn impl_generic_self_ty_single_fires_verified() {
        // Generic prefix (byte-equal) composes: the bare-pair emission
        // rewrites the header, proven by the scratch re-apply.
        let pre = "struct Old;\nstruct Foo;\nimpl<T> Old<T> {\n    fn f(&self) {}\n}\n";
        let mut used = Vec::new();
        let out = section_body(
            "f.rs",
            Some(pre),
            "impl<T> Old<T> {",
            "impl<T> New<T> {",
            &[3],
            &mut used,
        )
        .unwrap_fire("generic self-ty single must fire");
        assert!(out.contains("- impl Old\n"), "got:\n{out}");
        assert!(out.contains("+ impl New\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "struct Old;\nstruct Foo;\nimpl<T> New<T> {\n    fn f(&self) {}\n}\n"
        );
    }

    #[test]
    fn impl_qualified_trait_side_single_fires_verified() {
        // Qualified trait tail (byte-equal prefix) composes like bare.
        let pre = "struct Foo;\nimpl crate::Printable for Foo {\n}\n";
        let mut used = Vec::new();
        let out = section_body(
            "f.rs",
            Some(pre),
            "impl crate::Printable for Foo {",
            "impl crate::Labeled for Foo {",
            &[2],
            &mut used,
        )
        .unwrap_fire("qualified trait-side single must fire");
        assert!(out.contains("- impl Printable\n"), "got:\n{out}");
        assert!(out.contains("+ impl Labeled\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "struct Foo;\nimpl crate::Labeled for Foo {\n}\n"
        );
    }

    #[test]
    fn impl_generic_trait_side_single_fires_verified() {
        // Generic prefix on a trait-side header composes, alone and with
        // a qualified trait tail.
        let pre = "struct Foo;\nimpl<T> Printable for Foo {\n}\n";
        let mut used = Vec::new();
        let out = section_body(
            "f.rs",
            Some(pre),
            "impl<T> Printable for Foo {",
            "impl<T> Labeled for Foo {",
            &[2],
            &mut used,
        )
        .unwrap_fire("generic trait-side single must fire");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "struct Foo;\nimpl<T> Labeled for Foo {\n}\n");
        let combo = "struct Foo;\nimpl<T> crate::Printable for Foo {\n}\n";
        let mut used = Vec::new();
        let out = section_body(
            "f.rs",
            Some(combo),
            "impl<T> crate::Printable for Foo {",
            "impl<T> crate::Labeled for Foo {",
            &[2],
            &mut used,
        )
        .unwrap_fire("generic+qualified trait-side single must fire");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), combo.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "struct Foo;\nimpl<T> crate::Labeled for Foo {\n}\n"
        );
    }

    #[test]
    fn impl_header_prefix_mismatch_declines() {
        // Generic prefix must compare byte-equal: never invent params.
        let gpre = "struct Old;\nimpl<T> Old<T> {\n}\n";
        let mut used = Vec::new();
        assert!(section_body(
            "f.rs",
            Some(gpre),
            "impl<T> Old<T> {",
            "impl<U> New<T> {",
            &[2],
            &mut used
        )
        .is_miss());
        // Qualified prefix must compare byte-equal.
        let qpre = "struct Foo;\nimpl crate::Printable for Foo {\n}\n";
        assert!(section_body(
            "f.rs",
            Some(qpre),
            "impl crate::Printable for Foo {",
            "impl other::Labeled for Foo {",
            &[2],
            &mut used
        )
        .is_miss());
        // Bare-vs-qualified shape mismatch declines.
        assert!(section_body(
            "f.rs",
            Some(PRE),
            "impl Printable for Foo {",
            "impl crate::Labeled for Foo {",
            &[6],
            &mut used
        )
        .is_miss());
        assert!(used.is_empty());
    }

    #[test]
    fn impl_misaligned_pre_image_declines() {
        // Claimed line must equal the pre-image line at the site.
        let mut used = Vec::new();
        assert!(section_body(
            "f.rs",
            Some(PRE),
            "impl Old {",
            "impl New {",
            &[4],
            &mut used
        )
        .is_miss());
        assert!(used.is_empty());
    }
}
