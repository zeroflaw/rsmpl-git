//! WI-20 slice 1: rename+single-expr-body joint detector (D5 —
//! `fn_joint.rs` sits at the 2000-line gate; tests stay inline here per
//! WI-22).
//!
//! Fires when one balanced def hunk holds exactly a rename-only `fn`
//! header pair plus one non-call body line pair (both sides parse as
//! `syn::Expr`, differing text), and every other balanced pair in the
//! group is a callee-only call of old→new with at least one call.
//! Emission is the single `# sweep` + `# follow=uses` hunk (header `...`
//! pair + raw body pair lines); the engine applies the body pair
//! span-scoped inside the renamed body with a hard conflict otherwise.
//! Anything else in the group (slot evolution, attr delta, a second
//! body line, non-call shapes, unbalanced hunks) returns `None` so the
//! existing suppression path keeps the section wholly ordinary.

use std::collections::{HashMap, HashSet};

use super::fn_joint::{is_call_site_of, is_callee_only, slots_equal_except_name};
use super::fn_joint_types::{Fire, FlatPair, Joint};

/// Try the body-joint shape for one single-file group. `def_idx` is the
/// positional header-delta pair; `old`/`new` its names. Returns the
/// fireable payload, else `None` (the caller falls through to the
/// Joint-A/B path, which suppresses as today).
pub(crate) fn try_body_joint(
    flat: &[FlatPair],
    def_idx: usize,
    hunk_texts: &HashMap<(usize, usize), String>,
    has_unbalanced: bool,
    old: &str,
    new: &str,
) -> Option<Box<Fire>> {
    if has_unbalanced {
        return None;
    }
    // The def hunk holds exactly the header pair plus one body pair.
    let (def_sec, def_hunk) = (flat[def_idx].sec, flat[def_idx].hunk);
    let mut members: Vec<usize> = flat
        .iter()
        .enumerate()
        .filter(|(_, fp)| fp.sec == def_sec && fp.hunk == def_hunk)
        .map(|(i, _)| i)
        .collect();
    members.sort_unstable();
    if members.len() != 2 || !members.contains(&def_idx) {
        return None;
    }
    let body_idx = if members[0] == def_idx {
        members[1]
    } else {
        members[0]
    };
    // Rename-only header (any slot evolution stays on the suppress path).
    if !slots_equal_except_name(&flat[def_idx].minus, &flat[def_idx].plus) {
        return None;
    }
    // One whole-line expression pair, differing text, not itself a call
    // site of the rename.
    let (bm, bp) = (flat[body_idx].minus.trim(), flat[body_idx].plus.trim());
    if bm.is_empty() || bp.is_empty() || bm == bp {
        return None;
    }
    if syn::parse_str::<syn::Expr>(bm).is_err() || syn::parse_str::<syn::Expr>(bp).is_err() {
        return None;
    }
    if is_call_site_of(&flat[body_idx].minus, &flat[body_idx].plus, old, new) {
        return None;
    }
    // Every other balanced pair is a callee-only call (at least one).
    let mut calls: Vec<FlatPair> = Vec::new();
    for (i, fp) in flat.iter().enumerate() {
        if i == def_idx || i == body_idx {
            continue;
        }
        if !is_callee_only(&fp.minus, &fp.plus, old, new) {
            return None;
        }
        calls.push(fp.clone());
    }
    if calls.is_empty() {
        return None;
    }
    let mut claimed: Vec<FlatPair> = vec![flat[def_idx].clone(), flat[body_idx].clone()];
    claimed.extend(calls.iter().cloned());
    claimed.sort_by_key(|fp| (fp.sec, fp.hunk, fp.site));
    let mut claimed_secs: Vec<usize> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    for fp in &claimed {
        if seen.insert(fp.sec) {
            claimed_secs.push(fp.sec);
        }
    }
    let claimed_hunk_bytes: usize = {
        let mut seen_h: HashSet<(usize, usize)> = HashSet::new();
        let mut sum = 0usize;
        for fp in &claimed {
            if seen_h.insert((fp.sec, fp.hunk)) {
                sum += hunk_texts
                    .get(&(fp.sec, fp.hunk))
                    .map(|t| t.len())
                    .unwrap_or(0);
            }
        }
        sum
    };
    Some(Box::new(Fire {
        old: old.to_string(),
        new: new.to_string(),
        def_minus: flat[def_idx].minus.clone(),
        def_plus: flat[def_idx].plus.clone(),
        claims: claimed,
        claimed_secs,
        joint: Joint::Body {
            body_minus: bm.to_string(),
            body_plus: bp.to_string(),
        },
        leftover: HashMap::new(),
        claimed_hunk_bytes,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use super::super::item_intent::probe_reproduces;
    use super::super::item_intent::Pair;
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
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

    const PRE: &str =
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";

    fn body_sections() -> Vec<String> {
        vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: i32) -> i32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )]
    }

    #[test]
    fn body_joint_fires_verified_byte_identical() {
        let pre = pre_map(&[("src.rs", PRE)]);
        let out =
            infer_semantic_with_pre_images(&body_sections(), &pre).expect("body joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("# sweep"), "got:\n{out}");
        assert!(
            out.contains("- fn foo(x: i32) -> i32 { ... }"),
            "got:\n{out}"
        );
        assert!(
            out.contains("+ fn bar(x: i32) -> i32 { ... }"),
            "got:\n{out}"
        );
        assert!(out.contains("- x + 1"), "got:\n{out}");
        assert!(out.contains("+ x + 2"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    x + 2\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "body joint must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_beats_native_bytes_with_probe() {
        // D13: the composed emission reproduces the plus lines at the
        // claimed sites with every other line byte-identical.
        let pre = pre_map(&[("src.rs", PRE)]);
        let out =
            infer_semantic_with_pre_images(&body_sections(), &pre).expect("body joint must fire");
        let native: usize = body_sections().iter().map(|s| s.len()).sum();
        assert!(
            out.len() < native,
            "joint must beat native bytes ({} vs {native})",
            out.len()
        );
        let pairs = vec![
            Pair {
                sec: 0,
                path: "src.rs".to_string(),
                minus: "fn foo(x: i32) -> i32 {".to_string(),
                plus: "fn bar(x: i32) -> i32 {".to_string(),
                site: 1,
            },
            Pair {
                sec: 0,
                path: "src.rs".to_string(),
                minus: "x + 1".to_string(),
                plus: "x + 2".to_string(),
                site: 2,
            },
            Pair {
                sec: 0,
                path: "src.rs".to_string(),
                minus: "foo(1);".to_string(),
                plus: "bar(1);".to_string(),
                site: 5,
            },
            Pair {
                sec: 0,
                path: "src.rs".to_string(),
                minus: "foo(2);".to_string(),
                plus: "bar(2);".to_string(),
                site: 6,
            },
        ];
        assert!(
            probe_reproduces(&pre, &out, &pairs),
            "joint probe must reproduce, got:\n{out}"
        );
    }

    #[test]
    fn body_joint_no_calls_suppresses() {
        // Rename+body with no call uses is no joint (nothing for a
        // follow to own): the rename stays unclaimed (no follow hunk,
        // no rename rule) while the body line may raise as its own
        // general-expr single — never a rename-claiming sweep.
        let pre = pre_map(&[("src.rs", "fn foo(x: i32) -> i32 {\n    x + 1\n}\n")]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: i32) -> i32 {\n+    x + 2\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref()
                .is_none_or(|s| !s.contains("# follow=uses") && !s.contains("rename_foo_to_bar")),
            "no-call body shape must not claim the rename, got: {full:?}"
        );
    }

    #[test]
    fn body_joint_slot_evolution_suppresses() {
        // Rename+slot-evolution+body is two evolutions — stays ordinary.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: u32) -> i32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "slot+body double evolution must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn body_joint_two_body_lines_suppress() {
        // Two body-line deltas are not the single-expr slice — ordinary.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) -> i32 {\n-    let y = x + 1;\n-    y * 2\n+fn bar(x: i32) -> i32 {\n+    let y = x + 2;\n+    y * 3\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "two-body-line shape must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn body_joint_non_expr_body_suppresses() {
        // A `let`-statement body delta is not the general-expr slice.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = 1;\n    y\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    let y = 1;\n+fn bar(x: i32) -> i32 {\n+    let y = 2;\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "non-expr body shape must stay wholly ordinary, got: {full:?}"
        );
    }
}
