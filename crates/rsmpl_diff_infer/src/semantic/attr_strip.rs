//! WI-6(c) rename + full attribute/doc strip joint (SPEC D.4).
//!
//! A native hunk deleting N-1 attribute/doc lines above a fn header
//! rename pair, with a single header plus line carrying no attributes
//! (`/// Adds one.` + `#[inline]` + `fn foo…{` → `fn bar…{`), raises as
//! the authored joint: the item-cited range (attrs included, via `syn`)
//! plus `# attrs=strip` plus the rename frame (`- fn foo(...) {` /
//! `+ fn bar(...) {` + `...` + `}`). Partial strips (plus side retains
//! attributes) decline — residue, never a partial claim.
//!
//! The whole-section pair chain never sees this shape (it requires
//! balance), so the arm lives on the per-hunk unbalanced path, beside
//! the use-regroup Nv1 branch. Ranged hunks cite the stripped item, so
//! same-name twins elsewhere are simply untouched (orphans evidence at
//! apply time, never a veto here); an attribute line mentioning the old
//! ident vetoes as `attr-mentions-old` (opaque attr interiors are never
//! classified).

use super::prov::Counted;
use super::{item_emission_header, unique_rule};

/// Rename + full attr-strip joint for one unbalanced hunk, or the
/// counted decline. `minuses` are the N-1 deleted attr/doc lines plus
/// the header minus; `pluses` is the single bare header plus.
pub(crate) fn try_attr_strip_joint(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if pluses.len() != 1 || minuses.len() < 2 || changed.len() != minuses.len() {
        return Counted::Miss;
    }
    // Consecutive changed lines ending at the header (hunk-atomic).
    for w in changed.windows(2) {
        if w[1] != w[0] + 1 {
            return Counted::Miss;
        }
    }
    let header_minus = minuses[minuses.len() - 1];
    let header_plus = pluses[0];
    // Leading lines must all be attribute/doc deletions.
    for attr in &minuses[..minuses.len() - 1] {
        if !is_attr_line(attr) {
            return Counted::Miss;
        }
    }
    // The plus header must carry no attributes (full strip only).
    if is_attr_line(header_plus) {
        return Counted::Miss;
    }
    // The header pair must be a column-0 fn pure rename (plain or
    // qualified — the `classify_fn_pure_rename` / `split_item` law,
    // shared with the visqual joint).
    let (prefix, old_name, new_name) = match fn_rename_parts(header_minus, header_plus) {
        Some(p) => p,
        None => return Counted::Miss,
    };
    // Opaque attr honesty: a stripped line mentioning the old ident is
    // an unclassifiable occurrence — veto, never silently drop it.
    for attr in &minuses[..minuses.len() - 1] {
        if contains_word(attr, &old_name) {
            return Counted::Veto("attr-mentions-old");
        }
    }
    let pre = match file_old {
        Some(p) => p,
        None => return Counted::Miss,
    };
    let pre_lines: Vec<&str> = pre.lines().collect();
    // Pre-image alignment: the hunk's minus block must match verbatim.
    for (i, &site) in changed.iter().enumerate() {
        let Some(pre_line) = pre_lines.get(site.saturating_sub(1)) else {
            return Counted::Miss;
        };
        if pre_line.trim_end() != minuses[i].trim_end() {
            return Counted::Miss;
        }
    }
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, &format!("rename_{old_name}_to_{new_name}"));
    let mut out = item_emission_header(path, &rule, Some(pre), changed);
    // Single-item joint: `allow` is meaningless (the cited item is the
    // only match by construction, verified by the probe below), and
    // carrying `# allow=many` changes the engine's reported span
    // granularity (per-line sites vs the item site), breaking
    // canonical equality with the authored diet. A sweep-scoped cover
    // (span >64) declines instead — never a file-wide strip.
    if out.contains("# sweep") {
        used_rules.truncate(checkpoint);
        return Counted::Miss;
    }
    out = out.replace("# allow=many\n", "");
    out.push_str("# attrs=strip\n");
    out.push_str(&format!("- {prefix}fn {old_name}(...) {{\n"));
    out.push_str(&format!("+ {prefix}fn {new_name}(...) {{\n"));
    out.push_str("      ...\n  }\n");
    if !scratch_reproduces(pre, path, &out, &pre_lines, changed, pluses) {
        used_rules.truncate(checkpoint);
        return Counted::Miss;
    }
    Counted::Fire(out)
}

/// Attribute/doc line (deleted-strip domain): `#[…]`, `///…`, `//!…`.
fn is_attr_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("#[") || t.starts_with("///") || t.starts_with("//!")
}

/// Column-0 fn pure-rename parts as `(prefix, old, new)`: qualified
/// headers via the visqual classifier, plain headers via `split_item`.
/// Indented (method) headers decline; `;` decls decline.
fn fn_rename_parts(minus: &str, plus: &str) -> Option<(String, String, String)> {
    if let Some(pure) = super::item_joint_visqual::classify_fn_pure_rename(minus, plus) {
        return Some((pure.prefix, pure.old, pure.new));
    }
    let (old_prefix, old_kind, old_ident, old_suffix) = super::split_item(minus.trim_end())?;
    let (new_prefix, new_kind, new_ident, new_suffix) = super::split_item(plus.trim_end())?;
    if old_kind != "fn"
        || new_kind != "fn"
        || old_prefix != new_prefix
        || old_suffix != new_suffix
        || old_ident == new_ident
    {
        return None;
    }
    let suffix = old_suffix.trim_start();
    if !suffix.starts_with('(') || !minus.trim_end().ends_with('{') {
        return None;
    }
    // Stable `->` returns / `where` clauses ride the byte-equal suffix
    // (zero evolving slots — the frame emission carries them verbatim
    // and the section probe verifies); the vis/qual joints own
    // *evolving* slots, never this path.
    Some((old_prefix, old_ident, new_ident))
}

