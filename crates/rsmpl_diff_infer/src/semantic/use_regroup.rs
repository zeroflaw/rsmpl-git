//! `use`-regroup merge raising (WI-14 slice 2, SPEC 14.9).
//!
//! A minus set of `use` lines plus one `use` line whose tree is exactly
//! their merge (same paths, grouped at the longest common prefix)
//! raises to a use-path template pair — never a bare `- I` cut:
//!
//! ```diff
//! diff --git f.rs
//! @@ -1,2 @@ rule=r1
//! - use a::b::X;
//! - use a::b::Y;
//! + use a::b::{X, Y};
//! ```
//!
//! Merge-only, order-preserving, plain-`Name` members, uniform
//! visibility, no attributes. Anything else (splits, `as`/glob/nested
//! members, mixed visibility, attribute lines, order changes,
//! duplicate tails, single-minus) vetoes to ordinary. The emission is
//! directive-free (diet terse: `rule=` alone marks a semantic hunk,
//! SPEC 7.1) under the precision guard (`semantic <= ordinary + 16` —
//! a regroup never wins strict T375 since the `rule=` tag costs bytes
//! over the ordinary header; the D13 scratch-apply proof replaces bytes
//! per D3). Totality is the exactly-one-set occurrence proof plus the
//! D13 byte-compare probe; without a pre-image the shape stays ordinary
//! (a line-deleting raise without proof is data-loss risk, so the
//! section-visible legacy allow does not apply here).

use std::collections::HashMap;

use super::PRECISION_GUARD;

/// Unbalanced-hunk regroup entry: N≥2 `use` minus lines plus exactly
/// one plus line. Returns the hunk body plus the detector name (the
/// caller splices the body after the shared section prelude). Every
/// failure releases its reserved rule name.
pub(crate) fn try_use_regroup_hunk(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    hunk: &str,
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if pluses.len() != 1 || minuses.len() < 2 {
        return None;
    }
    let minus_raw = minuses.join("\n");
    let table = rsmpl_pattern::MetavarTable::from_directives(&[]).expect("empty directives parse");
    match rsmpl_pattern::classify_use_regroup(&minus_raw, pluses[0], &table) {
        // Not a regroup shape, or `use`-led but law-breaking: either
        // way wholly ordinary (the existing `use` gates own malformed
        // spellings from the authored side; here they simply decline).
        Ok(Some(_)) => {}
        Ok(None) | Err(_) => return None,
    }
    let pre = file_old?;
    // Exactly-one-set occurrence proof: every minus line occurs exactly
    // once as a full trimmed line in the pre-image, the plus grouped
    // line nowhere. A second identical set would merge ambiguously, so
    // it vetoes instead of guessing.
    let pre_lines: Vec<&str> = pre.lines().collect();
    for m in minuses {
        let t = m.trim();
        if pre_lines.iter().filter(|l| l.trim() == t).count() != 1 {
            return None;
        }
    }
    if pre_lines.iter().any(|l| l.trim() == pluses[0].trim()) {
        return None;
    }
    // Changed-line cover for the ranged emission: exactly the minus
    // lines, 1-based, contiguous or not (the range spans them).
    let changed = crate::totality::changed_old_lines(hunk);
    if changed.len() != minuses.len() || changed.contains(&0) {
        return None;
    }
    let min = changed.iter().min().copied().unwrap_or(0);
    let max = changed.iter().max().copied().unwrap_or(0);
    if min < 1 || max < min {
        return None;
    }
    // Adjacent-only: the engine merges at the first member line, which
    // reproduces the native post-tree exactly when the minus lines are
    // contiguous (a context line between them moves the native plus to
    // the last-minus position — the D13 probe would veto, but the
    // explicit gate names the reason instead of a bare probe miss).
    // Non-adjacent merges stay ordinary.
    if max - min + 1 != minuses.len() {
        return None;
    }
    let checkpoint = used_rules.len();
    let rule = super::unique_rule(used_rules, "use_regroup");
    let mut body = format!("@@ -{min},{} @@ rule={rule}\n", max - min + 1);
    for m in minuses {
        body.push_str(&format!("- {m}\n"));
    }
    body.push_str(&format!("+ {}\n", pluses[0]));
    // Precision guard on the hunk body (same posture as the single-pair
    // precision shapes: `body <= hunk + 16`).
    if body.len() > hunk.len() + PRECISION_GUARD {
        used_rules.truncate(checkpoint);
        return None;
    }
    // D13 joint probe: the ranged emission must apply with exactly one
    // match and reproduce byte-identically the native hunk's post-tree.
    if !probe_regroup_reproduces(path, pre, hunk, &body) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // The per-hunk caller splices the returned body after the shared
    // section prelude, so the body carries no file header (same shape
    // as the balanced pair bodies).
    Some((body, "use_regroup"))
}

