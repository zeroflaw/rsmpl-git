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
//! Scope note: hetero general-expr sweeps whose templates carry
//! `&`/`field` anchors replay source bytes verbatim (WI-3 span-slice
//! law) — `&cand.body_text` stays `&cand.body_text`, never
//! `& cand . body_text` — so these shapes generate, raise, and
//! re-apply byte-identical like every other kind below.
//!
//! WI-12 residue A: single-region unbalanced pairs (insertion-only,
//! deletion-only) generate through `native_unbalanced`, and the joint
//! rename-plus-body shape is pinned below via `native_hunks`.

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
/// Build a second U0 native section from explicit hunks (WI-12 residue A).
/// Each entry holds old/new starts plus minus/plus lines. Line-preserving
/// pairs go through native_section above; unbalanced and multi-region
/// pairs call the builder below directly.
#[allow(clippy::type_complexity)] // test-only 6-tuple hunk rows
fn native_hunks(
    path: &str,
    hunks: &[(usize, usize, usize, usize, Vec<String>, Vec<String>)],
) -> String {
    let mut body = String::new();
    for (o, ol, n, nl, minus, plus) in hunks {
        body.push_str(&format!("@@ -{o},{ol} +{n},{nl} @@\n"));
        for m in minus {
            body.push_str(&format!("-{m}\n"));
        }
        for p in plus {
            body.push_str(&format!("+{p}\n"));
        }
    }
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
}

