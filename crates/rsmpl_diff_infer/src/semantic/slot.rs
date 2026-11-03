//! N-site heterogeneous fn/method rename slot sweep (SPEC 14).
//!
//! Split from `mod.rs` under the 2000-line gate (D5): pure move,
//! no behavior change.

use super::{ident_len, is_ident_char, item_emission_header, unique_rule};

/// N-site (N ≥ 2) heterogeneous fn/method rename with a
/// callee-only delta: every pair is a single-line `{`-terminated
/// fn header differing in exactly the fn ident (vis/qual/generics/
/// params/return/where byte-equal per site, heterogeneous across
/// sites) — plus the WI-6 slice 1 N=1 one-line arm: a lone
/// `}`-terminated brace-balanced definition (byte-identical body,
/// rename-only) in an `impl` block compresses to the compact
/// filtered single under the same proof. Column-0 sites emit the free-fn slot sweep (T329 shape);
/// uniformly indented sites emit one hunk per enclosing `impl` block
/// with its verbatim header line (WI-4a: the engine header filter
/// matches only the named block); mixed indent stays passthrough.
pub(crate) fn try_fn_slot_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    // WI-19b: N=1 rename-only header singles ride the same totality
    // (`slot_holds` over the emitted minus header) under the precision
    // guard (callers). Disjoint from `try_item_rename`, which declines
    // vis-prefixed / generic / `->` / `where` headers. N=1 requires a
    // pre-image: the wide wildcard minus has no section-visible
    // evidence without an occurrence count (a same-shape twin
    // elsewhere would be silently rewritten).
    if minuses.is_empty() || minuses.len() != pluses.len() {
        return None;
    }
    if minuses.len() == 1 && file_old.is_none() {
        return None;
    }
    if changed.len() != minuses.len() {
        return None;
    }
    let mut old_name: Option<String> = None;
    let mut new_name: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n) = fn_rename_pair(m, p)?;
        if old_name.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_name.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        old_name = Some(o);
        new_name = Some(n);
    }
    let (old_name, new_name) = (old_name?, new_name?);
    if old_name == new_name {
        return None;
    }
    let all_indented = minuses.iter().all(|m| m.starts_with([' ', '\t']));
    let all_col0 = minuses
        .iter()
        .all(|m| !m.is_empty() && !m.starts_with([' ', '\t']));
    if !all_indented && !all_col0 {
        return None;
    }
    let plus_indented = pluses.iter().all(|p| p.starts_with([' ', '\t']));
    let plus_col0 = pluses
        .iter()
        .all(|p| !p.is_empty() && !p.starts_with([' ', '\t']));
    if all_indented != plus_indented || all_col0 != plus_col0 {
        return None;
    }
    if !crate::totality::slot_holds(file_old, &old_name, minuses, all_indented, changed)
        && !(minuses.len() == 1
            && all_indented
            && minuses[0].trim_end().ends_with('}')
            && crate::totality::no_opaque_hits(file_old, minuses, changed))
    {
        // WI-6 slice 1: a lone one-line (`}`) single with an untouched
        // same-name twin in another block fails the file-wide count,
        // but the WI-4a filter names the block so the twin never
        // matches — the per-emission scratch below proves the site
        // set; only macro/string interiors still veto. Every other
        // shape keeps the count (in particular a divergent class with
        // a co-renamed twin stays ordinary — the `{` path below never
        // takes this exception).
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_name}_to_{new_name}"));
    if all_indented {
        // WI-4a: the engine header filter matches only the named block,
        // so one advisory hunk can no longer sweep every block. Emit one
        // hunk per enclosing `impl` block with its verbatim header line;
        // all share the rule name (one intent, K hunks). Each hunk clears
        // its own engine probe (twin headers veto to ordinary).
        let pre = file_old?;
        let groups = super::block_headers::group_sites_by_impl_block(file_old, changed)?;
        // WI-6 slice 1: a lone one-line (`}`-terminated) method single
        // takes the compact inline-body emission — filter header plus
        // one pair, no frame, no directives (a lone site needs no
        // `allow`; the canonical range rule drops them anyway). The
        // filter is load-bearing (the filterless inline form never
        // matches); the scratch proves the exact site set, so no probe
        // is needed. Multi-line-header (`{`) singles keep the legacy
        // full-frame path below (the inline form cannot span a
        // multi-line body at a one-line range; N3 contraction owns
        // their equality).
        if minuses.len() == 1 && minuses[0].trim_end().ends_with('}') {
            let g = groups.first()?;
            if g.sites.len() != 1 || g.sites[0] != changed[0] {
                return None;
            }
            let minus_fx = format!("V Q fn {old_name} G (...) W {{ ... }}");
            let plus_fx = format!("V Q fn {new_name} G (...) W {{ ... }}");
            let mut out = format!(
                "{}\n@@ -{},1 @@ rule={rule}\n",
                crate::git_header(path),
                changed[0]
            );
            out.push_str(&format!(
                "  {}\n-     {minus_fx}\n+     {plus_fx}\n",
                g.header_line
            ));
            if !scratch_single_reproduces(pre, path, &out, changed, pluses) {
                return None;
            }
            return Some(out);
        }
        let minus_fn = format!("V Q fn {old_name} G (...) W {{");
        let plus_fn = format!("V Q fn {new_name} G (...) W {{");
        let mut out = String::new();
        for g in &groups {
            if !super::block_headers::probe_block_hunk(
                path,
                pre,
                &g.header_line,
                &minus_fn,
                &plus_fn,
                g.sites.len(),
            ) {
                return None;
            }
            out.push_str(&item_emission_header(path, &rule, file_old, &g.sites));
            out.push_str(&super::block_headers::block_hunk_body(
                &g.header_line,
                &minus_fn,
                &plus_fn,
            ));
        }
        return Some(out);
    }
    let mut hunk = item_emission_header(path, &rule, file_old, changed);
    hunk.push_str(&format!(
        "- V Q fn {old_name} G (...) W {{\n+ V Q fn {new_name} G (...) W {{\n      ...\n  }}\n"
    ));
    Some(hunk)
}

