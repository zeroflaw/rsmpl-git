//! WI-5d single-block hetero fusion: one contiguous run of K = 2–4
//! hetero plain-`let` pairs (a single block window, N = 1) fused into
//! one `Sequence`-shape hunk with within-pair abstraction.
//!
//! Unlike the WI-5c repeated-block detector (N ≥ 2 blocks sharing
//! cross-block holes), there is no cross-pair signal here: each
//! position anti-unifies *within* its own pair — a subterm preserved
//! verbatim between that pair's minus and plus rides a positional
//! hole (`Pj`/`Tj`/`Ej`, WI-5c numbering so window statements bind
//! independently, SPEC 15.7), anything that differs stays concrete.
//! Positions are independent: no hole is shared across lines, so no
//! cross-statement equality is ever imposed.
//!
//! Territory gate: the run must mix init families (expression
//! kinds). A family-uniform run is an earlier detector's class —
//! its verdict (fire or veto) stands, never undercut by a narrower
//! re-spelling here. Only kind-hetero runs, which no earlier
//! detector's shape matches uniformly, fuse.
//!
//! The emitted hunk is a single-window `Sequence` (tight original
//! range, allow-one — no `# allow=many`, no `# sweep`): the
//! single-window claim means an unexpected twin fails loud as
//! too-many instead of silently double-rewriting. Soundness is the
//! real-pair engine-count totality
//! (`template_sequence_holds` with n = 1): the K-line minus must
//! match exactly one statement window in the pre-image. The T375
//! byte-win guard in the callers decides emission — a fusion that
//! costs more than its ordinary lines stays ordinary.
//!
//! Boundaries (stays ordinary, documented non-goals): mixed runs (a header,
//! expression, or any non-plain-`let` member — permanent non-goal, no
//! sub-hunk partition (full-history probe: 0/2473 paired hunks over 467
//! commits hold the splittable shape, so fixture `12-stmt-rewrite`'s
//! header+lets run stays ordinary); family-uniform
//! runs (every init the same expression kind and head — earlier
//! detectors own the verdict, including vetoes); one-sided ascriptions
//! (a type-add/removal inside the run); K = 1 (true singles stay
//! ordinary, WI-4 N = 1 rule) and K > 4 (the `Sequence` engine arm's
//! exact-K bound).

use super::antiunify::{is_plain_let_line, parse_typed_let_full, parse_untyped_let_full, tok};
use super::shape::init_ok;
use super::unique_rule;
use quote::ToTokens;

