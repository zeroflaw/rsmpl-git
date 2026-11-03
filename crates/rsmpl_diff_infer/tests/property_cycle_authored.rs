//! WI-19 slice 2: authored-side property harness (Generator A).
//!
//! Random authored RSmPL patches drawn from the SPEC 18.6
//! supported-shape grammar are applied via the public
//! `apply_patch_to_tree` entry point and must satisfy the authored-side
//! cycle invariants (SPEC D.5):
//! (1) parse + apply return `Result` (no panic, ever — seed printed);
//! (2) Ok implies a second apply to a fresh pre map yields the
//! byte-identical post map (determinism / fixed point);
//! (3) Ok on firing shapes implies the post equals an independently
//! recomputed expected text (state-losslessness authored side, never by
//! reading the report's own fields);
//! (4) Err implies a typed `StructuredError` code in the known SPEC 25
//! set (verdict-honesty lite — error taxonomy, not orphan counts).
//!
//! Random cases assert only the invariants — never posture beyond the
//! per-kind contract below (firing kinds must Ok with the expected text;
//! veto kinds must Err, never Ok, never panic). Seeded xorshift64, no
//! new dependencies. Slice 2 is authored-side only; raising-side orphan
//! recomputation and the wide `tools/` run are later slices.

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

/// The known SPEC 25 error-code set (verdict-honesty lite: every Err must
/// carry one of these, never a panic, never an untyped string).
fn known_code(code: &str) -> bool {
    matches!(
        code,
        "RSMPL_RANGE_CONFLICT"
            | "RSMPL_PARSE_CONFLICT"
            | "RSMPL_PATTERN_CONFLICT"
            | "RSMPL_MATCH_CONFLICT"
            | "RSMPL_SYMBOL_CONFLICT"
            | "RSMPL_CAPTURE_CONFLICT"
            | "GIT_HUNK_CONFLICT"
            | "RSMPL_BUILD_CONFLICT"
            | "RSMPL_IO_CONFLICT"
    )
}

fn assert_known_err(err: &rsmpl_patch::StructuredError, seed: u64, what: &str) {
    let code = err.code.as_str();
    assert!(
        known_code(code),
        "seed {seed}: {what} Err must carry a known SPEC 25 code, got: {code} ({err})"
    );
}

/// Apply `patch_text` to `pre` (single file `f.rs`), returning the post
/// map on success. Panics with the seed on any internal panic path — the
/// harness has no `catch_unwind`: a panic fails the test loudly.
#[allow(clippy::result_large_err)]
fn apply_once(
    patch_text: &str,
    pre: &str,
    seed: u64,
) -> Result<HashMap<String, String>, rsmpl_patch::StructuredError> {
    let parsed = rsmpl_patch::parse_patch(patch_text).unwrap_or_else(|e| {
        panic!("seed {seed}: authored patch must parse or fail typed, got: {e}")
    });
    let mut files = pre_map(&[("f.rs", pre)]);
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    match rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) {
        Ok(_) => Ok(files),
        Err(e) => Err(e),
    }
}

const BASE_FN: &str = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = foo(1);\n    let b = foo(2);\n}\n";
const BOX_FN: &str = "fn m() {\n    let v = Box::new(Box::new(1));\n}\n";

fn expected_fn_rename(pre: &str) -> String {
    pre.replacen("fn foo(", "fn bar(", 1)
}

fn expected_call_swap(pre: &str) -> String {
    pre.replace("= foo(", "= bar(")
}

