//! WI-11 shared parse/probe cache: one parse, memoized probes.
//!
//! The detector chain re-attempts the same pre-image file text and the
//! same probe emissions once per hunk (and per sibling driver: whole-
//! section vs per-hunk, bool `*_holds` vs counted twins, subhunk
//! retries). Without sharing, cost grows with hunks x file size: every
//! totality probe re-parses the full pre-image inside the engine, and
//! every opaque walk re-parses it via `parse_pre`.
//!
//! Two thread-local maps, both living for one `infer` call only
//! (`infer_inner` calls [`reset_all`] at entry, so stale entries can
//! never cross calls):
//! - pre-image `syn::File` parses, keyed by the full pre-text hash
//!   ([`cached_parse_pre`], cloned on hit);
//! - engine probe match counts, keyed by (path, pre-text hash, probe
//!   patch text) ([`run_probe`], `None` = unproven memoized too).
//!
//! Both helpers are behavior-transparent: a hit returns exactly what a
//! miss would compute. Counters ([`probe_stats`], [`pre_stats`]) back
//! the bench shape assertion (probe calls per hunk bounded by a
//! constant) — wall-clock is warn-only, probe counts fail
//! deterministically.
//!
//! Memo keys are full-text `u64` hashes: a collision would return a
//! wrong count, so the key hashes the complete inputs (path + pre +
//! probe), never a prefix.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

fn hash_strs(parts: &[&str]) -> u64 {
    let mut h = DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    h.finish()
}

thread_local! {
    static PRE: RefCell<HashMap<u64, Option<syn::File>>> = RefCell::new(HashMap::new());
    static PROBE: RefCell<HashMap<u64, Option<usize>>> = RefCell::new(HashMap::new());
    static PROBES: Cell<u64> = const { Cell::new(0) };
    static PROBE_HITS: Cell<u64> = const { Cell::new(0) };
    static PROBE_MICROS: Cell<u64> = const { Cell::new(0) };
    static PRE_PARSES: Cell<u64> = const { Cell::new(0) };
    static PRE_HITS: Cell<u64> = const { Cell::new(0) };
}

/// Clear both maps and zero all counters. Called at every `infer_inner`
/// entry so each cache dies with its `infer` call (D5 invalidation law).
pub(crate) fn reset_all() {
    PRE.with(|m| m.borrow_mut().clear());
    PROBE.with(|m| m.borrow_mut().clear());
    PROBES.with(|c| c.set(0));
    PROBE_HITS.with(|c| c.set(0));
    PROBE_MICROS.with(|c| c.set(0));
    PRE_PARSES.with(|c| c.set(0));
    PRE_HITS.with(|c| c.set(0));
}

/// Parse-once pre-image cache: same text parses once per `infer` call,
/// clones on hit. `None` input stays `None` (legacy section-visible
/// allow); unparseable text memoizes as `None` (veto, no re-parse).
pub(crate) fn cached_parse_pre(file_old: Option<&str>) -> Option<syn::File> {
    let text = file_old?;
    let key = hash_strs(&[text]);
    if let Some(hit) = PRE.with(|m| m.borrow().get(&key).cloned()) {
        PRE_HITS.with(|c| c.set(c.get() + 1));
        return hit;
    }
    PRE_PARSES.with(|c| c.set(c.get() + 1));
    let parsed = syn::parse_str::<syn::File>(text).ok();
    PRE.with(|m| m.borrow_mut().insert(key, parsed.clone()));
    parsed
}

/// Memoized engine probe: `parse_patch(probe)` + single-file
/// `apply_patch_to_tree`, returning the summed rule match count
/// (`None` on parse/apply failure, memoized like any count).
pub(crate) fn run_probe(path: &str, pre: &str, probe: &str) -> Option<usize> {
    PROBES.with(|c| c.set(c.get() + 1));
    let key = hash_strs(&[path, pre, probe]);
    if let Some(hit) = PROBE.with(|m| m.borrow().get(&key).copied()) {
        PROBE_HITS.with(|c| c.set(c.get() + 1));
        return hit;
    }
    let start = std::time::Instant::now();
    let out = (|| {
        let parsed = rsmpl_patch::parse_patch(probe).ok()?;
        let mut files = HashMap::new();
        files.insert(path.to_string(), pre.to_string());
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).ok()?;
        Some(report.rules.iter().map(|r| r.matches).sum::<usize>())
    })();
    PROBE_MICROS.with(|c| c.set(c.get() + start.elapsed().as_micros() as u64));
    PROBE.with(|m| m.borrow_mut().insert(key, out));
    out
}

