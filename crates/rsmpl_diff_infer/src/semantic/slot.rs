//! N-site heterogeneous fn/method rename slot sweep (SPEC 14).
//!
//! Split from `mod.rs` under the 2000-line gate (D5): pure move,
//! no behavior change.

use super::{ident_len, is_ident_char, item_emission_header, unique_rule};

/// N-site (N ≥ 2) heterogeneous fn/method rename with a
/// callee-only delta: every pair is a single-line `{`-terminated
/// fn header differing in exactly the fn ident (vis/qual/generics/
/// params/return/where byte-equal per site, heterogeneous across
/// sites). Column-0 sites emit the free-fn slot sweep (T329 shape);
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
    if !crate::totality::slot_holds(file_old, &old_name, minuses, all_indented, changed) {
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
/// lines end with `{`, both parse as fn-shaped (`ItemFn` or
/// `ImplItemFn`), and they differ in exactly the fn ident.
fn fn_rename_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o = old_line.trim();
    let p = new_line.trim();
    if o.is_empty() || p.is_empty() {
        return None;
    }
    if !o.ends_with('{') || !p.ends_with('{') {
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
    let probe = format!("{trimmed} }}");
    syn::parse_str::<syn::ItemFn>(&probe).is_ok()
        || syn::parse_str::<syn::ImplItemFn>(&probe).is_ok()
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
}