/// Generate one (pre, patch, kind) triple. Kinds:
/// 0 = fn rename terse (fires, def-only);
/// 1 = call sweep `# sweep` (fires, calls only);
/// 2 = `# optional` zero-match (Ok, unchanged);
/// 3 = `# repeat` nested fold (fires);
/// 4 = stale range + `# sweep` (Ok-with-expected OR typed Err);
/// 5 = overlapping hunks (typed Err, never Ok).
fn generate(rng: &mut Rng) -> (String, String, usize) {
    let kind = rng.below(6);
    match kind {
        0 => (
            BASE_FN.to_string(),
            "diff --git f.rs\n@@ -1,1 @@ rule=r0\n# sweep\n- fn foo\n+ fn bar\n".to_string(),
            0,
        ),
        1 => (
            BASE_FN.to_string(),
            "diff --git f.rs\n@@ -1,1 @@ rule=r1\n# sweep\n- foo(E)\n+ bar(E)\n".to_string(),
            1,
        ),
        2 => (
            BASE_FN.to_string(),
            "diff --git f.rs\n@@ -1,1 @@ rule=r2\n# sweep\n# optional\n- missing_callee(E)\n+ other_callee(E)\n".to_string(),
            2,
        ),
        3 => (
            BOX_FN.to_string(),
            "diff --git f.rs\n@@ -1,1 @@ rule=r3\n# sweep\n# repeat\n- Box::new(E)\n+ E\n".to_string(),
            3,
        ),
        4 => (
            BASE_FN.to_string(),
            "diff --git f.rs\n@@ -50,3 @@ rule=r4\n# sweep\n- fn foo\n+ fn bar\n".to_string(),
            4,
        ),
        _ => (
            BASE_FN.to_string(),
            "diff --git f.rs\n@@ -1,1 @@ rule=r5a\n# sweep\n- fn foo\n+ fn bar\n@@ -1,1 @@ rule=r5b\n# sweep\n- fn foo\n+ fn baz\n"
                .to_string(),
            5,
        ),
    }
}

#[test]
fn property_authored_side_cycle_holds() {
    // 120 seeded cases; case count (not wall-clock) is the budget gate.
    for case in 0..120 {
        let mut rng = Rng(0x51ED_1E5Bu64.wrapping_add(case as u64 * 0x9E37_79B9));
        let (pre, patch_text, kind) = generate(&mut rng);
        let seed = case as u64 + 1;
        let first = apply_once(&patch_text, &pre, seed);
        match kind {
            0 => {
                let files = first.unwrap_or_else(|e| {
                    panic!("seed {seed}: fn-rename kind must Ok, got Err: {e}")
                });
                let expected = expected_fn_rename(&pre);
                assert_eq!(
                    files.get("f.rs").map(String::as_str),
                    Some(expected.as_str()),
                    "seed {seed}: fn-rename post must equal independent recomputation"
                );
                // Determinism: second apply to a fresh pre is identical.
                let second = apply_once(&patch_text, &pre, seed)
                    .unwrap_or_else(|e| panic!("seed {seed}: second apply must Ok, got: {e}"));
                assert_eq!(
                    files, second,
                    "seed {seed}: second apply must be byte-identical"
                );
            }
            1 => {
                let files = first.unwrap_or_else(|e| {
                    panic!("seed {seed}: call-sweep kind must Ok, got Err: {e}")
                });
                let expected = expected_call_swap(&pre);
                assert_eq!(
                    files.get("f.rs").map(String::as_str),
                    Some(expected.as_str()),
                    "seed {seed}: call-sweep post must equal independent recomputation"
                );
                let second = apply_once(&patch_text, &pre, seed)
                    .unwrap_or_else(|e| panic!("seed {seed}: second apply must Ok, got: {e}"));
                assert_eq!(
                    files, second,
                    "seed {seed}: second apply must be byte-identical"
                );
            }
            2 => {
                let files = first.unwrap_or_else(|e| {
                    panic!("seed {seed}: optional-zero kind must Ok, got Err: {e}")
                });
                assert_eq!(
                    files.get("f.rs").map(String::as_str),
                    Some(pre.as_str()),
                    "seed {seed}: optional zero-match must leave the tree unchanged"
                );
                let second = apply_once(&patch_text, &pre, seed)
                    .unwrap_or_else(|e| panic!("seed {seed}: second apply must Ok, got: {e}"));
                assert_eq!(
                    files, second,
                    "seed {seed}: second apply must be byte-identical"
                );
            }
            3 => {
                let files = first.unwrap_or_else(|e| {
                    panic!("seed {seed}: repeat-fold kind must Ok, got Err: {e}")
                });
                let expected = "fn m() {\n    let v = 1;\n}\n";
                assert_eq!(
                    files.get("f.rs").map(String::as_str),
                    Some(expected),
                    "seed {seed}: repeat fold must collapse both nest levels"
                );
                let second = apply_once(&patch_text, &pre, seed)
                    .unwrap_or_else(|e| panic!("seed {seed}: second apply must Ok, got: {e}"));
                assert_eq!(
                    files, second,
                    "seed {seed}: second apply must be byte-identical"
                );
            }
            4 => {
                // Stale range: Ok-with-expected OR typed Err — never panic,
                // and either outcome must be deterministic.
                match first {
                    Ok(files) => {
                        let expected = expected_fn_rename(&pre);
                        assert_eq!(
                            files.get("f.rs").map(String::as_str),
                            Some(expected.as_str()),
                            "seed {seed}: stale-range Ok must still reach the expected post"
                        );
                        let second = apply_once(&patch_text, &pre, seed).unwrap_or_else(|e| {
                            panic!("seed {seed}: stale Ok must be deterministic, got Err: {e}")
                        });
                        assert_eq!(
                            files, second,
                            "seed {seed}: stale-range Ok must be deterministic"
                        );
                    }
                    Err(e) => {
                        assert_known_err(&e, seed, "stale-range");
                        let second = apply_once(&patch_text, &pre, seed);
                        assert!(
                            second.is_err(),
                            "seed {seed}: stale-range Err must be deterministic (got Ok)"
                        );
                    }
                }
            }
            _ => {
                // Overlapping hunks: typed Err, never Ok, never panic.
                match first {
                    Ok(_) => panic!("seed {seed}: overlapping hunks must Err, got Ok"),
                    Err(e) => assert_known_err(&e, seed, "overlap"),
                }
                let second = apply_once(&patch_text, &pre, seed);
                assert!(
                    second.is_err(),
                    "seed {seed}: overlap veto must be deterministic (got Ok)"
                );
            }
        }
    }
}

