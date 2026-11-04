//! WI-6(a) column-0 one-line free-fn terse single (SPEC D.4).
//!
//! A lone `fn foo() {}` → `fn bar() {}` definition (whole item on one
//! line, rename-only, byte-identical body) compresses to the terse diet
//! pair (`- fn foo` / `+ fn bar`) under the precision guard. The VQ slot
//! sweep prices this shape at ~2x native by construction, so the slot
//! path can never own it; the terse diet (37B vs 42B ordinary) is the
//! only byte-honest spelling.
//!
//! Per-hunk-only by construction (the caller lives in the per-hunk
//! single path, never in partition/partial): the WI-3 parking trap — a
//! whole-section claim of the def single stranding a co-resident call
//! win ordinary — cannot fire. Multi-line headers still prefer
//! `try_item_rename` (the caller tries that first); this arm owns only
//! the one-line whole-item shape `try_item_rename` declines.
//!
//! Soundness: pre-image required; the pre-image line at the site must
//! equal the minus; any word-bound occurrence of the old ident outside
//! the def line vetoes as `unproven-use` (the raiser never invents a
//! call edit the diff did not show — a col0 rename whose calls the diff
//! does not update stays wholly ordinary); the D13 scratch-apply must
//! reproduce the plus line at the site with every other line
//! byte-identical (a same-name twin elsewhere is simply untouched by
//! the range-cited hunk, never a veto here).

use super::prov::Counted;
use super::{item_emission_header, unique_rule};

/// Column-0 one-line free-fn rename single, or the counted decline.
/// Exactly one pair; both trimmed lines must parse whole as
/// `syn::ItemFn` (brace-balanced by construction, string braces safe),
/// differ in exactly the fn ident with byte-equal surroundings, and be
/// column-0 (indented one-liners are methods, owned by the slot sweep).
pub(crate) fn try_col0_oneline_fn_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return Counted::Miss;
    }
    let (old_name, new_name) = match col0_oneline_pair(minuses[0], pluses[0]) {
        Some(n) => n,
        None => return Counted::Miss,
    };
    let pre = match file_old {
        Some(p) => p,
        None => return Counted::Miss,
    };
    // Pre-image alignment: the claimed line must be the diff's minus.
    let pre_lines: Vec<&str> = pre.lines().collect();
    let Some(pre_line) = pre_lines.get(changed[0].saturating_sub(1)) else {
        return Counted::Miss;
    };
    if pre_line.trim() != minuses[0].trim() {
        return Counted::Miss;
    }
    // Binder honesty for callees (D11): any other word-bound occurrence
    // of the old ident in the pre-image is a use the diff did not show
    // being updated — veto, never invent the edit.
    if has_off_def_use(pre, &old_name, changed[0]) {
        return Counted::Veto("unproven-use");
    }
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, &format!("rename_{old_name}_to_{new_name}"));
    let mut out = item_emission_header(path, &rule, Some(pre), changed);
    out.push_str(&format!("- fn {old_name}\n+ fn {new_name}\n"));
    if !scratch_reproduces(pre, path, &out, &pre_lines, changed, pluses) {
        used_rules.truncate(checkpoint);
        return Counted::Miss;
    }
    Counted::Fire(out)
}

/// A column-0 one-line fn pair as `(old_ident, new_ident)`, or `None`.
/// Both trimmed lines parse whole as `syn::ItemFn` (one-line whole
/// item); surrounding text outside the ident must be byte-equal;
/// bodies are therefore byte-identical (rename-only).
fn col0_oneline_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    if old_line.starts_with([' ', '\t']) || new_line.starts_with([' ', '\t']) {
        return None;
    }
    let o = old_line.trim();
    let p = new_line.trim();
    if o.is_empty() || p.is_empty() || !o.ends_with('}') || !p.ends_with('}') {
        return None;
    }
    if syn::parse_str::<syn::ItemFn>(o).is_err() || syn::parse_str::<syn::ItemFn>(p).is_err() {
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
    Some((o_ident, p_ident))
}

/// True when `name` occurs word-bound on any pre-image line outside the
/// def `site` (call sites, paths, comments — all conservative vetoes;
/// the raiser proves only what the diff shows).
fn has_off_def_use(pre: &str, name: &str, site: usize) -> bool {
    for (idx, line) in pre.lines().enumerate() {
        if idx + 1 == site {
            continue;
        }
        if contains_word(line, name) {
            return true;
        }
    }
    false
}

