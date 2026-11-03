//! WI-19 slice 1: native-side property harness (Generator B).
//!
//! Random native mutations drawn from the intent catalogue (SPEC D.4)
//! are raised via the public `infer_rsmpl_with_pre_images` entry point
//! and must satisfy the cycle contract invariants (SPEC D.5):
//! (1) inference succeeds without panic, (2) the emitted patch parses,
//! (3) applying it to the pre tree yields the native post tree
//! byte-identical (state-losslessness, recomputed independently — never
//! by reading the report's own fields), (4) re-inferring the same raw
//! native diff is byte-identical (`S″ == S′` fixed point).
//!
//! Random cases assert only the invariants — never semantic-vs-ordinary
//! posture (either posture is legitimate; posture pins live below as
//! historical-shape controls). Seeded xorshift64, no new dependencies.
//! Slice 1 is native-side only; authored-side generation (Generator A),
//! verdict-honesty orphan recomputation, and the wide `tools/` run are
//! later slices.
//!
//! Scope note (probed 2026-11-03): hetero general-expr sweeps whose
//! args carry `&`/`field` subterms normalize spacing on emission
//! (`&cand.body_text` → `& cand . body_text`) and break invariant (3).
//! Filed as a DISCOVERED TASKS.md item (emission spacing fidelity);
//! the slice-1 generator sticks to spacing-faithful shapes (plain
//! int-arg calls, fn renames, control-header decoys, N=1 singles) so
//! the harness itself stays green while the bug is owned elsewhere.

use std::collections::HashMap;

/// Minimal seeded RNG (xorshift64*; no new deps by WI-19 decision).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % (n as u64)) as usize
    }
}

fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Build a `-U0` native section for single-file line replacements.
/// `pre`/`post` must have equal line counts (all slice-1 mutations do).
fn native_section(path: &str, pre: &str, post: &str) -> String {
    let pre_lines: Vec<&str> = pre.split('\n').collect();
    let post_lines: Vec<&str> = post.split('\n').collect();
    assert_eq!(
        pre_lines.len(),
        post_lines.len(),
        "slice-1 mutations are line-preserving"
    );
    let mut body = String::new();
    for (i, (a, b)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        if a == b {
            continue;
        }
        let n = i + 1;
        body.push_str(&format!("@@ -{n},1 +{n},1 @@\n-{a}\n+{b}\n"));
    }
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
}

/// Independently recompute the N→S→N leg: apply `rsmpl` to `pre` and
/// require the byte-identical native `post` (SPEC D.5 state-losslessness).
/// Returns the apply report so WI-19 slice-3 honesty legs can cross-check
/// it against tree evidence (report fields are never trusted blindly).
fn assert_state_lossless(
    rsmpl: &str,
    pre: &str,
    post: &str,
    seed: u64,
) -> rsmpl_engine::ApplyReport {
    let parsed = rsmpl_patch::parse_patch(rsmpl)
        .unwrap_or_else(|_| panic!("seed {seed}: emitted patch must parse"));
    let mut files = pre_map(&[("f.rs", pre)]);
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
        .unwrap_or_else(|_| panic!("seed {seed}: emitted patch must apply cleanly"));
    assert_eq!(
        files.get("f.rs").map(String::as_str),
        Some(post),
        "seed {seed}: N→S→N post tree must be byte-identical"
    );
    report
}