#[test]
fn property_authored_fn_rename_fires() {
    let pre = BASE_FN;
    let patch = "diff --git f.rs\n@@ -1,1 @@ rule=rename_foo\n# sweep\n- fn foo\n+ fn bar\n";
    let files =
        apply_once(patch, pre, 1001).unwrap_or_else(|e| panic!("fn rename must apply, got: {e}"));
    assert_eq!(
        files.get("f.rs").map(String::as_str),
        Some(expected_fn_rename(pre).as_str())
    );
}

#[test]
fn property_authored_call_sweep_fires() {
    let pre = BASE_FN;
    let patch = "diff --git f.rs\n@@ -1,1 @@ rule=call_swap\n# sweep\n- foo(E)\n+ bar(E)\n";
    let files =
        apply_once(patch, pre, 1002).unwrap_or_else(|e| panic!("call sweep must apply, got: {e}"));
    assert_eq!(
        files.get("f.rs").map(String::as_str),
        Some(expected_call_swap(pre).as_str())
    );
}

#[test]
fn property_authored_optional_zero_ok_unchanged() {
    let pre = BASE_FN;
    let patch = "diff --git f.rs\n@@ -1,1 @@ rule=opt_zero\n# sweep\n# optional\n- missing_callee(E)\n+ other_callee(E)\n";
    let files = apply_once(patch, pre, 1003)
        .unwrap_or_else(|e| panic!("optional zero-match must Ok, got: {e}"));
    assert_eq!(files.get("f.rs").map(String::as_str), Some(pre));
}

#[test]
fn property_authored_repeat_fold_fires() {
    let pre = BOX_FN;
    let patch = "diff --git f.rs\n@@ -1,1 @@ rule=unbox\n# sweep\n# repeat\n- Box::new(E)\n+ E\n";
    let files =
        apply_once(patch, pre, 1004).unwrap_or_else(|e| panic!("repeat fold must apply, got: {e}"));
    assert_eq!(
        files.get("f.rs").map(String::as_str),
        Some("fn m() {\n    let v = 1;\n}\n")
    );
}

#[test]
fn property_authored_overlap_vetoes() {
    let pre = BASE_FN;
    let patch = "diff --git f.rs\n@@ -1,1 @@ rule=ov_a\n# sweep\n- fn foo\n+ fn bar\n@@ -1,1 @@ rule=ov_b\n# sweep\n- fn foo\n+ fn baz\n";
    match apply_once(patch, pre, 1005) {
        Ok(_) => panic!("overlapping hunks must veto, got Ok"),
        Err(e) => assert_known_err(&e, 1005, "overlap pin"),
    }
}
