//! Trait+impl method rename sweep (WI-4b, SPEC D.4).
//!
//! N-site (N ≥ 2) method rename spanning at least one `trait` block
//! (declarations `fn m(...);` and default bodies) and at least one
//! `impl` block, all sharing one ident delta with per-site suffixes
//! preserved. The engine has matched this shape since T182 and the
//! WI-4a header filter selects each named block, so emission is one
//! hunk per enclosing block (verbatim header lines, shared rule name)
//! with `;` pairs on trait-decl groups and `{` pairs elsewhere —
//! each hunk clearing its own engine probe (`probe_block_hunk`).
//! Pure trait-side classes stay ordinary (no impl side to share the
//! intent); pure impl-side classes stay with the slot sweep.

use super::{item_emission_header, unique_rule};

/// N-site trait+impl method rename with a callee-only delta: every
/// pair is a single-line fn header differing in exactly the fn ident
/// (vis/qual/generics/params/return/where byte-equal per site,
/// heterogeneous across sites), `;`-terminated (trait declarations)
/// or `{`-terminated (definitions and trait defaults). Uniformly
/// indented sites emit one hunk per enclosing trait/`impl` block with
/// its verbatim header line (WI-4a: the engine header filter matches
/// only the named block); all share the rule name (one intent, K
/// hunks). Each hunk clears its own engine probe (twin headers and
/// untouched twins veto to ordinary).
pub(crate) fn try_trait_method_rename(
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
    let pre = file_old?;
    // WI-23: two-phase classification. Def pairs parse as fn headers
    // with a uniform ident delta; non-def pairs are candidate anchored
    // call sites gated cheaply here (ident-boundary share) with the
    // single-follow scratch probe authoritative below.
    let mut old_name: Option<String> = None;
    let mut new_name: Option<String> = None;
    let mut terms: Vec<Option<char>> = Vec::with_capacity(minuses.len());
    let mut is_def: Vec<bool> = Vec::with_capacity(minuses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        if let Some((o, n, term)) = trait_rename_pair(m, p) {
            if old_name.as_ref().is_some_and(|c| *c != o) {
                return None;
            }
            if new_name.as_ref().is_some_and(|c| *c != n) {
                return None;
            }
            old_name = Some(o);
            new_name = Some(n);
            terms.push(Some(term));
            is_def.push(true);
        } else {
            terms.push(None);
            is_def.push(false);
        }
    }
    let (old_name, new_name) = (old_name?, new_name?);
    if old_name == new_name {
        return None;
    }
    // Candidate calls must share the ident delta (ident-boundary,
    // never substring); the scratch probe below proves anchoring.
    for (i, (m, p)) in minuses.iter().zip(pluses.iter()).enumerate() {
        if is_def[i] {
            continue;
        }
        if m.trim() == p.trim() {
            return None;
        }
        if !contains_ident(m, &old_name) || !contains_ident(p, &new_name) {
            return None;
        }
    }
    let has_calls = is_def.iter().any(|d| !d);
    let all_indented = minuses.iter().all(|m| m.starts_with([' ', '\t']));
    let plus_indented = pluses.iter().all(|p| p.starts_with([' ', '\t']));
    if !all_indented || !plus_indented {
        return None;
    }
    // Side classification over def pairs only: every def line must sit
    // in a trait block or an impl block (macro bodies, strings, and
    // free fns veto the whole class). Call lines may sit anywhere —
    // the scratch probe proves each anchored site.
    let mut trait_lines: Vec<usize> = Vec::new();
    let mut impl_lines: Vec<usize> = Vec::new();
    for (i, &site) in changed.iter().enumerate() {
        if !is_def[i] {
            continue;
        }
        if in_trait_block(pre, site) {
            trait_lines.push(site);
        } else if in_impl_block(pre, site) {
            impl_lines.push(site);
        } else {
            return None;
        }
    }
    if trait_lines.is_empty() || impl_lines.is_empty() {
        return None;
    }
    let trait_groups = super::block_headers::group_sites_by_trait_block(file_old, &trait_lines)?;
    let impl_groups = super::block_headers::group_sites_by_impl_block(file_old, &impl_lines)?;
    // Pair index per site line (whole-section and per-hunk callers
    // pair lines and `changed` in the same order).
    let mut term_of: std::collections::HashMap<usize, char> = std::collections::HashMap::new();
    for (i, &site) in changed.iter().enumerate() {
        if let Some(t) = terms[i] {
            term_of.insert(site, t);
        }
    }
    // Document order across both halves.
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for g in &trait_groups {
        groups.push((g.header_line.clone(), g.sites.clone()));
    }
    for g in &impl_groups {
        groups.push((g.header_line.clone(), g.sites.clone()));
    }
    groups.sort_by_key(|(_, sites)| sites.first().copied().unwrap_or(usize::MAX));
    let rule = unique_rule(used_rules, &format!("rename_{old_name}_to_{new_name}"));
    // WI-3: the trait follow hunk alone suffices (engine claims impl
    // definitions as `rename` spans). Emit the document-first trait hunk
    // with `# follow=uses`; the scratch probe proves the impl defs.
    // Fallback: the K-hunk intent when the single-hunk probe fails.
    if let Some(first_trait) = trait_groups.first() {
        let mut term = None;
        for s in &first_trait.sites {
            let t = term_of.get(s)?;
            if term.is_some_and(|c| c != *t) {
                return None;
            }
            term = Some(*t);
        }
        let minus_fn = format!("V Q fn {old_name} G (...) W {}", term?);
        let plus_fn = format!("V Q fn {new_name} G (...) W {}", term?);
        if super::block_headers::probe_block_hunk(
            path,
            pre,
            &first_trait.header_line,
            &minus_fn,
            &plus_fn,
            first_trait.sites.len(),
        ) {
            let mut single = String::new();
            single.push_str(&item_emission_header(
                path,
                &rule,
                file_old,
                &first_trait.sites,
            ));
            single.push_str("# follow=uses\n");
            single.push_str(&super::block_headers::block_hunk_body(
                &first_trait.header_line,
                &minus_fn,
                &plus_fn,
            ));
            if scratch_single_reproduces(pre, path, &single, changed, minuses, pluses) {
                return Some(single);
            }
            // WI-23: calls have no block hunk; a failed call-inclusive
            // scratch must stay ordinary, never a def-only partial.
            if has_calls {
                return None;
            }
        }
    }
    let mut out = String::new();
    for (header, sites) in &groups {
        // Per-block uniform terminator (a block holds at most one
        // method of this name; mixed would be unstatable in one pair).
        let mut term = None;
        for s in sites {
            let t = term_of.get(s)?;
            if term.is_some_and(|c| c != *t) {
                return None;
            }
            term = Some(*t);
        }
        let minus_fn = format!("V Q fn {old_name} G (...) W {}", term?);
        let plus_fn = format!("V Q fn {new_name} G (...) W {}", term?);
        if !super::block_headers::probe_block_hunk(
            path,
            pre,
            header,
            &minus_fn,
            &plus_fn,
            sites.len(),
        ) {
            return None;
        }
        out.push_str(&item_emission_header(path, &rule, file_old, sites));
        out.push_str(&super::block_headers::block_hunk_body(
            header, &minus_fn, &plus_fn,
        ));
    }
    Some(out)
}
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