/// D13 scratch-apply probe for regroup (T361 doctrine): the ranged
/// emission applies to the pre-image with exactly one match and yields
/// byte-identically the post-tree of applying the native hunk.
fn probe_regroup_reproduces(path: &str, pre: &str, native_hunk: &str, body: &str) -> bool {
    let native_section = format!(
        "{}\n--- a/{path}\n+++ b/{path}\n{native_hunk}",
        crate::git_header(path)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&native_section) else {
        return false;
    };
    let mut expected = HashMap::new();
    expected.insert(path.to_string(), pre.to_string());
    if rsmpl_engine::apply_patch_to_tree(&mut expected, &parsed).is_err() {
        return false;
    }
    let probe_section = format!("{}\n{body}", crate::git_header(path));
    let Ok(parsed2) = rsmpl_patch::parse_patch(&probe_section) else {
        return false;
    };
    let mut got = HashMap::new();
    got.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut got, &parsed2) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == 1
        && got.get(path) == expected.get(path)
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
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

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    fn native_post(section: &str, pre: &HashMap<String, String>) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(section).expect("native must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("native must apply");
        files
    }

    #[test]
    fn regroup_two_to_one_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,2 @@\n-use a::b::X;\n-use a::b::Y;\n+use a::b::{X, Y};\n fn m() {}\n",
        );
        let pre = pre_map(&[("f.rs", "use a::b::X;\nuse a::b::Y;\nfn m() {}\n")]);
        let out =
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).expect("must fire");
        validated_live(&out, &pre);
        assert!(out.contains("@@ -1,2 @@ rule=r1\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "directive-free, got: {out}");
        assert!(!out.contains("# sweep"), "got: {out}");
        assert!(!out.contains("- I"), "never an item cut, got: {out}");
        assert!(out.contains("- use a::b::X;\n"), "got: {out}");
        assert!(out.contains("- use a::b::Y;\n"), "got: {out}");
        assert!(out.contains("+ use a::b::{X, Y};\n"), "got: {out}");
        // Byte-identical to the native post-tree.
        let files = pre.clone();
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = files;
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files, native_post(&raw, &pre));
    }

    #[test]
    fn regroup_three_to_one_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,2 @@\n-use a::b::X;\n-use a::b::Y;\n-use a::b::Z;\n+use a::b::{X, Y, Z};\n fn m() {}\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "use a::b::X;\nuse a::b::Y;\nuse a::b::Z;\nfn m() {}\n",
        )]);
        let out =
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).expect("must fire");
        validated_live(&out, &pre);
        assert!(out.contains("+ use a::b::{X, Y, Z};\n"), "got: {out}");
        let mut files = pre.clone();
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files, native_post(&raw, &pre));
    }

    #[test]
    fn regroup_nonadjacent_stays_ordinary() {
        // Minus lines separated by an untouched line: the native plus
        // lands at the last-minus position while the engine merges at
        // the first, so the raise stays ordinary (adjacency gate).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,3 @@\n-use a::b::X;\n use a::b::Keep;\n-use a::b::Y;\n+use a::b::{X, Y};\n fn m() {}\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "use a::b::X;\nuse a::b::Keep;\nuse a::b::Y;\nfn m() {}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn regroup_veto_matrix() {
        // Order swap: the group must list tails in minus order.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,2 @@\n-use a::b::X;\n-use a::b::Y;\n+use a::b::{Y, X};\n fn m() {}\n",
        );
        let pre = pre_map(&[("f.rs", "use a::b::X;\nuse a::b::Y;\nfn m() {}\n")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
        // `as` member: unstatable tail.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,2 @@\n-use a::b::X;\n-use a::b::Y;\n+use a::b::{X, Y as Z};\n fn m() {}\n",
        );
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
        // Mixed visibility: merge must never change visibility.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,2 @@\n-use a::b::X;\n-pub use a::b::Y;\n+use a::b::{X, Y};\n fn m() {}\n",
        );
        let pre_mixed = pre_map(&[("f.rs", "use a::b::X;\npub use a::b::Y;\nfn m() {}\n")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_mixed).is_none());
        // Deeper member: not at the grouping depth.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,2 @@\n-use a::b::X;\n-use a::b::Y::Deep;\n+use a::b::{X, Y::Deep};\n fn m() {}\n",
        );
        let pre_deep = pre_map(&[("f.rs", "use a::b::X;\nuse a::b::Y::Deep;\nfn m() {}\n")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_deep).is_none());
    }

    #[test]
    fn regroup_duplicate_set_vetoes() {
        // A second identical minus set in the pre-image: the merge is
        // ambiguous, so the raise vetoes to ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,2 @@\n-use a::b::X;\n-use a::b::Y;\n+use a::b::{X, Y};\n fn m() {}\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "use a::b::X;\nuse a::b::Y;\nuse a::b::X;\nuse a::b::Y;\nfn m() {}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn regroup_no_pre_image_stays_ordinary() {
        // A line-deleting raise without proof is data-loss risk: no
        // pre-image means no raise (stricter than non-deleting shapes).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,2 @@\n-use a::b::X;\n-use a::b::Y;\n+use a::b::{X, Y};\n fn m() {}\n",
        );
        assert!(super::super::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn regroup_pub_uniform_fires() {
        // Uniform `pub` merges keep visibility on the grouped line.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,2 @@\n-pub use a::b::X;\n-pub use a::b::Y;\n+pub use a::b::{X, Y};\n fn m() {}\n",
        );
        let pre = pre_map(&[("f.rs", "pub use a::b::X;\npub use a::b::Y;\nfn m() {}\n")]);
        let out =
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).expect("must fire");
        validated_live(&out, &pre);
        assert!(out.contains("+ pub use a::b::{X, Y};\n"), "got: {out}");
        let mut files = pre.clone();
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files, native_post(&raw, &pre));
    }
}
