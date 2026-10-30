//! Tree-aware canonical form (SPEC D.5, WI-1a).
//!
//! [`canonicalize_with_tree`] compares patches by what they rewrite, not
//! how they cite it: each `allow == many` hunk is matched against the
//! pre-image tree and re-emitted under the D.5 range rule (tight range
//! when its sites fit in 64 lines, else the `# sweep` dummy), so an
//! authored sweep and an inferred ranged hunk over the same sites share
//! one canonical form. Pure [`rsmpl_patch::canonicalize`] stays the
//! spelling-preserving comparator (fixed-point pins); this is the
//! meaning-preserving comparator for the cycle harness.
//!
//! Lives here (not in `rsmpl_patch`) because matching needs the engine
//! and the engine already depends on the patch crate — the reverse
//! dependency would be a cycle (PLAN.md documents the deviation).

use std::collections::HashMap;

use rsmpl_patch::{canonicalize, canonicalize_with_spans, parse_patch, ParseError};

/// Canonicalize `patch_text` against the pre-image tree `pre`
/// (repo-relative path → file text).
///
/// Site spans come from a real engine apply on a clone of `pre`, so the
/// normalization reflects what the patch actually matches. Missing
/// targets seed as empty (new-file leniency, mirroring the `applyrs`
/// CLI); rename sections seed the new side from the old side's content
/// (hunks cite old-side coordinates, SPEC T377). Any apply failure
/// (zero-match, overlap, unparseable target) falls back to pure
/// [`canonicalize`] — comparison never hard-fails on a divergent leg;
/// the harness tree legs still pin real breakage.
pub fn canonicalize_with_tree(
    patch_text: &str,
    pre: &HashMap<String, String>,
) -> Result<String, ParseError> {
    let patch = parse_patch(patch_text)?;
    let mut files: HashMap<String, String> = pre.clone();
    for sec in &patch.files {
        if files.contains_key(&sec.target_path) {
            continue;
        }
        if sec.is_rename() {
            let old_rel = rsmpl_patch::stripped_path(&sec.old_path);
            if let Some(old_text) = pre.get(&old_rel).cloned() {
                files.insert(sec.target_path.clone(), old_text);
                continue;
            }
        }
        files.insert(sec.target_path.clone(), String::new());
    }
    let report = match crate::apply_patch_to_tree(&mut files, &patch) {
        Ok(r) => r,
        Err(_) => return canonicalize(patch_text),
    };
    let mut spans: HashMap<(String, String), Vec<(u32, u32)>> = HashMap::new();
    for entry in &report.rules {
        spans
            .entry((entry.file.clone(), entry.rule.clone()))
            .or_default()
            .extend(entry.spans.iter().copied());
    }
    canonicalize_with_spans(patch_text, &spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(files: &[(&str, &str)]) -> HashMap<String, String> {
        files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect()
    }

    const PRE: &str = "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n";

    fn ranged_patch() -> String {
        "diff --git src/lib.rs\n@@ -2,3 @@ rule=calls\n# allow=many\n- foo(E0)\n+ bar(E0)\n"
            .to_string()
    }

    fn sweep_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=calls\n# sweep\n- foo(E0)\n+ bar(E0)\n".to_string()
    }

    #[test]
    fn sweep_and_ranged_unify() {
        // Same rewrite, two locality spellings → one tree-aware form.
        let p = pre(&[("src/lib.rs", PRE)]);
        let a = canonicalize_with_tree(&sweep_patch(), &p).unwrap();
        let b = canonicalize_with_tree(&ranged_patch(), &p).unwrap();
        assert_eq!(a, b, "sweep:\n{a}\nranged:\n{b}");
        assert!(
            a.contains("@@ -2,3 @@ rule=r1\n# allow=many\n"),
            "tight range wins under 64 lines:\n{a}"
        );
    }

    #[test]
    fn pure_canonical_keeps_spellings_apart() {
        // Guard for the two-comparator contract: pure form preserves the
        // authored locality, tree-aware form unifies it.
        let pure_sweep = canonicalize(&sweep_patch()).unwrap();
        let pure_ranged = canonicalize(&ranged_patch()).unwrap();
        assert_ne!(pure_sweep, pure_ranged);
    }

    #[test]
    fn wide_spread_normalizes_to_sweep() {
        // 70 one-line sites span 70 lines > 64 → both spellings become
        // sweep. Both patches must cover all sites (a ranged hunk only
        // rewrites its cited range): the ranged twin cites the full
        // `@@ -2,70 @@` tight range here.
        let mut src = String::from("fn m() {\n");
        for i in 0..70 {
            src.push_str(&format!("    foo({i});\n"));
        }
        src.push_str("}\n");
        let wide_ranged =
            "diff --git src/lib.rs\n@@ -2,70 @@ rule=calls\n# allow=many\n- foo(E0)\n+ bar(E0)\n";
        let p = pre(&[("src/lib.rs", &src)]);
        let a = canonicalize_with_tree(&sweep_patch(), &p).unwrap();
        let b = canonicalize_with_tree(wide_ranged, &p).unwrap();
        assert_eq!(a, b);
        assert!(a.contains("# sweep"), "wide spread → sweep:\n{a}");
        assert!(a.contains("@@ -1,1 @@"), "wide spread → dummy:\n{a}");
    }

    #[test]
    fn zero_match_falls_back_to_pure_form() {
        // Nothing matches: comparison degrades to pure canonicalize,
        // never an error.
        let p = pre(&[("src/lib.rs", "fn m() {}\n")]);
        let got = canonicalize_with_tree(&sweep_patch(), &p).unwrap();
        assert_eq!(got, canonicalize(&sweep_patch()).unwrap());
    }

    #[test]
    fn allow_one_ranges_untouched() {
        // Single-site hunks keep their authored ranges — the T375
        // single-site equality is tree + fixed-point, not canonical.
        let text = "diff --git src/lib.rs\n@@ -2,1 @@ rule=one\n- foo(E0)\n+ bar(E0)\n";
        let p = pre(&[("src/lib.rs", PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            got.contains("@@ -2,1 @@ rule=r1\n"),
            "allow=one range must survive:\n{got}"
        );
    }

    #[test]
    fn multifile_keys_stay_per_file() {
        // Same rule name in two files normalizes each file's range
        // from that file's own sites only.
        let text = "diff --git src/a.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\ndiff --git src/b.rs\n@@ -2,2 @@ rule=s\n# allow=many\n- foo(E0)\n+ bar(E0)\n";
        let p = pre(&[
            ("src/a.rs", "fn a() {\n    foo(1);\n}\n"),
            ("src/b.rs", "fn b() {\n    foo(1);\n    foo(2);\n}\n"),
        ]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            got.contains("@@ -2,1 @@"),
            "a.rs tightens to its own site:\n{got}"
        );
    }
}