fn contains_word(haystack: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    haystack
        .split(|c: char| c != '_' && !c.is_ascii_alphanumeric())
        .any(|w| w == word)
}

/// D13 scratch proof: the emission applied to a pre-image clone must
/// show the native plus line at the claimed site with every other line
/// byte-identical.
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
        if changed_set.contains(&(idx + 1)) {
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
    use super::super::{infer_semantic_with_pre_images, unique_rule};
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const NO_USE_PRE: &str = "fn foo() {}\n\nfn calls() {\n    old_call();\n}\n";

    #[test]
    fn col0_oneline_fires_without_uses() {
        // WI-6(a) fixture-04 shape at detector level: one-line col0
        // rename with no other `foo` occurrence compresses to terse and
        // round-trips byte-identical.
        let mut used = Vec::new();
        let out = super::try_col0_oneline_fn_single(
            &["fn foo() {}"],
            &["fn bar() {}"],
            "f.rs",
            Some(NO_USE_PRE),
            &[1],
            &mut used,
        )
        .unwrap_fire("col0 one-line must fire");
        assert!(out.contains("- fn foo\n"), "got:\n{out}");
        assert!(out.contains("+ fn bar\n"), "got:\n{out}");
        assert!(!out.contains("follow"), "terse, never follow, got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", NO_USE_PRE)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("fn bar() {}"),
            "got: {}",
            files["f.rs"]
        );
        assert!(!files["f.rs"].contains("fn foo"), "got: {}", files["f.rs"]);
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn col0_oneline_vetoes_unproven_use() {
        // A call the diff does not update vetoes (never invent edits).
        let pre = "fn foo() {}\nfn main() {\n    foo();\n}\n";
        let mut used = Vec::new();
        assert!(matches!(
            super::try_col0_oneline_fn_single(
                &["fn foo() {}"],
                &["fn bar() {}"],
                "f.rs",
                Some(pre),
                &[1],
                &mut used,
            ),
            super::super::prov::Counted::Veto("unproven-use")
        ));
    }

    #[test]
    fn col0_oneline_pipeline_leaves_call_win() {
        // End to end over the 04 shape: the def raises terse AND the
        // co-resident call still raises (no parking — the arm is
        // per-hunk-only, invisible to partition/partial).
        let raw = "diff --git a/f.rs b/f.rs\n@@ -1 +1 @@\n-fn foo() {}\n+fn bar() {}\n@@ -4 +4 @@\n-    old_call();\n+    new_call();\n";
        let out =
            infer_semantic_with_pre_images(&[raw.to_string()], &pre_map(&[("f.rs", NO_USE_PRE)]))
                .expect("04 shape must raise");
        assert!(out.contains("- fn foo\n"), "def terse, got:\n{out}");
        assert!(
            out.contains("- old_call()") && out.contains("+ new_call()"),
            "call win kept, got:\n{out}"
        );
    }

    #[test]
    fn col0_oneline_declines_matrix() {
        // Indented one-liner (method), body delta, multi-line header,
        // and missing pre-image all decline.
        let mut used = Vec::new();
        assert!(super::try_col0_oneline_fn_single(
            &["    fn foo() {}"],
            &["    fn bar() {}"],
            "f.rs",
            Some(NO_USE_PRE),
            &[1],
            &mut used,
        )
        .is_miss());
        assert!(super::try_col0_oneline_fn_single(
            &["fn foo() {}"],
            &["fn bar() { 1 }"],
            "f.rs",
            Some(NO_USE_PRE),
            &[1],
            &mut used,
        )
        .is_miss());
        assert!(super::try_col0_oneline_fn_single(
            &["fn foo() {"],
            &["fn bar() {"],
            "f.rs",
            Some(NO_USE_PRE),
            &[1],
            &mut used,
        )
        .is_miss());
        assert!(super::try_col0_oneline_fn_single(
            &["fn foo() {}"],
            &["fn bar() {}"],
            "f.rs",
            None,
            &[1],
            &mut used,
        )
        .is_miss());
    }
}
