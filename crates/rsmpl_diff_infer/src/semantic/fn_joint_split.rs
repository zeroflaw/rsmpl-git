//! WI-15 slice 1 (D5): def-hunk partition for the fn-joint pre-pass.
//!
//! Split from `fn_joint.rs` under the 2000-line gate (D5): pure move,
//! no behavior change. `fn_joint.rs` calls `try_def_hunk_split` before
//! its B2 fork; tests stay inline here per WI-22.
//!
//! A fused def hunk (a `fn`-header pair plus body pairs in one balanced
//! hunk) whose bodies the body joint declined (binder-changing lets,
//! init-less lets, plain expressions) would fail `callee_only` and
//! suppress the provable header+calls intent. The split parks each
//! non-call-site def-hunk body pair as a tight verbatim leftover hunk
//! so the reduced header+calls claim proceeds through the unchanged
//! Joint-A gates. The split only engages where today suppresses, so no
//! existing fire changes shape.

use std::collections::{HashMap, HashSet};

use super::fn_joint_b2::attr_def_shape;
use super::fn_joint_calls::is_call_site_of;
use super::fn_joint_sig::{header_fire_gates, slots_equal_except_name};
use super::fn_joint_types::FlatPair;

/// A parked def-hunk split: flat indices of parked body pairs plus the
/// synthesized verbatim leftover hunks (site order) for the def section.
pub(crate) struct DefSplit {
    pub(crate) parked: HashSet<usize>,
    pub(crate) sec: usize,
    pub(crate) hunks: Vec<String>,
    /// Synthesized `@@` header bytes (emission side of the byte gate).
    pub(crate) overhead: usize,
    /// Parked raw bytes (leave the claimed side of the byte gate).
    pub(crate) parked_bytes: usize,
}

/// Outcome of the def-hunk split probe: `Skip` (no fused bodies —
//  today's paths own the group), `Split` (park and proceed), or
/// `Suppress` (a fused shape no partitioned form proves — same verdict
/// as today's wholesale suppress, decided early).
pub(crate) enum SplitOutcome {
    Skip,
    Split(DefSplit),
    Suppress,
}