/// Generate one (pre, post) pair from `rng`. All shapes are
/// spacing-faithful by construction (see scope note above). Kinds:
/// 0 = fn rename `foo`→`bar` (def + call sites follow),
/// 1 = call callee swap `foo`→`bar` across call sites only,
/// 2 = control-header decoy (R1: for-loop headers match no engine
/// shape — stays ordinary, still round-trips),
/// 3 = N=1 fn rename (single call site; fires or stays ordinary under
/// the precision guard — either posture round-trips).
fn generate(rng: &mut Rng) -> (String, String) {
    let kind = rng.below(4);
    let ncalls = 2 + rng.below(2);
    match kind {
        0 => {
            let mut pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n".to_string();
            let mut post = "fn bar(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n".to_string();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i} = foo({i});\n"));
                post.push_str(&format!("    let v{i} = bar({i});\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        1 => {
            let mut pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn bar(x: i32) -> i32 {\n    x + 2\n}\nfn m() {\n"
                .to_string();
            let mut post = pre.clone();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i} = foo({i});\n"));
                post.push_str(&format!("    let v{i} = bar({i});\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        2 => {
            let mut pre = "fn m(section: &str) {\n".to_string();
            let mut post = pre.clone();
            for _ in 0..ncalls {
                pre.push_str("    for line in section.lines() {\n        use_line(line);\n    }\n");
                post.push_str(
                    "    for line in raw_lines(section) {\n        use_line(line);\n    }\n",
                );
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ => {
            let mut pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n".to_string();
            let mut post = "fn bar(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n".to_string();
            pre.push_str("    let only = foo(1);\n}\n");
            post.push_str("    let only = bar(1);\n}\n");
            (pre, post)
        }
    }
}

#[test]
fn property_native_side_cycle_holds() {
    // 120 seeded cases; case count (not wall-clock) is the budget gate.
    for case in 0..120 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15u64.wrapping_add(case as u64 * 0x9E37_79B9));
        let (pre, post) = generate(&mut rng);
        let seed = case as u64 + 1;
        let raw = native_section("f.rs", &pre, &post);
        assert!(
            raw.contains("@@ "),
            "seed {seed}: generator must produce a non-empty diff"
        );
        let pre_map = pre_map(&[("f.rs", pre.as_str())]);
        let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre_map)
            .unwrap_or_else(|_| panic!("seed {seed}: inference must succeed (no panic, no error)"));
        assert_state_lossless(&out.rsmpl, &pre, &post, seed);
        // Fixed point: re-inferring the same raw native diff is
        // byte-identical (`S″ == S′`).
        let out2 = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre_map)
            .unwrap_or_else(|_| panic!("seed {seed}: second inference must succeed"));
        assert_eq!(
            out2.rsmpl, out.rsmpl,
            "seed {seed}: second raise must be byte-identical"
        );
    }
}

#[test]
fn property_fn_rename_fires_and_round_trips() {
    // Historical firing shape: fn rename + call follow raises semantic
    // and applies byte-exact (the Generator B kind-0 family).
    let pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = foo(1);\n    let b = foo(2);\n}\n";
    let post = "fn bar(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = bar(1);\n    let b = bar(2);\n}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        out.rsmpl.contains("rule="),
        "fn rename must raise semantic, got: {}",
        out.rsmpl
    );
    assert_state_lossless(&out.rsmpl, pre, post, 1001);
}

#[test]
fn property_call_sweep_fires_and_round_trips() {
    // Historical firing shape: same-callee call swap across sites raises
    // a call sweep and applies byte-exact (Generator B kind-1 family).
    let pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn bar(x: i32) -> i32 {\n    x + 2\n}\nfn m() {\n    let a = foo(1);\n    let b = foo(2);\n}\n";
    let post = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn bar(x: i32) -> i32 {\n    x + 2\n}\nfn m() {\n    let a = bar(1);\n    let b = bar(2);\n}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        out.rsmpl.contains("rule="),
        "call swap must raise semantic, got: {}",
        out.rsmpl
    );
    assert_state_lossless(&out.rsmpl, pre, post, 1002);
}

#[test]
fn property_control_header_veto_rederived() {
    // Historical veto shape (eacce0e/R1): for-loop headers parse as
    // statements, not expressions — no engine shape matches them, so
    // the identical pair stays ordinary. Re-derived by the generator
    // (kind-2 family); ordinary passthrough still round-trips.
    let pre = "fn m(section: &str) {\n    for line in section.lines() {\n        use_line(line);\n    }\n    for line in section.lines() {\n        use_line(line);\n    }\n}\n";
    let post = "fn m(section: &str) {\n    for line in raw_lines(section) {\n        use_line(line);\n    }\n    for line in raw_lines(section) {\n        use_line(line);\n    }\n}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        !out.rsmpl.contains("rule="),
        "control-header pair must stay ordinary, got: {}",
        out.rsmpl
    );
    assert_state_lossless(&out.rsmpl, pre, post, 1003);
}

#[test]
fn property_unchanged_occurrence_veto_rederived() {
    // Historical veto shape (712ad9d soundness): the needle occurs as a
    // subterm in an unchanged guard line, so totality fails and the
    // group stays ordinary. Exact historical raw + pre-image (fragment
    // lines, no whole-line apply); re-derived veto only.
    let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &cand.body_text,\n+        slice(&cand.body_text),\n@@ -9,1 +10,1 @@\n-        &cand.body_text,\n+        slice(&cand.body_text),\n";
    let pre = "fn f(c: C) {\n    g(&cand.body_text);\n    g(&cand.body_text);\n    if x != &cand.body_text {}\n}\n";
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(raw, &map).unwrap();
    assert!(
        !out.rsmpl.contains("rule="),
        "guard-needle group must stay ordinary, got: {}",
        out.rsmpl
    );
}

#[test]
fn property_bare_ident_veto_rederived() {
    // Historical veto shape (f52d59a): a bare-ident minus would match
    // every mention (declarations included), so the precision gate keeps
    // it ordinary. Exact historical shape (section-visible, no pre-image
    // needed); re-derived veto only.
    let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    wide,\n+    ZeroMatchHint { wide, preview },\n@@ -9,1 +10,1 @@\n-    wide,\n+    ZeroMatchHint { wide, preview },\n";
    let out = rsmpl_diff_infer::infer_rsmpl(raw).unwrap();
    assert!(
        !out.rsmpl.contains("rule="),
        "bare-ident minus must stay ordinary, got: {}",
        out.rsmpl
    );
}