/// A `-`/`+` line pair as a rename-only fn header: both trimmed
/// lines end with `{` (multi-line header), or both end with `}` and
/// parse whole as a brace-balanced one-line definition (WI-6 slice 1:
/// balance by construction — no brace counting, string braces safe by
/// parser construction). Both parse as fn-shaped (`ItemFn` or
/// `ImplItemFn`), and they differ in exactly the fn ident; the shared
/// suffix comparison below keeps one-line bodies byte-identical
/// (rename-only — any body delta declines).
fn fn_rename_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o = old_line.trim();
    let p = new_line.trim();
    if o.is_empty() || p.is_empty() {
        return None;
    }
    let o_brace = o.ends_with('{');
    let p_brace = p.ends_with('{');
    if o_brace != p_brace {
        return None;
    }
    if !o_brace && (!o.ends_with('}') || !p.ends_with('}')) {
        return None;
    }
    if !is_fn_shaped(o) || !is_fn_shaped(p) {
        return None;
    }
    let (o_pre, o_ident, o_suf) = split_fn_ident(o)?;
    let (p_pre, p_ident, p_suf) = split_fn_ident(p)?;
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
    Some((o_ident, p_ident))
}

fn is_fn_shaped(trimmed: &str) -> bool {
    if trimmed.ends_with('}') {
        // WI-6 slice 1: brace-balanced one-line definition — whole-item
        // parse (literal-aware by construction).
        return syn::parse_str::<syn::ItemFn>(trimmed).is_ok()
            || syn::parse_str::<syn::ImplItemFn>(trimmed).is_ok();
    }
    let probe = format!("{trimmed} }}");
    syn::parse_str::<syn::ItemFn>(&probe).is_ok()
        || syn::parse_str::<syn::ImplItemFn>(&probe).is_ok()
}