/// Probe a positional def hunk for the WI-15 partition. `d` is the
/// flat index of the header pair. Returns `Skip` when the def hunk
/// holds no non-call-site bodies; `Split` with tight verbatim hunks
/// when a Joint-A-shaped or rename-only header (never B1 — decided
/// downstream) fronts one to three `syn::Stmt`/`syn::Expr` body pairs;
/// `Suppress` for every other fused shape (attr/doc/item lines, more
/// than three bodies, non-joint headers, attr-shaped company,
/// unparseable or misaligned ranges — never guessed).
pub(crate) fn try_def_hunk_split(
    flat: &[FlatPair],
    d: usize,
    hunk_members: &HashMap<(usize, usize), Vec<usize>>,
    hunk_texts: &HashMap<(usize, usize), String>,
    unbalanced_hunks: &[(usize, usize, Vec<String>, Vec<String>)],
    old: &str,
    new: &str,
) -> SplitOutcome {
    let (ds, dh) = (flat[d].sec, flat[d].hunk);
    let in_def: Vec<usize> = hunk_members.get(&(ds, dh)).cloned().unwrap_or_default();
    let mut bodies: Vec<usize> = in_def
        .iter()
        .copied()
        .filter(|i| *i != d && !is_call_site_of(&flat[*i].minus, &flat[*i].plus, old, new))
        .collect();
    bodies.sort_unstable();
    if bodies.is_empty() {
        return SplitOutcome::Skip;
    }
    // An attr-shaped unbalanced hunk keeps today's B2 path (its
    // composition with in-hunk bodies is unprobed) — the split stays
    // out of its way entirely.
    let has_attr = unbalanced_hunks
        .iter()
        .any(|(_, _, m, p)| attr_def_shape(m, p).is_some());
    if has_attr {
        return SplitOutcome::Skip;
    }
    // WI-15 decision: parked pairs are body *statements* — each side
    // parses as `syn::Stmt` or `syn::Expr` (binder-changing lets,
    // init-less lets, plain expressions). Anything else in the def hunk
    // (attr/doc lines, items) keeps today's Suppress: only genuine
    // body lines park as verbatim.
    let all_body = bodies.iter().all(|b| {
        let (m, p) = (&flat[*b].minus, &flat[*b].plus);
        (syn::parse_str::<syn::Stmt>(m).is_ok() && syn::parse_str::<syn::Stmt>(p).is_ok())
            || (syn::parse_str::<syn::Expr>(m).is_ok() && syn::parse_str::<syn::Expr>(p).is_ok())
    });
    if !all_body {
        return SplitOutcome::Suppress;
    }
    // WI-15 decision: one to three body lines. More parked bodies than
    // a Body joint could carry (`try_body_joint` caps at three)
    // suppress as today — the fused hunk stays out of the partitioned
    // form.
    if bodies.len() > 3 {
        return SplitOutcome::Suppress;
    }
    // Non-call-site body pairs ride only Joint-A-shaped or rename-only
    // headers (the follow-routed single-hunk form, whose leftover
    // composition is WI-4f-next precedent). Whether a params-evolving
    // header is Joint-A or B1 is decided downstream by the calls
    // (callee-only or not) — the split must not prejudge it here.
    let joint_a = header_fire_gates(&flat[d].minus, &flat[d].plus);
    let rename_only = slots_equal_except_name(&flat[d].minus, &flat[d].plus);
    if !joint_a && !rename_only {
        return SplitOutcome::Suppress;
    }
    // Tight verbatim hunks in site order. Plus-side sites come from the
    // def hunk header (`+c` + positional index for the balanced hunk);
    // any parse or alignment mismatch declines to Suppress, never
    // guesses.
    let starts = hunk_texts
        .get(&(ds, dh))
        .and_then(|t| parse_def_hunk_starts(t));
    let Some((old_start, new_start)) = starts else {
        return SplitOutcome::Suppress;
    };
    let mut order = in_def.clone();
    order.sort_unstable();
    let mut parked: HashSet<usize> = HashSet::new();
    let mut hunks: Vec<String> = Vec::new();
    let mut overhead: usize = 0;
    let mut parked_bytes: usize = 0;
    for b in &bodies {
        let pos = order.iter().position(|m| m == b).unwrap_or(usize::MAX);
        if pos == usize::MAX || flat[*b].site != old_start + pos {
            return SplitOutcome::Suppress;
        }
        let head = format!("@@ -{} +{} @@\n", flat[*b].site, new_start + pos);
        overhead += head.len();
        // Raw parked bytes (marker + text + newline per side),
        // subtracted from the claimed side of the byte gate.
        parked_bytes += flat[*b].minus.len() + flat[*b].plus.len() + 4;
        parked.insert(*b);
        hunks.push(format!("{head}-{}\n+{}\n", flat[*b].minus, flat[*b].plus));
    }
    SplitOutcome::Split(DefSplit {
        parked,
        sec: ds,
        hunks,
        overhead,
        parked_bytes,
    })
}

/// Parse a def hunk header `@@ -a[,b] +c[,d] @@` into `(old_start,
/// new_start)`. Elided `,1` lengths read as 1. `None` on any shape
/// mismatch — the caller declines to Suppress, never guesses.
fn parse_def_hunk_starts(hunk_text: &str) -> Option<(usize, usize)> {
    let header = hunk_text.lines().next()?;
    let inner = header.strip_prefix("@@")?.split("@@").next()?.trim();
    let mut parts = inner.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let start_of = |s: &str| s.split(',').next()?.parse::<usize>().ok();
    let (a, c) = (start_of(old)?, start_of(new)?);
    if a == 0 || c == 0 {
        return None;
    }
    Some((a, c))
}

