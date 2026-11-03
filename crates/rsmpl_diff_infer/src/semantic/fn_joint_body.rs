//! WI-20 slice 1 + WI-13 slices 1–4: rename+body-pair joint detector
//! (D5 — `fn_joint.rs` sits at the 2000-line gate; tests stay inline
//! here per WI-22).
//!
//! Fires when one balanced def hunk holds exactly a rename-only (WI-20 /
//! WI-13 slices 1–3) or Joint-A-shaped evolving (WI-13 slice 4: the
//! raiser's `header_fire_gates` — evolving ⊆ {vis, qual, params, ret}
//! with params or ret evolving, at most two evolve, generics/where
//! preserved) `fn` header pair plus one (WI-20 / WI-13 slice 1), two
//! (WI-13 slice 2), or three (WI-13 slice 3)
//! non-call body line pairs (each a `syn::Expr` or a plain same-binder
//! `let` with an init, differing text), and every other balanced pair
//! in the group is a callee-only call of old→new with at least one
//! call. Emission is the single `# sweep` + `# follow=uses` hunk
//! (header `...` pair + raw body pair lines); the engine applies the
//! header via `plan_sig_edits` (so a triple's slot evolution rides the
//! same splice as a Joint-A header) and each body pair span-scoped
//! inside the renamed body with a hard conflict otherwise. Anything
//! else in the group (non-Joint-A slot evolution, attr delta, a fourth
//! body line, non-call shapes, unbalanced hunks) returns `None`
//! so the existing suppression path keeps the section wholly ordinary.

use std::collections::{HashMap, HashSet};

use super::fn_joint::{is_call_site_of, is_callee_only};
use super::fn_joint_sig::{header_fire_gates, slots_equal_except_name};
use super::fn_joint_types::{Fire, FlatPair, Joint};