/// Scratch probe for the WI-3 single-hunk follow emission: applying the
/// single trait hunk must reproduce every changed line (trait + impl
/// defs) with all other lines byte-identical. Same discipline as
/// `assoc.rs::scratch_reproduces`.
fn scratch_single_reproduces(
    pre: &str,
    path: &str,
    section: &str,
    changed: &[usize],
    minuses: &[&str],
    pluses: &[&str],
) -> bool {
    let parsed = match rsmpl_patch::parse_patch(section) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    let post = &files[path];
    let pre_lines: Vec<&str> = pre.lines().collect();
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
        let _ = minuses[i];
    }
    for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        if changed_set.contains(&(idx + 1)) {
            continue;
        }
        if pre_line != post_line {
            return false;
        }
    }
    true
}

/// A `-`/`+` line pair as a method-rename header: both trimmed lines
/// end with `{` (definitions, trait defaults) or `;` (trait
/// declarations), both parse as fn-shaped, and they differ in exactly
/// the fn ident. Returns (old, new, terminator).
fn trait_rename_pair(old_line: &str, new_line: &str) -> Option<(String, String, char)> {
    let o = old_line.trim();
    let p = new_line.trim();
    if o.is_empty() || p.is_empty() {
        return None;
    }
    let term = match (o.chars().last(), p.chars().last()) {
        (Some(a), Some(b)) if a == b && (a == '{' || a == ';') => a,
        _ => return None,
    };
    if !is_method_shaped(o, term) || !is_method_shaped(p, term) {
        return None;
    }
    let (o_pre, o_ident, o_suf) = super::slot::split_fn_ident(o)?;
    let (p_pre, p_ident, p_suf) = super::slot::split_fn_ident(p)?;
    if o_pre != p_pre || o_suf != p_suf {
        return None;
    }
    if o_ident == p_ident || o_ident.is_empty() || p_ident.is_empty() {
        return None;
    }
    if o_suf.is_empty() {
        return None;
    }
    let tail = o_suf.trim_start();
    if !(tail.starts_with('(') || tail.starts_with('<')) {
        return None;
    }
    Some((o_ident, p_ident, term))
}