/// Header admission for the reduced header+calls claim. Without a split
/// only Joint-A-shaped headers fire (pure renames are item_intent's);
/// with a split (parked bodies) a rename-only header also fires —
/// disjoint from item_intent, which owns leftover-free renames via the
/// `Candidate::None` arm.
pub(crate) fn admits_split_claim(minus: &str, plus: &str, split_active: bool) -> bool {
    header_fire_gates(minus, plus) || (split_active && slots_equal_except_name(minus, plus))
}

/// Byte gate for a split fire: the proved header+calls set prices the
/// WI-7 precision allowance over its pair count, with the synthesized
/// leftover header overhead on the emission side (never uncounted).
pub(crate) fn split_fire_gate(
    emission_bytes: usize,
    claimed_bytes: usize,
    n_claims: usize,
    extra: usize,
) -> bool {
    super::guard::within_precision(emission_bytes + extra, claimed_bytes, n_claims, 0)
}

/// Claim the def hunk's non-parked members (the header plus call-site
/// pairs). Without a split this claims the whole def hunk, as before.
pub(crate) fn claim_def_members(members: &[usize], parked: &HashSet<usize>, out: &mut Vec<usize>) {
    for i in members {
        if !parked.contains(i) {
            out.push(*i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
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

    #[test]
    fn def_hunk_split_joint_a_fires_partial() {
        // WI-15 slice 1: Joint-A header (rename+params) fused with an
        // inadmissible body pair (binder change) plus calls. The header
        // rides one follow hunk; the body parks as a verbatim leftover.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = 1;\n    y + x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    let y = 1;\n+fn bar(x: u32) -> i32 {\n+    let z = 1;\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre)
            .expect("split Joint-A intent must fire");
        assert!(full.contains("# follow=uses"), "got:\n{full}");
        assert!(
            full.contains("+ fn bar(x: u32) -> i32 { ... }"),
            "got:\n{full}"
        );
        assert!(full.contains("-    let y = 1;\n"), "got:\n{full}");
        assert!(full.contains("+    let z = 1;\n"), "got:\n{full}");
        validated_live(&full, &pre);
        let parsed = rsmpl_patch::parse_patch(&full).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: u32) -> i32 {\n    let z = 1;\n    y + x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "split must round-trip byte-identical"
        );
    }

    #[test]
    fn def_hunk_split_b1_header_suppresses() {
        // WI-15 slice 1 residue: a B1 (arg-add) header fused with an
        // inadmissible body pair has no probe-sound composed form yet —
        // wholly ordinary, like every other B1 veto (the arg-add call
        // sweep below is the pre-existing per-hunk fallback, unchanged).
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let y = 1;\n    y + x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    let y = 1;\n+fn foo(x: i32, e: i32) -> i32 {\n+    let z = 1;\n@@ -6,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    foo(1, 0);\n+    foo(2, 0);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("# follow=uses")),
            "B1+body split must not claim the follow, got: {full:?}"
        );
    }

    #[test]
    fn def_hunk_split_four_bodies_suppress() {
        // WI-15 decision: at most three parked body lines — a fourth
        // suppresses as today.
        let pre = pre_map(&[(
            "src.rs",
            "fn foo(x: i32) -> i32 {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n    a\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
        )]);
        let sections = vec![section(
            "src.rs",
            "@@ -1,5 +1,5 @@\n-fn foo(x: i32) -> i32 {\n-    let a = 1;\n-    let b = 2;\n-    let c = 3;\n-    let d = 4;\n+fn bar(x: i32) -> i32 {\n+    let e = 1;\n+    let f = 2;\n+    let g = 3;\n+    let h = 4;\n@@ -9,2 +9,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("# follow=uses")),
            "four parked bodies must stay wholly ordinary, got: {full:?}"
        );
    }
}