fn contains_word(haystack: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    haystack
        .split(|c: char| c != '_' && !c.is_ascii_alphanumeric())
        .any(|w| w == word)
}

/// D13 unbalanced splice proof: the expected post is the pre-image
/// with the first changed line replaced by the plus line and the rest
/// of the changed block deleted; the emission applied to a pre clone
/// must equal it byte-identical.
fn scratch_reproduces(
    pre: &str,
    path: &str,
    section: &str,
    pre_lines: &[&str],
    changed: &[usize],
    pluses: &[&str],
) -> bool {
    if changed.is_empty() || pluses.len() != 1 {
        return false;
    }
    let Ok(parsed) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    let post = &files[path];
    let mut expected: Vec<&str> = Vec::with_capacity(pre_lines.len());
    for (idx, line) in pre_lines.iter().enumerate() {
        let no = idx + 1;
        if no == changed[0] {
            expected.push(pluses[0]);
        } else if changed.contains(&no) {
            continue;
        } else {
            expected.push(line);
        }
    }
    post.lines().collect::<Vec<&str>>() == expected
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    const PRE: &str = "/// Adds one.\n#[inline]\nfn foo(x: i32) -> i32 {\n    x + 1\n}\n";

    #[test]
    fn attr_strip_fires_and_round_trips() {
        // WI-6(c) fixture-22 shape: full strip + rename raises as the
        // authored joint and rebuilds the post tree byte-identical.
        let minuses = ["/// Adds one.", "#[inline]", "fn foo(x: i32) -> i32 {"];
        let pluses = ["fn bar(x: i32) -> i32 {"];
        let mut used = Vec::new();
        let out = super::try_attr_strip_joint(
            &minuses,
            &pluses,
            "f.rs",
            Some(PRE),
            &[1, 2, 3],
            &mut used,
        )
        .unwrap_fire("22 shape must fire");
        assert!(out.contains("# attrs=strip\n"), "got:\n{out}");
        assert!(out.contains("- fn foo(...) {\n"), "got:\n{out}");
        assert!(out.contains("+ fn bar(...) {\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", PRE)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn bar(x: i32) -> i32 {\n    x + 1\n}\n");
    }

    #[test]
    fn attr_strip_vetoes_old_mention() {
        // A stripped attr naming the old ident vetoes (opaque interior).
        let pre =
            "#[deprecated(note = \"use foo instead\")]\nfn foo(x: i32) -> i32 {\n    x + 1\n}\n";
        let minuses = [
            "#[deprecated(note = \"use foo instead\")]",
            "fn foo(x: i32) -> i32 {",
        ];
        let pluses = ["fn bar(x: i32) -> i32 {"];
        let mut used = Vec::new();
        assert!(matches!(
            super::try_attr_strip_joint(&minuses, &pluses, "f.rs", Some(pre), &[1, 2], &mut used),
            super::super::prov::Counted::Veto("attr-mentions-old")
        ));
    }

    #[test]
    fn attr_strip_declines_matrix() {
        // Plus retains an attr (partial strip), non-attr leading line,
        // and non-consecutive sites all decline.
        let mut used = Vec::new();
        assert!(super::try_attr_strip_joint(
            &["/// Adds one.", "#[inline]", "fn foo(x: i32) -> i32 {"],
            &["#[inline]", "fn bar(x: i32) -> i32 {"],
            "f.rs",
            Some(PRE),
            &[1, 2, 3],
            &mut used,
        )
        .is_miss());
        assert!(super::try_attr_strip_joint(
            &["let x = 1;", "fn foo(x: i32) -> i32 {"],
            &["fn bar(x: i32) -> i32 {"],
            "f.rs",
            Some("let x = 1;\nfn foo(x: i32) -> i32 {\n    x + 1\n}\n"),
            &[1, 2],
            &mut used,
        )
        .is_miss());
        assert!(super::try_attr_strip_joint(
            &["/// Adds one.", "#[inline]", "fn foo(x: i32) -> i32 {"],
            &["fn bar(x: i32) -> i32 {"],
            "f.rs",
            Some(PRE),
            &[1, 2, 4],
            &mut used,
        )
        .is_miss());
    }

    #[test]
    fn attr_strip_pipeline_raises_joint() {
        // WI-6(c) pipeline: the 22-shape unbalanced hunk raises through
        // the per-hunk path (whole-section `whole=false` never reaches
        // the pair chain). Partial strip (plus retains an attr) stays
        // ordinary.
        let raw = section(
            "src/lib.rs",
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1 @@\n-/// Adds one.\n-#[inline]\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n",
        );
        let pre = pre_map(&[(
            "src/lib.rs",
            "/// Adds one.\n#[inline]\nfn foo(x: i32) -> i32 {\n    x + 1\n}\n",
        )]);
        let out = super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("rename+strip must raise the joint");
        assert!(out.contains("# attrs=strip\n"), "got:\n{out}");
        assert!(out.contains("- fn foo(...) {\n"), "got:\n{out}");
        assert!(
            !out.contains("# allow=many"),
            "single-item joint is diet:\n{out}"
        );
        let partial = section(
            "src/lib.rs",
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n-/// Adds one.\n-fn foo(x: i32) -> i32 {\n+/// Kept.\n+fn bar(x: i32) -> i32 {\n",
        );
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&partial), &pre)
                .is_none(),
            "partial strip stays ordinary"
        );
    }
}