/// Single-region unbalanced U0 section (WI-12 residue A): prefix/suffix
/// split of the pre/post lines with one hunk for the differing middle.
/// Pure insertions emit an old-side zero length, pure deletions a
/// new-side zero length.
fn native_unbalanced(path: &str, pre: &str, post: &str) -> String {
    let pre_lines: Vec<&str> = pre.split('\n').collect();
    let post_lines: Vec<&str> = post.split('\n').collect();
    let mut prefix = 0;
    while prefix < pre_lines.len()
        && prefix < post_lines.len()
        && pre_lines[prefix] == post_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < pre_lines.len() - prefix
        && suffix < post_lines.len() - prefix
        && pre_lines[pre_lines.len() - 1 - suffix] == post_lines[post_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let pre_mid = &pre_lines[prefix..pre_lines.len() - suffix];
    let post_mid = &post_lines[prefix..post_lines.len() - suffix];
    assert!(
        !pre_mid.is_empty() || !post_mid.is_empty(),
        "unbalanced pair must differ"
    );
    // Zero-length sides anchor *after* their preceding line: a pure
    // insertion rides old-side `prefix,0` (after the common prefix), a
    // pure deletion rides new-side `prefix,0`; replacements start low.
    let old_at = if pre_mid.is_empty() {
        prefix
    } else {
        prefix + 1
    };
    let new_at = if post_mid.is_empty() {
        prefix
    } else {
        prefix + 1
    };
    native_hunks(
        path,
        &[(
            old_at,
            pre_mid.len(),
            new_at,
            post_mid.len(),
            pre_mid.iter().map(|s| s.to_string()).collect(),
            post_mid.iter().map(|s| s.to_string()).collect(),
        )],
    )
}

/// Raw builder dispatch (WI-12 residue A): line-preserving pairs keep
/// the per-line section builder, unbalanced pairs use the split above.
fn raw_for(path: &str, pre: &str, post: &str) -> String {
    if pre.split('\n').count() == post.split('\n').count() {
        native_section(path, pre, post)
    } else {
        native_unbalanced(path, pre, post)
    }
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

/// Generate one (pre, post) pair from `rng`. Kinds:
/// 0 = fn rename `foo`→`bar` (def + call sites follow),
/// 1 = call callee swap `foo`→`bar` across call sites only,
/// 2 = control-header decoy (R1: for-loop headers match no engine
/// shape — stays ordinary, still round-trips),
/// 3 = N=1 fn rename (single call site; fires or stays ordinary under
/// the precision guard — either posture round-trips),
/// 4 = WI-3 let-init reference-anchor sweep (hetero bindings, uniform
/// `&a.b` → `&a.c` init anchor — the template carries `&`/`.` and must
/// replay source bytes),
/// 5 = WI-3 binary field-anchor sweep (hetero bases ride `E0`, uniform
/// `&cand.body_text` → `&cand.value_text` anchor delta replays source
/// bytes on both sides),
/// 6 = WI-12 method-to-method sweep (`r.old_m(a)` → `r.new_m(a)`),
/// 7 = WI-12 method-to-function refactor (`r.old_m(a)` →
/// `new_f(r, a)`, receiver-prepend delta),
/// 8 = WI-12 qualified-call sweep (`old::foo(a)` → `new::foo(a)`),
/// 9 = WI-12 use-prefix sweep (`use crate::old::A;` →
/// `use crate::new::A;`, 2 use lines),
/// 10 = WI-12 macro rename (`old_m!(a)` → `new_m!(a)`),
/// 11 = WI-12 turbofish callee sweep (`foo::<u8>(a)` →
/// `bar::<u8>(a)`, same turbofish args),
/// 12 = WI-12 header-evolution sweep (2 fn headers, uniform param-type
/// delta `i32` -> `u64` over hetero names),
/// 13 = WI-12 type-annotation sweep (`let v: Vec<i32>` ->
/// `let v: Box<[i32]>`, same init),
/// 14 = WI-12 residue A insertion-only (one `let added` line appears;
/// L-insertion stays ordinary, still round-trips),
/// 15 = WI-12 residue A deletion-only (one `let dead` line disappears;
/// same ordinary posture as insertion).
fn generate(rng: &mut Rng) -> (String, String) {
    let kind = rng.below(16);
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
        _ if kind == 3 => {
            let mut pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n".to_string();
            let mut post = "fn bar(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n".to_string();
            pre.push_str("    let only = foo(1);\n}\n");
            post.push_str("    let only = bar(1);\n}\n");
            (pre, post)
        }
        _ if kind == 4 => {
            let mut pre = "fn m() {\n".to_string();
            let mut post = "fn m() {\n".to_string();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i} = &a.b;\n"));
                post.push_str(&format!("    let v{i} = &a.c;\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ if kind == 5 => {
            let mut pre = "fn m() {\n".to_string();
            let mut post = "fn m() {\n".to_string();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i} = base{i} + &cand.body_text;\n"));
                post.push_str(&format!("    let v{i} = base{i} + &cand.value_text;\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ if kind == 6 => {
            let mut pre =
                "struct R;\nimpl R {\n    fn old_m(&self, x: i32) -> i32 {\n        x\n    }\n}\nfn m(r: R) {\n"
                    .to_string();
            let mut post = pre.clone();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i} = r.old_m({i});\n"));
                post.push_str(&format!("    let v{i} = r.new_m({i});\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ if kind == 7 => {
            let mut pre =
                "struct R;\nimpl R {\n    fn old_m(&self, x: i32) -> i32 {\n        x\n    }\n}\nfn new_f(r: R, x: i32) -> i32 {\n    x\n}\nfn m(r: R) {\n"
                    .to_string();
            let mut post = pre.clone();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i} = r.old_m({i});\n"));
                post.push_str(&format!("    let v{i} = new_f(r, {i});\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ if kind == 8 => {
            let mut pre =
                "mod old {\n    pub fn foo(x: i32) -> i32 {\n        x\n    }\n}\nmod new {\n    pub fn foo(x: i32) -> i32 {\n        x\n    }\n}\nfn m() {\n"
                    .to_string();
            let mut post = pre.clone();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i} = old::foo({i});\n"));
                post.push_str(&format!("    let v{i} = new::foo({i});\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ if kind == 9 => (
            "use crate::old::Thing;\nuse crate::old::Other;\nfn m() {}\n".to_string(),
            "use crate::new::Thing;\nuse crate::new::Other;\nfn m() {}\n".to_string(),
        ),
        _ if kind == 10 => {
            let mut pre = "fn m() {\n".to_string();
            let mut post = "fn m() {\n".to_string();
            for i in 0..ncalls {
                pre.push_str(&format!("    old_m!({i});\n"));
                post.push_str(&format!("    new_m!({i});\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ if kind == 11 => {
            let mut pre =
                "fn foo<T>(x: T) -> T {\n    x\n}\nfn bar<T>(x: T) -> T {\n    x\n}\nfn m() {\n"
                    .to_string();
            let mut post = pre.clone();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i} = foo::<u8>({i});\n"));
                post.push_str(&format!("    let v{i} = bar::<u8>({i});\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ if kind == 12 => (
            "fn f0(x: i32) -> i32 {\n    x + 1\n}\nfn f1(y: i32) -> i32 {\n    y + 2\n}\n"
                .to_string(),
            "fn f0(x: u64) -> i32 {\n    x + 1\n}\nfn f1(y: u64) -> i32 {\n    y + 2\n}\n"
                .to_string(),
        ),
        _ if kind == 13 => {
            let mut pre = "fn m() {\n".to_string();
            let mut post = "fn m() {\n".to_string();
            for i in 0..ncalls {
                pre.push_str(&format!("    let v{i}: Vec<i32> = Vec::new();\n"));
                post.push_str(&format!("    let v{i}: Box<[i32]> = Vec::new();\n"));
            }
            pre.push_str("}\n");
            post.push_str("}\n");
            (pre, post)
        }
        _ if kind == 14 => (
            "fn m() {\n    let keep = 1;\n}\n".to_string(),
            "fn m() {\n    let keep = 1;\n    let added = 2;\n}\n".to_string(),
        ),
        _ => (
            "fn m() {\n    let keep = 1;\n    let dead = 0;\n}\n".to_string(),
            "fn m() {\n    let keep = 1;\n}\n".to_string(),
        ),
    }
}

#[test]
fn property_native_side_cycle_holds() {
    // 120 seeded cases over 16 kinds (not wall-clock); the count is the gate.
    for case in 0..120 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15u64.wrapping_add(case as u64 * 0x9E37_79B9));
        let (pre, post) = generate(&mut rng);
        let seed = case as u64 + 1;
        let raw = raw_for("f.rs", &pre, &post);
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
fn property_ref_anchor_sweep_fires_and_round_trips() {
    // WI-3 historical firing shape (Generator B kind-4 family):
    // hetero bindings, uniform `&a.b` → `&a.c` init anchor raises
    // semantic with verbatim spacing and applies byte-exact.
    let pre = "fn m() {\n    let v0 = &a.b;\n    let v1 = &a.b;\n}\n";
    let post = "fn m() {\n    let v0 = &a.c;\n    let v1 = &a.c;\n}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        out.rsmpl.contains("rule="),
        "ref-anchor sweep must raise semantic, got: {}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("- &a.b\n"),
        "verbatim anchor, got: {}",
        out.rsmpl
    );
    assert_state_lossless(&out.rsmpl, pre, post, 1004);
}

#[test]
fn property_field_anchor_sweep_fires_and_round_trips() {
    // WI-3 historical firing shape (Generator B kind-5 family):
    // hetero bases ride `E0`, the uniform `&cand.body_text` →
    // `&cand.value_text` anchor delta replays source bytes.
    let pre = "fn m() {\n    let v0 = base0 + &cand.body_text;\n    let v1 = base1 + &cand.body_text;\n}\n";
    let post = "fn m() {\n    let v0 = base0 + &cand.value_text;\n    let v1 = base1 + &cand.value_text;\n}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        out.rsmpl.contains("rule="),
        "field-anchor sweep must raise semantic, got: {}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("- E0 + &cand.body_text\n"),
        "verbatim anchor, got: {}",
        out.rsmpl
    );
    assert_state_lossless(&out.rsmpl, pre, post, 1005);
}

#[test]
fn property_method_sweep_fires_and_round_trips() {
    // WI-12 historical firing shape (Generator B kind-6 family):
    // method-to-method rename across sites raises a method sweep and
    // applies byte-exact.
    let pre = "struct R;\nimpl R {\n    fn old_m(&self, x: i32) -> i32 {\n        x\n    }\n}\nfn m(r: R) {\n    let a = r.old_m(1);\n    let b = r.old_m(2);\n}\n";
    let post = "struct R;\nimpl R {\n    fn old_m(&self, x: i32) -> i32 {\n        x\n    }\n}\nfn m(r: R) {\n    let a = r.new_m(1);\n    let b = r.new_m(2);\n}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        out.rsmpl.contains("rule="),
        "method rename must raise semantic, got: {}",
        out.rsmpl
    );
    assert_state_lossless(&out.rsmpl, pre, post, 1006);
}

#[test]
fn property_use_prefix_sweep_fires_and_round_trips() {
    // WI-12 historical firing shape (Generator B kind-9 family):
    // use-prefix sweep raises semantic and applies byte-exact.
    let pre = "use crate::old::Thing;\nuse crate::old::Other;\nfn m() {}\n";
    let post = "use crate::new::Thing;\nuse crate::new::Other;\nfn m() {}\n";
    let raw = native_section("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        out.rsmpl.contains("rule="),
        "use-prefix sweep must raise semantic, got: {}",
        out.rsmpl
    );
    assert_state_lossless(&out.rsmpl, pre, post, 1007);
}

#[test]
fn property_insertion_stays_ordinary_and_round_trips() {
    // WI-12 residue A pin (Generator B kind-14 family): a single-region
    // insertion-only pair raises ordinary per law L-insertion and the
    // passthrough still round-trips byte-exact.
    let pre = "fn m() {\n    let keep = 1;\n}\n";
    let post = "fn m() {\n    let keep = 1;\n    let added = 2;\n}\n";
    let raw = native_unbalanced("f.rs", pre, post);
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert!(
        !out.rsmpl.contains("rule="),
        "insertion-only pair must stay ordinary, got: {}",
        out.rsmpl
    );
    assert_state_lossless(&out.rsmpl, pre, post, 1008);
}

#[test]
fn property_joint_rename_plus_body_round_trips() {
    // WI-12 residue A joint probe: fn rename plus body delta in one diff,
    // two regions through the multi-hunk builder. Either posture is
    // legitimate; the pin asserts lossless apply plus the fixed point.
    let pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = foo(1);\n    let b = foo(2);\n}\n";
    let post = "fn bar(x: i32) -> i32 {\n    x + 2\n}\nfn m() {\n    let a = bar(1);\n    let b = bar(2);\n}\n";
    let raw = native_hunks(
        "f.rs",
        &[
            (
                1,
                2,
                1,
                2,
                vec![
                    "fn foo(x: i32) -> i32 {".to_string(),
                    "    x + 1".to_string(),
                ],
                vec![
                    "fn bar(x: i32) -> i32 {".to_string(),
                    "    x + 2".to_string(),
                ],
            ),
            (
                5,
                2,
                5,
                2,
                vec![
                    "    let a = foo(1);".to_string(),
                    "    let b = foo(2);".to_string(),
                ],
                vec![
                    "    let a = bar(1);".to_string(),
                    "    let b = bar(2);".to_string(),
                ],
            ),
        ],
    );
    let map = pre_map(&[("f.rs", pre)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert_state_lossless(&out.rsmpl, pre, post, 1009);
    let out2 = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &map).unwrap();
    assert_eq!(
        out2.rsmpl, out.rsmpl,
        "joint probe second raise must be byte-identical"
    );
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
    // 64 cases = 4 per kind over 16 kinds (WI-12 residue A budget).
    for case in 0..64 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15u64.wrapping_add(case as u64 * 0x9E37_79B9));
        let (pre, post) = generate(&mut rng);
        let seed = case as u64 + 1;
        let raw = raw_for("f.rs", &pre, &post);
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
        // call-swap keeps both defs (only `= foo(` calls must go;
        // turbofish twins likewise keep both generic defs, so `foo::<`
        // calls must go too); qualified-call twins keep both module defs
        // (only `old::foo(` calls must go); full renames must leave no
        // `foo` at all. The control decoy carries no `foo` either way
        // and passes vacuously, as do the method/macro/use/sig/type
        // families (no `foo` in their pre images).
        if pre.contains("mod new") {
            assert!(
                !applied.contains("old::foo("),
                "seed {seed}: applied post must not retain old qualified calls"
            );
        } else if pre.contains("fn bar") {
            assert!(
                !applied.contains("= foo("),
                "seed {seed}: applied post must not retain old callee calls"
            );
            assert!(
                !applied.contains("foo::<"),
                "seed {seed}: applied post must not retain old turbofish calls"
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