/// Within-pair anti-unification for one `let` pair at sequence
/// position `j`: preserved subterms ride positional holes, deltas
/// stay concrete. Returns full `let` statement templates, or `None`
/// when the pair is not one uniform within-pair rewrite (v1
/// boundary, never guessed).
///
/// Slot discipline: pattern token-equal → `P{j}`, else concrete
/// both sides; type both-absent → absent, both-present+equal →
/// `T{j}`, both-present+differ → concrete both, one-sided →
/// decline; init token-equal → `E{j}` (gated by the shared
/// engine-matchability gate `init_ok`), else concrete both. Concrete
/// slots render in original site spelling (span-sliced, type-sweep
/// precedent) — never token-normalized — so the post-tree keeps the
/// worktree's spacing (`wrap(alpha)` stays `wrap(alpha)`, not diet
/// `wrap (alpha)`); comparison stays token-based. The templates must
/// differ (a whitespace-only delta equalizes: joinery is normalized)
/// and both must re-parse as plain lets; the engine-count totality
/// probe (not this core) proves the window is unique. Plus-side holes
/// are bound on minus by construction — every plus hole has a
/// same-named minus hole from the same slot.
fn antiunify_single_block_pair(m: &str, p: &str, j: usize) -> Option<(String, String)> {
    let (m_pat, m_ty, m_init) = parse_let_sides(m)?;
    let (p_pat, p_ty, p_init) = parse_let_sides(p)?;
    let (pj, tj, ej) = (format!("P{j}"), format!("T{j}"), format!("E{j}"));
    let pat_tpl = if tok(&m_pat) == tok(&p_pat) {
        pj.clone()
    } else {
        // Concrete delta in original site spelling (the engine
        // compacts spacing at compare time, so either spelling
        // matches; site spelling keeps the post-tree natural).
        slice_node(m, &m_pat)?
    };
    let pat_plus = if tok(&m_pat) == tok(&p_pat) {
        pj.clone()
    } else {
        slice_node(p, &p_pat)?
    };
    let (ty_tpl, ty_plus) = match (m_ty, p_ty) {
        (None, None) => (None, None),
        (Some(a), Some(b)) => {
            if tok(&a) == tok(&b) {
                (Some(tj.clone()), Some(tj))
            } else {
                (Some(slice_node(m, &a)?), Some(slice_node(p, &b)?))
            }
        }
        // One-sided ascription (a type-add/removal inside the run):
        // the addition/removal cores own whole-class singles, never
        // one fused position — stays ordinary.
        _ => return None,
    };
    let (init_tpl, init_plus) = if tok(&m_init) == tok(&p_init) {
        if !init_ok(&m_init) {
            return None;
        }
        (ej.clone(), ej)
    } else {
        // Concrete delta inits ride the same engine-matchability gate
        // as holes (SPEC 18.6 reject table): a macro/opaque init is
        // structurally unmatchable, so the template could never fire —
        // decline instead of emitting a dead hunk (much less a blind
        // one under section-visible evidence).
        if !init_ok(&m_init) || !init_ok(&p_init) {
            return None;
        }
        (slice_node(m, &m_init)?, slice_node(p, &p_init)?)
    };
    let render = |pat: &str, ty: &Option<String>, init: &str| match ty {
        Some(t) => format!("let {pat}: {t} = {init};"),
        None => format!("let {pat} = {init};"),
    };
    let minus_tpl = render(&pat_tpl, &ty_tpl, &init_tpl);
    let plus_tpl = render(&pat_plus, &ty_plus, &init_plus);
    if minus_tpl == plus_tpl {
        return None;
    }
    if !is_plain_let_line(&minus_tpl) || !is_plain_let_line(&plus_tpl) {
        return None;
    }
    Some((minus_tpl, plus_tpl))
}