fn is_method_shaped(trimmed: &str, term: char) -> bool {
    if term == ';' {
        syn::parse_str::<syn::TraitItemFn>(trimmed).is_ok()
    } else {
        let probe = format!("{trimmed} }}");
        syn::parse_str::<syn::ItemFn>(&probe).is_ok()
            || syn::parse_str::<syn::ImplItemFn>(&probe).is_ok()
    }
}

/// True when 1-based `site` lies in a `trait` block (top-level and
/// inline-mod items).
fn in_trait_block(pre: &str, site: usize) -> bool {
    let Ok(file) = syn::parse_str::<syn::File>(pre) else {
        return false;
    };
    fn walk(items: &[syn::Item], site: usize) -> bool {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Trait(tr) => {
                    if tr.span().start().line <= site && site <= tr.span().end().line {
                        return true;
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        if walk(inner, site) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }
    walk(&file.items, site)
}

/// True when 1-based `site` lies in an `impl` block (top-level and
/// inline-mod items).
fn in_impl_block(pre: &str, site: usize) -> bool {
    let Ok(file) = syn::parse_str::<syn::File>(pre) else {
        return false;
    };
    fn walk(items: &[syn::Item], site: usize) -> bool {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Impl(imp) => {
                    if imp.span().start().line <= site && site <= imp.span().end().line {
                        return true;
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        if walk(inner, site) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }
    walk(&file.items, site)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRE: &str =
        "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n";

    fn run(minuses: &[&str], pluses: &[&str], pre: &str, changed: &[usize]) -> Option<String> {
        let mut used = Vec::new();
        try_trait_method_rename(minuses, pluses, "f.rs", Some(pre), changed, &mut used)
    }

    #[test]
    fn fires_over_trait_decl_and_impl_def() {
        let out = run(
            &["    fn old(&self);", "    fn old(&self) {"],
            &["    fn new(&self);", "    fn new(&self) {"],
            PRE,
            &[2, 6],
        )
        .expect("trait+impl rename must compress");
        // WI-3: single trait follow hunk owns the impl definition.
        assert_eq!(out.matches("rule=r1").count(), 1, "out:\n{out}");
        assert!(out.contains("trait T {"), "out:\n{out}");
        assert!(!out.contains("impl T for S {"), "out:\n{out}");
        assert!(out.contains("# follow=uses"), "out:\n{out}");
        assert!(out.contains("-     V Q fn old G (...) W ;"), "out:\n{out}");
        rsmpl_patch::parse_patch(&out).expect("emission must parse");
        // Round-trip: the impl definition rewrites via the follow.
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), PRE.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("fn new(&self);"),
            "got: {}",
            files["f.rs"]
        );
        assert!(
            files["f.rs"].contains("fn new(&self) {"),
            "got: {}",
            files["f.rs"]
        );
        assert!(!files["f.rs"].contains("fn old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn inherent_impl_declines_single_follow() {
        // Trait decl + inherent impl def are different items: the single
        // follow hunk must not claim the inherent def, so the class falls
        // back to the K-hunk intent (or ordinary if probes fail).
        let pre =
            "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl S {\n    fn old(&self) {}\n}\n";
        let out = run(
            &["    fn old(&self);", "    fn old(&self) {"],
            &["    fn new(&self);", "    fn new(&self) {"],
            pre,
            &[2, 6],
        )
        .expect("fallback intent still compresses");
        assert!(!out.contains("# follow=uses"), "got:\n{out}");
    }

    #[test]
    fn trait_only_stays_ordinary() {
        let pre = "trait T {\n    fn old(&self);\n}\ntrait U {\n    fn old(&self);\n}\n";
        assert!(
            run(
                &["    fn old(&self);", "    fn old(&self);"],
                &["    fn new(&self);", "    fn new(&self);"],
                pre,
                &[2, 5],
            )
            .is_none(),
            "decl-only classes stay ordinary by design"
        );
    }

    #[test]
    fn impl_only_stays_with_slot() {
        let pre = "struct A;\nstruct B;\nimpl A {\n    fn old(&self) {}\n}\nimpl B {\n    fn old(&self) {}\n}\n";
        assert!(
            run(
                &["    fn old(&self) {", "    fn old(&self) {"],
                &["    fn new(&self) {", "    fn new(&self) {"],
                pre,
                &[4, 7],
            )
            .is_none(),
            "impl-only classes stay with the slot sweep"
        );
    }

    #[test]
    fn divergent_suffix_vetoes() {
        assert!(
            run(
                &["    fn old(&self);", "    fn old(&self, x: i32) {"],
                &["    fn new(&self);", "    fn new(&self, x: i64) {"],
                PRE,
                &[2, 6],
            )
            .is_none(),
            "per-site suffixes must be preserved within each pair"
        );
    }

    #[test]
    fn free_fn_site_vetoes() {
        let pre = "trait T {\n    fn old(&self);\n}\nfn old() {}\n";
        assert!(
            run(
                &["    fn old(&self);", "fn old() {"],
                &["    fn new(&self);", "fn new() {"],
                pre,
                &[2, 4],
            )
            .is_none(),
            "mixed indent / non-method sites never claim"
        );
    }

    #[test]
    fn divergent_delta_vetoes() {
        assert!(run(
            &["    fn old(&self);", "    fn old(&self) {"],
            &["    fn new(&self);", "    fn other(&self) {"],
            PRE,
            &[2, 6],
        )
        .is_none());
    }

    #[test]
    fn single_declines_by_design() {
        // WI-19b: N=1 needs both a trait and an impl side to share one
        // intent; a lone line is never this detector's shape (slot or
        // a future single-impl detector owns it).
        assert!(run(&["    fn old(&self);"], &["    fn new(&self);"], PRE, &[2],).is_none());
        assert!(run(
            &["    fn old(&self) {"],
            &["    fn new(&self) {"],
            PRE,
            &[6],
        )
        .is_none());
    }

    #[test]
    fn untouched_twin_vetoes() {
        // Two identical trait blocks, only one renamed: the per-block
        // probe counts both, vetoing the emission.
        let twins = "trait T {\n    fn old(&self);\n}\ntrait T {\n    fn old(&self);\n}\nstruct S;\nimpl S {\n    fn old(&self) {}\n}\n";
        assert!(
            run(
                &["    fn old(&self);", "    fn old(&self) {"],
                &["    fn new(&self);", "    fn new(&self) {"],
                twins,
                &[2, 9],
            )
            .is_none(),
            "untouched twin vetoes via the per-hunk probe"
        );
    }
    #[test]
    fn call_inclusive_compresses_to_single_follow() {
        // WI-23: trait decl + impl def + anchored call compress to the
        // single trait follow hunk iff the scratch probe reproduces all
        // three lines.
        let pre = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f(s: S) {\n    s.old();\n}\n";
        let out = run(
            &["    fn old(&self);", "    fn old(&self) {", "    s.old();"],
            &["    fn new(&self);", "    fn new(&self) {", "    s.new();"],
            pre,
            &[2, 6, 10],
        )
        .expect("defs+calls must compress to one follow hunk");
        assert_eq!(out.matches("rule=").count(), 1, "out:\n{out}");
        assert!(out.contains("# follow=uses"), "out:\n{out}");
        assert!(!out.contains("impl T for S {"), "out:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("fn new(&self);"),
            "got: {}",
            files["f.rs"]
        );
        assert!(
            files["f.rs"].contains("fn new(&self) {"),
            "got: {}",
            files["f.rs"]
        );
        assert!(files["f.rs"].contains("s.new();"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("old"), "got: {}", files["f.rs"]);
    }
    #[test]
    fn unanchored_call_declines_to_ordinary() {
        // Receiver of an unrelated type is `off-target` evidence, never
        // a rewrite: the scratch over the full set fails, so the class
        // stays ordinary (no def-only partial).
        let pre = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nstruct U;\nfn f(y: U) {\n    y.old();\n}\n";
        assert!(
            run(
                &["    fn old(&self);", "    fn old(&self) {", "    y.old();"],
                &["    fn new(&self);", "    fn new(&self) {", "    y.new();"],
                pre,
                &[2, 6, 11],
            )
            .is_none(),
            "unproven receivers never ride the follow"
        );
    }
    #[test]
    fn shadowed_call_declines_to_ordinary() {
        // Same-name inherent method shadows the call: engine skips it,
        // the full-set scratch fails, the section stays ordinary.
        let pre = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nstruct U;\nimpl U {\n    fn old(&self) {}\n}\nfn f(u: U) {\n    u.old();\n}\n";
        assert!(
            run(
                &["    fn old(&self);", "    fn old(&self) {", "    u.old();"],
                &["    fn new(&self);", "    fn new(&self) {", "    u.new();"],
                pre,
                &[2, 6, 14],
            )
            .is_none(),
            "shadowed receivers never ride the follow"
        );
    }
    #[test]
    fn s_assoc_call_compresses_to_single_follow() {
        let pre = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f() {\n    S::old();\n}\n";
        let out = run(
            &["    fn old(&self);", "    fn old(&self) {", "    S::old();"],
            &["    fn new(&self);", "    fn new(&self) {", "    S::new();"],
            pre,
            &[2, 6, 10],
        )
        .expect("S::old tails must ride the follow");
        assert_eq!(out.matches("rule=").count(), 1, "out:\n{out}");
        assert!(out.contains("# follow=uses"), "out:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("S::new();"),
            "got: {}",
            files["f.rs"]
        );
        assert!(!files["f.rs"].contains("old"), "got: {}", files["f.rs"]);
    }
    #[test]
    fn qself_call_compresses_to_single_follow() {
        let pre = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nfn f() {\n    <S as T>::old();\n}\n";
        let out = run(
            &[
                "    fn old(&self);",
                "    fn old(&self) {",
                "    <S as T>::old();",
            ],
            &[
                "    fn new(&self);",
                "    fn new(&self) {",
                "    <S as T>::new();",
            ],
            pre,
            &[2, 6, 10],
        )
        .expect("qself tails must ride the follow");
        assert_eq!(out.matches("rule=").count(), 1, "out:\n{out}");
        assert!(out.contains("# follow=uses"), "out:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("<S as T>::new();"),
            "got: {}",
            files["f.rs"]
        );
        assert!(!files["f.rs"].contains("old"), "got: {}", files["f.rs"]);
    }
    #[test]
    fn unrelated_assoc_tail_declines_to_ordinary() {
        let pre = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nstruct X;\nfn f() {\n    X::old();\n}\n";
        assert!(
            run(
                &["    fn old(&self);", "    fn old(&self) {", "    X::old();"],
                &["    fn new(&self);", "    fn new(&self) {", "    X::new();"],
                pre,
                &[2, 6, 12],
            )
            .is_none(),
            "unrelated X::old tails never ride the follow"
        );
    }
    #[test]
    fn self_assoc_tail_compresses_to_single_follow() {
        let pre = "trait T {\n    fn old(&self);\n    fn run(&self) {\n        Self::old();\n    }\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n";
        let out = run(
            &[
                "    fn old(&self);",
                "    fn old(&self) {",
                "        Self::old();",
            ],
            &[
                "    fn new(&self);",
                "    fn new(&self) {",
                "        Self::new();",
            ],
            pre,
            &[2, 9, 4],
        )
        .expect("Self::old tails must ride the follow");
        assert_eq!(out.matches("rule=").count(), 1, "out:\n{out}");
        assert!(out.contains("# follow=uses"), "out:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("Self::new();"),
            "got: {}",
            files["f.rs"]
        );
        assert!(
            !files["f.rs"].contains("Self::old"),
            "got: {}",
            files["f.rs"]
        );
    }
}