/// (probe calls, probe memo hits) since the last [`reset_all`].
/// Test-only shape-gate reader (no production caller needs counts).
#[cfg(test)]
pub(crate) fn probe_stats() -> (u64, u64) {
    (PROBES.with(|c| c.get()), PROBE_HITS.with(|c| c.get()))
}

/// Microseconds spent inside engine probes (miss path only) since the
/// last [`reset_all`]. WI-11 profile evidence: separates probe cost
/// from chain/shape overhead in the landing numbers. Test-only.
#[cfg(test)]
pub(crate) fn probe_micros() -> u64 {
    PROBE_MICROS.with(|c| c.get())
}

/// (real pre-image parses, parse cache hits) since the last [`reset_all`].
/// Test-only shape-gate reader.
#[cfg(test)]
pub(crate) fn pre_stats() -> (u64, u64) {
    (PRE_PARSES.with(|c| c.get()), PRE_HITS.with(|c| c.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_parse_memoizes_same_text() {
        reset_all();
        let text = "fn f() {}\n";
        assert!(cached_parse_pre(Some(text)).is_some());
        assert!(cached_parse_pre(Some(text)).is_some());
        let (parses, hits) = pre_stats();
        assert_eq!((parses, hits), (1, 1), "one parse, one hit");
        // A different text parses again; unparseable text memoizes None.
        assert!(cached_parse_pre(Some("fn g() {}\n")).is_some());
        assert!(cached_parse_pre(Some("this is not rust {{{\n")).is_none());
        assert!(cached_parse_pre(Some("this is not rust {{{\n")).is_none());
        let (parses, hits) = pre_stats();
        assert_eq!((parses, hits), (3, 2), "bad text parses once, hits twice");
        assert!(cached_parse_pre(None).is_none());
    }

    #[test]
    fn probe_memoizes_same_emission() {
        reset_all();
        let pre = "fn f() {\n    foo(1);\n}\n";
        let probe = format!(
            "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n- foo(E0)\n+ foo(E0)\n",
            crate::git_header("f.rs")
        );
        assert_eq!(run_probe("f.rs", pre, &probe), Some(1));
        assert_eq!(run_probe("f.rs", pre, &probe), Some(1));
        let (probes, hits) = probe_stats();
        assert_eq!((probes, hits), (2, 1), "second identical probe hits");
        // A different emission probes again; a bad probe memoizes None.
        let other = probe.replace("foo(E0)", "bar(E0)");
        assert_eq!(run_probe("f.rs", pre, &other), Some(0));
        assert_eq!(run_probe("f.rs", pre, "not a patch"), None);
        assert_eq!(run_probe("f.rs", pre, "not a patch"), None);
        let (probes, hits) = probe_stats();
        assert_eq!((probes, hits), (5, 2));
    }

    #[test]
    fn reset_clears_maps_and_counters() {
        reset_all();
        let text = "fn f() {}\n";
        assert!(cached_parse_pre(Some(text)).is_some());
        let pre = "fn f() {\n    foo(1);\n}\n";
        let probe = format!(
            "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n- foo(E0)\n+ foo(E0)\n",
            crate::git_header("f.rs")
        );
        assert_eq!(run_probe("f.rs", pre, &probe), Some(1));
        reset_all();
        assert_eq!(probe_stats(), (0, 0));
        assert_eq!(pre_stats(), (0, 0));
        // Entries are gone: the same inputs parse and probe again.
        assert!(cached_parse_pre(Some(text)).is_some());
        assert_eq!(run_probe("f.rs", pre, &probe), Some(1));
        assert_eq!(probe_stats(), (1, 0));
        assert_eq!(pre_stats(), (1, 0));
        reset_all();
    }

    /// WI-11 shape corpus: the `gen_scale.py` deep shape (300 fns, every
    /// 6th edited, call / let-init / header-comment cycle = 50 hunks),
    /// synthesized in-test with no git checkout.
    fn deep_corpus(items: usize, every: usize) -> (String, String) {
        fn item(i: usize, edited: Option<&str>) -> String {
            match edited {
                Some("call") => format!(
                    "fn f{i:03}(x: i32) -> i32 {{\n    let v{i:03} = x + {i};\n    bar(v{i:03})\n}}\n"
                ),
                Some("let") => format!(
                    "fn f{i:03}(x: i32) -> i32 {{\n    let v{i:03} = x + {};\n    foo(v{i:03})\n}}\n",
                    i + 1000
                ),
                Some("comment") => format!(
                    "fn f{i:03}(x: i32) -> i32 {{ // tweaked {i}\n    let v{i:03} = x + {i};\n    foo(v{i:03})\n}}\n"
                ),
                _ => format!(
                    "fn f{i:03}(x: i32) -> i32 {{\n    let v{i:03} = x + {i};\n    foo(v{i:03})\n}}\n"
                ),
            }
        }
        let mut pre = String::new();
        let mut diff = String::from(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n",
        );
        for i in 0..items {
            pre.push_str(&item(i, None));
            if i % every != 0 {
                continue;
            }
            let kind = ["call", "let", "comment"][(i / every) % 3];
            let base = i * 4; // 4 lines per fn, 1-based below.
            let (old_line, minus, plus) = match kind {
                "call" => (
                    base + 3,
                    format!("    foo(v{i:03})"),
                    format!("    bar(v{i:03})"),
                ),
                "let" => (
                    base + 2,
                    format!("    let v{i:03} = x + {i};"),
                    format!("    let v{i:03} = x + {};", i + 1000),
                ),
                _ => (
                    base + 1,
                    format!("fn f{i:03}(x: i32) -> i32 {{"),
                    format!("fn f{i:03}(x: i32) -> i32 {{ // tweaked {i}"),
                ),
            };
            diff.push_str(&format!(
                "@@ -{old_line},1 +{old_line},1 @@\n-{minus}\n+{plus}\n"
            ));
        }
        (pre, diff)
    }

    fn infer_counted(pre: &str, diff: &str) -> (String, u64, u64, u64, usize) {
        reset_all();
        let start = std::time::Instant::now();
        let mut map = HashMap::new();
        map.insert("src/lib.rs".to_string(), pre.to_string());
        let out = crate::infer_rsmpl_with_pre_images(diff, &map).expect("infer must succeed");
        let total_ms = start.elapsed().as_millis() as u64;
        let (probes, _) = probe_stats();
        let hunks = diff.lines().filter(|l| l.starts_with("@@")).count();
        (out.rsmpl, probes, probe_micros() / 1000, total_ms, hunks)
    }

    #[test]
    fn deep_raise_probes_bounded_per_hunk() {
        // The WI-11 shape gate: probe calls grow with hunks, never with
        // hunks x file size. K=4/hunk holds ~12x headroom over the
        // measured ~0.34 probes/hunk at landing (50 hunks, 300-item
        // file); removing the cache must fail this test (mutation
        // check in REVIEW).
        let (pre, diff) = deep_corpus(300, 6);
        let (rsmpl, probes, probe_ms, total_ms, hunks) = infer_counted(&pre, &diff);
        assert_eq!(hunks, 50, "corpus must carry 50 hunks");
        assert!(
            rsmpl.contains("rule="),
            "deep corpus must raise, got: {}",
            &rsmpl[..rsmpl.len().min(400)]
        );
        rsmpl_patch::parse_patch(&rsmpl).expect("emission must parse");
        eprintln!(
            "WI-11 profile (300 items / 50 hunks): probes={probes} probe_ms={probe_ms} total_ms={total_ms}"
        );
        let bound = 4 * hunks as u64;
        assert!(
            probes <= bound,
            "probes {probes} exceed {bound} (4/hunk over {hunks} hunks)"
        );
    }

    #[test]
    fn deep_raise_probes_scale_linearly() {
        // Doubled corpus stays linear by probe count with the same K:
        // 12 and 24 hunks over the same 300-item file must both fit
        // 4/hunk (a quadratic probe loop would separate them).
        for (items, every, want_hunks) in [(300usize, 25usize, 12usize), (300, 12, 25)] {
            let (pre, diff) = deep_corpus(items, every);
            let (_, probes, probe_ms, total_ms, hunks) = infer_counted(&pre, &diff);
            let (_, hits) = probe_stats();
            eprintln!(
                "WI-11 linearity ({items} items / {hunks} hunks): probes={probes} hits={hits} probe_ms={probe_ms} total_ms={total_ms}"
            );
            assert_eq!(hunks, want_hunks);
            let bound = 4 * hunks as u64;
            assert!(
                probes <= bound,
                "probes {probes} exceed {bound} ({hunks} hunks)"
            );
        }
    }

    #[test]
    fn deep_raise_pre_image_parses_once_per_file() {
        // The pre-image text parses a handful of times per `infer` call
        // (one cached parse shared by every detector and hunk), never
        // once per hunk x detector.
        let (pre, diff) = deep_corpus(300, 6);
        reset_all();
        let mut map = HashMap::new();
        map.insert("src/lib.rs".to_string(), pre.clone());
        crate::infer_rsmpl_with_pre_images(&diff, &map).expect("infer must succeed");
        let (parses, hits) = pre_stats();
        eprintln!("WI-11 pre-parse: parses={parses} hits={hits}");
        assert!(
            parses <= 3,
            "pre-image parsed {parses}x for one file (want <= 3)"
        );
    }
}