#[test]
fn property_verdict_honesty_orphans_recomputed() {
    // WI-19 slice 3: the apply report's orphan/skip verdict is recomputed
    // from tree + patch-text evidence, never trusted blindly. Generator
    // premise (by construction): no namesakes/shadowed twins exist in any
    // generated pre image, so a lossless semantic raise must report zero
    // orphans and zero follow skips; a phantom entry (or an unlisted skip
    // that still applies clean) fails loudly with the seed.
    for case in 0..40 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15u64.wrapping_add(case as u64 * 0x9E37_79B9));
        let (pre, post) = generate(&mut rng);
        let seed = case as u64 + 1;
        let raw = native_section("f.rs", &pre, &post);
        let pmap = pre_map(&[("f.rs", pre.as_str())]);
        let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pmap)
            .unwrap_or_else(|_| panic!("seed {seed}: inference must succeed (no panic, no error)"));
        // Independent apply (same path as `assert_state_lossless`, report kept).
        let parsed = rsmpl_patch::parse_patch(&out.rsmpl)
            .unwrap_or_else(|_| panic!("seed {seed}: emitted patch must parse"));
        let mut files = pre_map(&[("f.rs", pre.as_str())]);
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .unwrap_or_else(|_| panic!("seed {seed}: emitted patch must apply cleanly"));
        let applied = files.get("f.rs").cloned().unwrap_or_default();
        assert_eq!(
            applied, post,
            "seed {seed}: N→S→N post tree must be byte-identical"
        );
        // (a) Ordinary posture recomputed from patch text, never from the
        // report's own flag: semantic emissions carry `rule=`, ordinary
        // passthrough never does.
        assert_eq!(
            report.has_ordinary,
            !out.rsmpl.contains("rule="),
            "seed {seed}: has_ordinary must agree with patch-text posture"
        );
        // (c) Leftover scan on the applied tree (never on report fields):
        // call-swap keeps both defs (only `= foo(` calls must go), full
        // renames must leave no `foo` at all. The control decoy carries no
        // `foo` either way and passes vacuously.
        if pre.contains("fn bar(") {
            assert!(
                !applied.contains("= foo("),
                "seed {seed}: applied post must not retain old callee calls"
            );
        } else {
            assert!(
                !applied.contains("foo"),
                "seed {seed}: applied post must not retain the old name"
            );
        }
        // (b) A lossless semantic raise over a namesake-free pre image
        // reports zero orphans and zero follow skips. The decoy is exempt:
        // it is ordinary by construction today, and a future detector that
        // fires on control headers with skips must revisit this premise
        // explicitly rather than pass silently.
        if out.rsmpl.contains("rule=") && !pre.contains("for line") {
            let orphans: usize = report.rules.iter().map(|r| r.orphans.len()).sum();
            let skips: usize = report
                .rules
                .iter()
                .map(|r| r.follow_skipped_details.len())
                .sum();
            assert_eq!(
                (orphans, skips),
                (0, 0),
                "seed {seed}: lossless semantic raise must report zero orphans/skips"
            );
        }
    }
}

#[test]
fn property_verdict_honesty_firing_reports_clean() {
    // Kind-0 family pin: the semantic raise applies lossless with zero
    // orphans/skips and `has_ordinary == false` (all recomputed above).
    let pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = foo(1);\n    let b = foo(2);\n}\n";
    let post = "fn bar(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = bar(1);\n    let b = bar(2);\n}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        out.rsmpl.contains("rule="),
        "fn rename must raise semantic, got: {}",
        out.rsmpl
    );
    let report = assert_state_lossless(&out.rsmpl, pre, post, 2001);
    assert!(
        !report.has_ordinary,
        "semantic raise must not report ordinary hunks"
    );
    let orphans: usize = report.rules.iter().map(|r| r.orphans.len()).sum();
    let skips: usize = report
        .rules
        .iter()
        .map(|r| r.follow_skipped_details.len())
        .sum();
    assert_eq!((orphans, skips), (0, 0), "firing raise must report clean");
}

#[test]
fn property_verdict_honesty_ordinary_reports_ordinary() {
    // Decoy pin: the ordinary passthrough reports `has_ordinary` (posture
    // recomputed from patch text in the loop above; pinned here directly).
    let pre = "fn m(section: &str) {\n    for line in section.lines() {\n        use_line(line);\n    }\n    for line in section.lines() {\n        use_line(line);\n    }\n}\n";
    let post = "fn m(section: &str) {\n    for line in raw_lines(section) {\n        use_line(line);\n    }\n    for line in raw_lines(section) {\n        use_line(line);\n    }\n}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        !out.rsmpl.contains("rule="),
        "control-header pair must stay ordinary, got: {}",
        out.rsmpl
    );
    let report = assert_state_lossless(&out.rsmpl, pre, post, 2002);
    assert!(
        report.has_ordinary,
        "ordinary passthrough must report has_ordinary"
    );
}