/// Scratch totality for the WI-6 slice 1 compact single: applying the
/// exact emission to an in-memory pre-image clone must show the native
/// plus line at the claimed site with every other line byte-identical
/// (same discipline as `impl_single.rs::scratch_reproduces` — the
/// emission, not a probe device, is what gets proved).
fn scratch_single_reproduces(
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

/// Split `... fn Ident<...>(...)` into (prefix-before-ident, ident,
//suffix-after-ident). The `fn` keyword must be word-bounded.
pub(crate) fn split_fn_ident(text: &str) -> Option<(String, String, String)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..i + 2] == *b"fn"
            && (i == 0 || !is_ident_char(bytes[i - 1] as char))
            && (i + 2 < bytes.len() && bytes[i + 2].is_ascii_whitespace())
        {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let start = j;
            let len = ident_len(&text[j..])?;
            let end = start + len;
            return Some((
                text[..start].to_string(),
                text[start..end].to_string(),
                text[end..].to_string(),
            ));
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, unique_rule};
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn single_vis_rename_fires_and_round_trips() {
        // WI-19b: a lone vis-prefixed rename single (item-rename
        // declines vis) compresses under the precision guard.
        let pre = "pub fn foo(x: VeryLongTypeNameArgumentNumberOne) {\n    x\n}\n";
        let raw = "diff --git a/f.rs b/f.rs\n@@ -1,1 +1,1 @@\n-pub fn foo(x: VeryLongTypeNameArgumentNumberOne) {\n+pub fn bar(x: VeryLongTypeNameArgumentNumberOne) {\n";
        let out = infer_semantic_with_pre_images(&[raw.to_string()], &pre_map(&[("f.rs", pre)]))
            .expect("slot single must fire");
        assert!(out.contains("fn foo"), "got:\n{out}");
        assert!(out.contains("fn bar"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", pre)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("pub fn bar("),
            "got: {}",
            files["f.rs"]
        );
        assert!(!files["f.rs"].contains("pub fn foo("));
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn single_twin_vetoes() {
        // An untouched same-header twin vetoes via `slot_holds`.
        let pre = "pub fn foo(x: i32) {\n    x\n}\npub fn foo(y: i32) {\n    y\n}\n";
        let raw = "diff --git a/f.rs b/f.rs\n@@ -1,1 +1,1 @@\n-pub fn foo(x: i32) {\n+pub fn bar(x: i32) {\n";
        assert!(
            infer_semantic_with_pre_images(&[raw.to_string()], &pre_map(&[("f.rs", pre)]),)
                .is_none()
        );
    }

    #[test]
    fn single_plain_fn_stays_item_rename() {
        // Plain `fn foo` singles belong to `try_item_rename` (terse
        // diet), never the slot sweep: order pin.
        let pre = "fn foo() {\n    1\n}\n";
        let raw = "diff --git a/f.rs b/f.rs\n@@ -1,1 +1,1 @@\n-fn foo() {\n+fn bar() {\n";
        let out = infer_semantic_with_pre_images(&[raw.to_string()], &pre_map(&[("f.rs", pre)]))
            .expect("plain single must fire");
        assert!(out.contains("- fn foo\n"), "got:\n{out}");
        assert!(
            !out.contains("V Q"),
            "slot must not claim plain singles, got:\n{out}"
        );
    }

    const IMPL_TWIN_PRE: &str =
        "struct A;\nstruct B;\nimpl A {\n    fn old(&self) {}\n}\nimpl B {\n    fn old(&self) {}\n}\n";

    #[test]
    fn one_line_twin_fires_with_filter() {
        // WI-2 slice 2 (WI-6 slice 1): one-line def rename with an
        // untouched same-name twin in another impl block compresses to
        // the compact filtered single (the filter names the match
        // domain; the file-wide count would veto).
        let raw = "diff --git a/f.rs b/f.rs\n@@ -4,1 +4,1 @@\n-    fn old(&self) {}\n+    fn new(&self) {}\n";
        let out = infer_semantic_with_pre_images(
            &[raw.to_string()],
            &pre_map(&[("f.rs", IMPL_TWIN_PRE)]),
        )
        .expect("one-line twin single must fire");
        assert!(out.contains("impl A {"), "filter kept, got:\n{out}");
        assert!(
            out.contains("-     V Q fn old G (...) W { ... }\n"),
            "compact pair, got:\n{out}"
        );
        assert!(
            out.contains("+     V Q fn new G (...) W { ... }\n"),
            "compact pair, got:\n{out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", IMPL_TWIN_PRE)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("    fn new(&self) {}"),
            "got: {}",
            files["f.rs"]
        );
        // The impl-B twin is untouched.
        assert!(
            files["f.rs"].contains("impl B {\n    fn old(&self) {}"),
            "twin must survive, got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn one_line_body_delta_vetoes() {
        // Rename plus a body change is not rename-only (the shared
        // suffix comparison keeps one-line bodies byte-identical).
        let raw = "diff --git a/f.rs b/f.rs\n@@ -4,1 +4,1 @@\n-    fn old(&self) {}\n+    fn new(&self) { 1 }\n";
        assert!(infer_semantic_with_pre_images(
            &[raw.to_string()],
            &pre_map(&[("f.rs", IMPL_TWIN_PRE)]),
        )
        .is_none());
    }

    #[test]
    fn one_line_string_brace_fires() {
        // Braces inside string literals never unbalance the one-line
        // gate (whole-item parse is literal-aware by construction).
        let pre = "struct A;\nimpl A {\n    fn old(&self) { println!(\"{\") }\n}\n";
        let raw =
            "diff --git a/f.rs b/f.rs\n@@ -3,1 +3,1 @@\n-    fn old(&self) { println!(\"{\") }\n+    fn new(&self) { println!(\"{\") }\n";
        let out = infer_semantic_with_pre_images(&[raw.to_string()], &pre_map(&[("f.rs", pre)]))
            .expect("string-brace one-liner must fire");
        assert!(out.contains("impl A {"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", pre)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("fn new(&self)"),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn one_line_cross_header_twin_fires() {
        // The legal twin shape: an inherent method and a trait-impl
        // method sharing a name (different headers — this compiles,
        // unlike two same-name methods in one block, E0592). The
        // filter names the inherent block; the trait-impl twin and
        // the trait decl are outside the match domain.
        let pre = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl S {\n    fn old(&self) {}\n}\nimpl T for S {\n    fn old(&self) {}\n}\n";
        let raw =
            "diff --git a/f.rs b/f.rs\n@@ -6,1 +6,1 @@\n-    fn old(&self) {}\n+    fn new(&self) {}\n";
        let out = infer_semantic_with_pre_images(&[raw.to_string()], &pre_map(&[("f.rs", pre)]))
            .expect("cross-header twin single must fire");
        assert!(out.contains("impl S {"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", pre)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("impl S {\n    fn new(&self) {}"),
            "got: {}",
            files["f.rs"]
        );
        // Trait decl and trait-impl twin untouched.
        assert!(
            files["f.rs"].contains("impl T for S {\n    fn old(&self) {}"),
            "trait-impl twin must survive, got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn one_line_col0_fires_at_detector_level() {
        // A free-fn one-line def parses as a pair (the per-hunk guard
        // owns the pricing — the compact filterless form never
        // matches, so this stays ordinary through the pipeline).
        let pre = "fn old() { 1 }\nfn main() {}\n";
        let mut used = Vec::new();
        let out = super::try_fn_slot_sweep(
            &["fn old() { 1 }"],
            &["fn new() { 1 }"],
            "f.rs",
            Some(pre),
            &[1],
            &mut used,
        )
        .expect("col0 one-line pair must parse");
        assert!(out.contains("fn old"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", pre)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("fn new()"), "got: {}", files["f.rs"]);
    }
}