/// WI-13 slices 1–3: per-pair half of the joint gate (mirrors the
/// engine pattern's `body_pair_holds`; slices 2–3 call it once per pair).
/// Accepts one `syn::Expr` each side, or one plain same-binder `let`
/// (`syn::Stmt::Local`, both with `init`, no attributes, binder `pat`
/// token-equal).
fn body_pair_ok(bm: &str, bp: &str) -> bool {
    if syn::parse_str::<syn::Expr>(bm).is_ok() && syn::parse_str::<syn::Expr>(bp).is_ok() {
        return true;
    }
    let (lm, lp) = match (
        syn::parse_str::<syn::Stmt>(bm),
        syn::parse_str::<syn::Stmt>(bp),
    ) {
        (Ok(syn::Stmt::Local(m)), Ok(syn::Stmt::Local(p))) => (m, p),
        _ => return false,
    };
    if !lm.attrs.is_empty() || !lp.attrs.is_empty() {
        return false;
    }
    if lm.init.is_none() || lp.init.is_none() {
        return false;
    }
    let norm = |pat: &syn::Pat| {
        quote::quote!(#pat)
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    norm(&lm.pat) == norm(&lp.pat)
}

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
    // The def hunk holds exactly the header pair plus one (WI-20 /
    // WI-13 slice 1), two (WI-13 slice 2), or three (WI-13 slice 3)
    // body pairs.
    let (def_sec, def_hunk) = (flat[def_idx].sec, flat[def_idx].hunk);
    let mut members: Vec<usize> = flat
        .iter()
        .enumerate()
        .filter(|(_, fp)| fp.sec == def_sec && fp.hunk == def_hunk)
        .map(|(i, _)| i)
        .collect();
    members.sort_unstable();
    if (members.len() < 2 || members.len() > 4) || !members.contains(&def_idx) {
        return None;
    }
    let body_idxs: Vec<usize> = members.iter().copied().filter(|&i| i != def_idx).collect();
    // Rename-only header (slices 1–3) or a Joint-A-shaped slot
    // evolution (slice 4 triple — the raiser's `header_fire_gates`;
    // B1-shaped headers decline here and keep today's B1-with-leftover
    // path via the Joint-A/B arm below).
    if !slots_equal_except_name(&flat[def_idx].minus, &flat[def_idx].plus)
        && !header_fire_gates(&flat[def_idx].minus, &flat[def_idx].plus)
    {
        return None;
    }
    // Whole-line body pairs, each differing text, none itself a call
    // site of the rename: a general expression (WI-20) or a plain
    // same-binder `let` statement with an init (WI-13 slice 1),
    // checked per pair (slices 2-3 admit two or three). Binder-changing `let`
    // pairs decline — binder renames belong to D11/D16 capture, out of
    // scope here.
    let mut bodies: Vec<(String, String)> = Vec::new();
    for &body_idx in &body_idxs {
        let (bm, bp) = (flat[body_idx].minus.trim(), flat[body_idx].plus.trim());
        if bm.is_empty() || bp.is_empty() || bm == bp {
            return None;
        }
        if !body_pair_ok(bm, bp) {
            return None;
        }
        if is_call_site_of(&flat[body_idx].minus, &flat[body_idx].plus, old, new) {
            return None;
        }
        bodies.push((bm.to_string(), bp.to_string()));
    }
    // Every other balanced pair is a callee-only call (at least one).
    let mut calls: Vec<FlatPair> = Vec::new();
    for (i, fp) in flat.iter().enumerate() {
        if i == def_idx || body_idxs.contains(&i) {
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
    let mut claimed: Vec<FlatPair> = vec![flat[def_idx].clone()];
    for &body_idx in &body_idxs {
        claimed.push(flat[body_idx].clone());
    }
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
        joint: Joint::Body { bodies },
        leftover: HashMap::new(),
        claimed_hunk_bytes,
        split_extra: 0,
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
    fn body_joint_slot_evolution_triple_fires_verified_byte_identical() {
        // WI-13 slice 4: rename + one Joint-A-shaped slot evolution
        // (params retype here) + one body pair fires as one follow
        // hunk carrying the evolved header and the body pair.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: u32) -> i32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("triple must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("# sweep"), "got:\n{out}");
        assert!(
            out.contains("- fn foo(x: i32) -> i32 { ... }"),
            "got:\n{out}"
        );
        assert!(
            out.contains("+ fn bar(x: u32) -> i32 { ... }"),
            "got:\n{out}"
        );
        assert!(out.contains("- x + 1"), "got:\n{out}");
        assert!(out.contains("+ x + 2"), "got:\n{out}");
        assert_eq!(out.matches("rule=").count(), 1, "one rule, got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: u32) -> i32 {\n    x + 2\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "triple must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_ret_evolution_triple_fires() {
        // WI-13 slice 4: a return-only evolution rides the triple too
        // (Joint-A-shaped: params preserved, return evolving).
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: i32) -> u32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("ret triple must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(
            out.contains("+ fn bar(x: i32) -> u32 { ... }"),
            "got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> u32 {\n    x + 2\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "ret triple must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_generics_evolution_triple_suppresses() {
        // Generics evolution is outside the Joint-A law — the triple
        // declines and the section stays wholly ordinary.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo<T>(x: T) -> T {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo<T>(x: T) -> T {\n-    x\n+fn bar<U>(x: U) -> U {\n+    x + x\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "generics-evolving triple must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn body_joint_vis_only_triple_suppresses() {
        // A vis-only evolution is the terse intent's domain (Joint-A
        // reserves it) — with body pairs it declines, never fragments.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+pub fn bar(x: i32) -> i32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "vis-only triple must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn body_joint_three_slot_triple_suppresses() {
        // Three slot evolutions exceed the Joint-A budget — the triple
        // declines and the section stays wholly ordinary.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+pub fn bar(x: u32) -> u64 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "three-slot triple must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn body_joint_arg_add_header_fused_body_suppresses() {
        // A B1-shaped header (params evolve + uniform arg-add calls)
        // with a body pair fused in the def hunk keeps today's
        // boundary: the body pair is neither a call-template arg nor
        // a follow-owned line, so the section stays wholly ordinary —
        // never a triple, never a partial B1.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: i32, y: i32) -> i32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "fused B1+body shape must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn body_joint_two_body_lines_fire_verified_byte_identical() {
        // WI-13 slice 2: rename + two body pairs (mixed `let` + expr)
        // fires as one follow hunk carrying both pairs.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) -> i32 {\n-    let y = x + 1;\n-    y * 2\n+fn bar(x: i32) -> i32 {\n+    let y = x + 2;\n+    y * 3\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("two-pair joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- let y = x + 1;"), "got:\n{out}");
        assert!(out.contains("+ let y = x + 2;"), "got:\n{out}");
        assert!(out.contains("- y * 2"), "got:\n{out}");
        assert!(out.contains("+ y * 3"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    let y = x + 2;\n    y * 3\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "two-pair joint must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_three_body_lines_fire_verified_byte_identical() {
        // WI-13 slice 3: rename + three body pairs (two same-binder
        // `let`s + one expr) fires as one follow hunk carrying all
        // three pairs.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    z + 0\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,4 +1,4 @@\n-fn foo(x: i32) -> i32 {\n-    let y = x + 1;\n-    let z = y * 2;\n-    z + 0\n+fn bar(x: i32) -> i32 {\n+    let y = x + 2;\n+    let z = y * 3;\n+    z + 1\n@@ -7,2 +7,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("three-pair joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- let y = x + 1;"), "got:\n{out}");
        assert!(out.contains("+ let y = x + 2;"), "got:\n{out}");
        assert!(out.contains("- let z = y * 2;"), "got:\n{out}");
        assert!(out.contains("+ let z = y * 3;"), "got:\n{out}");
        assert!(out.contains("- z + 0"), "got:\n{out}");
        assert!(out.contains("+ z + 1"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    let y = x + 2;\n    let z = y * 3;\n    z + 1\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "three-pair joint must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_four_body_lines_suppress() {
        // Four body-line deltas stay ordinary (WI-13 slice 3 admits
        // exactly three — IMPROVE caps the set there).
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    let w = z + 0;\n    w - 0\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,5 +1,5 @@\n-fn foo(x: i32) -> i32 {\n-    let y = x + 1;\n-    let z = y * 2;\n-    let w = z + 0;\n-    w - 0\n+fn bar(x: i32) -> i32 {\n+    let y = x + 2;\n+    let z = y * 3;\n+    let w = z + 1;\n+    w - 1\n@@ -8,2 +8,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "four-body-line shape must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn body_joint_let_single_fires_verified_byte_identical() {
        // WI-13 slice 1: rename + one same-binder `let` body pair fires
        // as the single follow hunk, like the expr single.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = 1;\n    y\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    let y = 1;\n+fn bar(x: i32) -> i32 {\n+    let y = 2;\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("let body joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- let y = 1;"), "got:\n{out}");
        assert!(out.contains("+ let y = 2;"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    let y = 2;\n    y\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "let body joint must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_let_binder_change_fires_with_split_leftover() {
        // WI-15 slice 1: a binder-changing `let` pair is no joint body
        // (D11/D16 capture — the joint never rewrites the binder), but
        // it no longer sinks the provable header+calls intent. The def
        // hunk splits: the rename rides one follow hunk, the binder
        // pair a verbatim leftover hunk.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = 1;\n    y\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    let y = 1;\n+fn bar(x: i32) -> i32 {\n+    let z = 1;\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre)
            .expect("split header+calls intent must fire");
        assert!(full.contains("# follow=uses"), "got:\n{full}");
        assert!(full.contains("-    let y = 1;\n"), "got:\n{full}");
        assert!(full.contains("+    let z = 1;\n"), "got:\n{full}");
        validated_live(&full, &pre);
        let parsed = rsmpl_patch::parse_patch(&full).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    let z = 1;\n    y\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "split must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_two_pair_second_binder_change_fires_with_split_leftover() {
        // WI-15 slice 1: per-pair partition — the clean Expr pair is
        // re-harvested as a single by the chain pass over the rebuilt
        // section, while the binder-changing pair stays a verbatim
        // leftover beside the header follow hunk.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) -> i32 {\n-    let y = x + 1;\n-    y * 2\n+fn bar(x: i32) -> i32 {\n+    let z = x + 1;\n+    y * 3\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre)
            .expect("split header+calls intent must fire");
        assert!(full.contains("# follow=uses"), "got:\n{full}");
        assert!(full.contains("-    let y = x + 1;\n"), "got:\n{full}");
        assert!(full.contains("+    let z = x + 1;\n"), "got:\n{full}");
        assert!(full.contains("- E0 * 2\n"), "got:\n{full}");
        assert!(full.contains("+ E0 * 3\n"), "got:\n{full}");
        validated_live(&full, &pre);
        let parsed = rsmpl_patch::parse_patch(&full).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    let z = x + 1;\n    y * 3\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "split must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_three_pair_third_binder_change_fires_with_split_leftover() {
        // WI-15 slice 1: two clean pairs re-harvest as singles, the
        // binder-changing third pair stays verbatim, the header rides
        // the follow hunk — one partitioned section, byte-identical.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = x + 1;\n    let z = y * 2;\n    z + 0\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,4 +1,4 @@\n-fn foo(x: i32) -> i32 {\n-    let y = x + 1;\n-    let z = y * 2;\n-    z + 0\n+fn bar(x: i32) -> i32 {\n+    let y = x + 2;\n+    let w = y * 2;\n+    z + 1\n@@ -7,2 +7,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre)
            .expect("split header+calls intent must fire");
        assert!(full.contains("# follow=uses"), "got:\n{full}");
        assert!(full.contains("-    let z = y * 2;\n"), "got:\n{full}");
        assert!(full.contains("+    let w = y * 2;\n"), "got:\n{full}");
        assert!(full.contains("- E0 + 1\n"), "got:\n{full}");
        assert!(full.contains("+ E0 + 2\n"), "got:\n{full}");
        validated_live(&full, &pre);
        let parsed = rsmpl_patch::parse_patch(&full).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    let y = x + 2;\n    let w = y * 2;\n    z + 1\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "split must round-trip byte-identical"
        );
    }

    #[test]
    fn body_joint_non_let_body_fires_with_split_leftover() {
        // WI-15 slice 1: an init-less `let` carries no init delta, so
        // it is no joint body — but like a binder change it parks as a
        // verbatim leftover while the rename rides the follow hunk.
        // (The call hunk cites the true sites 7,8: a stale range fails
        // the D13 probe and suppresses instead — never guessed.)
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y;\n    y = 1;\n    y\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    let y;\n+fn bar(x: i32) -> i32 {\n+    let z;\n@@ -7,2 +7,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre)
            .expect("split header+calls intent must fire");
        assert!(full.contains("# follow=uses"), "got:\n{full}");
        assert!(full.contains("-    let y;\n"), "got:\n{full}");
        assert!(full.contains("+    let z;\n"), "got:\n{full}");
        validated_live(&full, &pre);
        let parsed = rsmpl_patch::parse_patch(&full).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32) -> i32 {\n    let z;\n    y = 1;\n    y\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "split must round-trip byte-identical"
        );
    }
}