/// Original site spelling of one parsed subterm: the node's own span
/// sliced from its trimmed source line (single-line lets only, so the
/// 1-based span columns index the line). `None` on multi-line spans,
/// out-of-bounds columns, or non-char-boundaries (extract_type_pair
/// posture) — the caller declines, never guesses a rendering.
fn slice_node<T: ToTokens>(line: &str, node: &T) -> Option<String> {
    use syn::spanned::Spanned as _;
    let trimmed = line.trim();
    let span = node.span();
    if span.start().line != 1 || span.end().line != 1 {
        return None;
    }
    // Span columns are Unicode scalars (SPEC 31.3), not bytes.
    let s = rsmpl_pattern::char_col_to_byte(trimmed, span.start().column);
    let e = rsmpl_pattern::char_col_to_byte(trimmed, span.end().column);
    if s >= e || e > trimmed.len() {
        return None;
    }
    trimmed
        .get(s..e)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// One `let` side as (pattern, optional ascribed type, init): typed
/// and untyped parses are disjoint (`Pat::Type` presence), so trying
/// both covers every plain `let` exactly once. Attributes, missing
/// inits, and `let-else` diverge veto via the shared parsers.
fn parse_let_sides(line: &str) -> Option<(syn::Pat, Option<syn::Type>, syn::Expr)> {
    if let Some((pat, ty, init)) = parse_typed_let_full(line) {
        return Some((pat, Some(ty), init));
    }
    let (pat, init) = parse_untyped_let_full(line)?;
    Some((pat, None, init))
}

/// One init's expression family: the `syn::Expr` variant. Families
/// decide territory, never matching: a run whose every init shares
/// one family is an earlier detector's class (their verdicts —
/// including totality and spelling vetoes — stand, never undercut by
/// a narrower re-spelling here); a run mixing families is hetero,
/// owned by no earlier detector, and fuses. Deliberately
/// variant-level, not head-aware: same-kind runs (two bare calls,
/// however hetero the callees) stay earlier detectors' territory,
/// so no veto (`leading-::`/turbofish/divergent-callee/twin)
/// is ever re-spelled around here.
fn init_family(init: &syn::Expr) -> String {
    // `Discriminant` debug is an opaque index — equality is all the
    // gate needs, never the name.
    format!("node:{:?}", std::mem::discriminant(init))
}

/// Within-pair anti-unification over a single K = 2–4 run: position
/// `j` generalizes only its own pair, holes numbered per position so
/// window statements bind independently. Returns the K minus/plus
/// template lines, or `None` (stays ordinary).
///
/// Territory gate: the run must mix init families (at least two
/// positions with different expression kinds). A family-uniform run
/// is an earlier detector's class: call sweeps own call runs, the
/// fold owns nested spines, the literal sweep owns trailing
/// literals, and their vetoes
/// (turbofish/leading-`::`/mixed-depth/divergent-callee/twin)
/// stand. Only kind-hetero runs (no earlier detector's shape
/// matches uniformly) fuse here.
fn antiunify_single_block(minuses: &[&str], pluses: &[&str]) -> Option<(Vec<String>, Vec<String>)> {
    if minuses.len() != pluses.len() || !(2..=4).contains(&minuses.len()) {
        return None;
    }
    // All-identical whole classes stay for the tighter verbatim sweep
    // (T422 precedent).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut minus_tpls = Vec::with_capacity(minuses.len());
    let mut plus_tpls = Vec::with_capacity(pluses.len());
    let mut families = Vec::with_capacity(minuses.len());
    for (j, (m, p)) in minuses.iter().zip(pluses.iter()).enumerate() {
        let (_, _, m_init) = parse_let_sides(m)?;
        families.push(init_family(&m_init));
        let (tm, tp) = antiunify_single_block_pair(m, p, j)?;
        minus_tpls.push(tm);
        plus_tpls.push(tp);
    }
    if families.iter().all(|f| *f == families[0]) {
        return None;
    }
    if minus_tpls.iter().zip(plus_tpls.iter()).all(|(m, p)| m == p) {
        return None;
    }
    Some((minus_tpls, plus_tpls))
}

/// Single-block sweep (WI-5d): one contiguous run of K = 2–4 hetero
/// plain-`let` pairs fused into one allow-one `Sequence` hunk (tight
/// original range). Contiguity (`changed` consecutive) is the run
/// gate — the engine only matches consecutive statement windows, so
/// a context-split class declines. Mixed runs (any non-plain-`let`
/// member) decline: v1 does no sub-hunk split. Totality is the
/// real-pair engine-count check with n = 1
/// (`template_sequence_holds`): the K-line minus must match exactly
/// one window in the pre-image — an untouched twin vetoes, exactly
/// like the real sweep rewrites. Detector name `single_block`.
pub(crate) fn try_single_block_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let n = minuses.len();
    if n != pluses.len() || !(2..=4).contains(&n) || changed.len() != n {
        return None;
    }
    // One run: every changed old line adjacent (a context line inside
    // the span means two runs, which v1 never fuses).
    for w in changed.windows(2) {
        if w[1] != w[0] + 1 {
            return None;
        }
    }
    // Pure let runs only — a header, expression, or any other member
    // (fixture 12's fn-header line) declines the whole run; v1 leaves
    // mixed runs ordinary instead of splitting hunks.
    if !minuses.iter().all(|l| is_plain_let_line(l)) || !pluses.iter().all(|l| is_plain_let_line(l))
    {
        return None;
    }
    // All-identical whole classes stay for the tighter verbatim sweep
    // (T422 precedent).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let (minus_tpls, plus_tpls) = antiunify_single_block(minuses, pluses)?;
    if !crate::totality::template_sequence_holds(path, file_old, &minus_tpls, &plus_tpls, 1) {
        return None;
    }
    let rule = unique_rule(used_rules, "single_block");
    // Allow-one tight range (SPEC D.5): a single-window claim needs no
    // `# allow=many` and no `# sweep` — an unexpected twin fails loud
    // as too-many at apply time.
    let min = changed.iter().min().copied().unwrap_or(0);
    if min < 1 {
        return None;
    }
    let mut hunk = format!(
        "{}\n@@ -{min},{n} @@ rule={rule}\n",
        crate::git_header(path)
    );
    for l in &minus_tpls {
        hunk.push_str(&format!("- {l}\n"));
    }
    for l in &plus_tpls {
        hunk.push_str(&format!("+ {l}\n"));
    }
    Some(hunk)
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic, infer_semantic_with_pre_images};
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
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

    /// Fixture-12's let run: one holed init (`E0`) plus one concrete
    /// rename position. The fn-header pair is excluded (mixed runs
    /// decline) — this is the pure-let core the detector owns.
    const PRE_TWO: &str =
        "fn scale(values: Vec<u32>) -> u32 {\n    let count = values.len() as u32;\n    let bumped = count + 1;\n    bumped\n}\n";

    fn two_let_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let count = values.len() as u32;\n-    let bumped = count + 1;\n+    let n = values.len() as u32;\n+    let bumped = n + 1;\n"
            .to_string()
    }

    #[test]
    fn single_block_fires() {
        let raw = section("f.rs", &two_let_body());
        let pre = pre_map(&[("f.rs", PRE_TWO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        // Allow-one: no sweep machinery on a single-window claim.
        assert!(!out.contains("# allow=many"), "got: {out}");
        assert!(!out.contains("# sweep"), "got: {out}");
        assert!(out.contains("- let count = E0;\n"), "got: {out}");
        assert!(out.contains("- let P1 = count + 1;\n"), "got: {out}");
        assert!(out.contains("+ let n = E0;\n"), "got: {out}");
        assert!(out.contains("+ let P1 = n + 1;\n"), "got: {out}");
        // Inferred holes need no declarations (SPEC 15.8).
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("# pat"), "got: {out}");
        // Consumed: no native let lines survive.
        assert!(!out.contains("values.len()"), "got: {out}");
    }

    #[test]
    fn single_block_twin_window_vetoes() {
        // Totality: an untouched twin window in the pre-image would be
        // silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &two_let_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn scale(values: Vec<u32>) -> u32 {\n    let count = values.len() as u32;\n    let bumped = count + 1;\n    bumped\n}\nfn other() -> u32 {\n    let count = values.len() as u32;\n    let bumped = count + 1;\n    bumped\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn single_block_header_member_vetoes() {
        // Mixed runs decline: the fn-header pair cannot join a let
        // window (v1 does no sub-hunk split).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn scale(values: Vec<u32>) -> u32 {\n-    let count = values.len() as u32;\n-    let bumped = count + 1;\n+fn scale(values: Box<[u32]>) -> u32 {\n+    let n = values.len() as u32;\n+    let bumped = n + 1;\n",
        );
        let pre = pre_map(&[("f.rs", PRE_TWO)]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn single_block_split_run_vetoes() {
        // A context line inside the span means two runs, never one
        // window: the engine could not match across the gap.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-    let alpha = compute_first(input);\n     let keep = compute_keep(other);\n-    let beta = compute_second(extra);\n+    let aaa = compute_first(input);\n     let keep = compute_keep(other);\n+    let bbb = compute_second(extra);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let alpha = compute_first(input);\n    let keep = compute_keep(other);\n    let beta = compute_second(extra);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn single_block_one_sided_ascription_vetoes() {
        // A type-add inside the run is another detector's class (whole
        // singles own it), never one fused position.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let alpha = compute_first(input);\n-    let beta = wrap(alpha);\n+    let alpha: u32 = compute_first(input);\n+    let beta = wrap(bbb);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let alpha = compute_first(input);\n    let beta = wrap(alpha);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn single_block_all_identical_declines() {
        // Core-level T422 guard: duplicate pairs belong to the tighter
        // verbatim sweep (which dedups them upstream at chain level),
        // never to a hole template over the same lines.
        let m = [
            "    let alpha = compute_first(input);",
            "    let alpha = compute_first(input);",
        ];
        let p = [
            "    let aaa = compute_first(input);",
            "    let aaa = compute_first(input);",
        ];
        assert!(super::antiunify_single_block(&m, &p).is_none());
    }

    #[test]
    fn single_block_k_bounds_decline() {
        // K = 1 is a true single (WI-4 N = 1 rule: stays ordinary).
        let one = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    let count = values.len() as u32;\n+    let n = values.len() as u32;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&one)).is_none());
        // K = 5 exceeds the Sequence engine arm's exact-K bound (hetero
        // class no earlier detector owns, so the K gate is what
        // declines — a uniform f→g class would rightly belong to the
        // call sweep instead).
        let five = section(
            "f.rs",
            "@@ -2,5 +2,5 @@\n-    let a1 = f1(x1);\n-    let a2 = f2(x2);\n-    let a3 = f3(x3);\n-    let a4 = f4(x4);\n-    let a5 = f5(x5);\n+    let b1 = f1(x1);\n+    let b2 = f2(x2);\n+    let b3 = f3(x3);\n+    let b4 = f4(x4);\n+    let b5 = f5(x5);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let a1 = f1(x1);\n    let a2 = f2(x2);\n    let a3 = f3(x3);\n    let a4 = f4(x4);\n    let a5 = f5(x5);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&five), &pre).is_none());
    }

    #[test]
    fn single_block_typed_positions_fire() {
        // Mixed typed/untyped positions generalize independently: the
        // typed position rides T1. Inits are family-hetero (call vs
        // binary) — a family-uniform run is an earlier detector's
        // territory and stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let alpha = compute_first(input);\n-    let beta: u64 = delta + 1;\n+    let aaa = compute_first(input);\n+    let bbb: u64 = delta + 2;\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let alpha = compute_first(input);\n    let beta: u64 = delta + 1;\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let alpha = E0;\n"), "got: {out}");
        assert!(out.contains("- let beta: T1 = delta + 1;\n"), "got: {out}");
        assert!(out.contains("+ let aaa = E0;\n"), "got: {out}");
        assert!(out.contains("+ let bbb: T1 = delta + 2;\n"), "got: {out}");
    }

    #[test]
    fn single_block_family_uniform_run_declines() {
        // Same init family (two bare calls, however hetero the
        // callees — call-sweep territory, verdict stands even when it
        // vetoes) never fuses here; only kind-hetero runs do. Pat
        // renames with preserved inits are the shape; the shared
        // `Call` kind is the gate.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let alpha = compute_first(input);\n-    let beta = wrap(delta);\n+    let aaa = compute_first(input);\n+    let bbb = wrap(delta);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let alpha = compute_first(input);\n    let beta = wrap(delta);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn single_block_whitespace_only_delta_vetoes() {
        // A spacing-only delta equalizes under token comparison: the
        // templates would be identical (a no-op hunk) — decline.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let alpha=compute_first(input);\n-    let beta = wrap(alpha);\n+    let alpha = compute_first(input);\n+    let beta = wrap(alpha);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let alpha=compute_first(input);\n    let beta = wrap(alpha);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn single_block_unparseable_pre_vetoes() {
        let raw = section("f.rs", &two_let_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }
}
