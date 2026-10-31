//! T433 fixture-driven diffrs full cycle, part 2 (fixtures `45` and up).
//!
//! Same six-step loop as part 1 (harness in `common`, `run_all`); this
//! part supplies the shape pins for fixtures `45`–`60`. Fixtures `61+`
//! ride the generic round-trip with no shape pins here — their fires
//! and vetoes are pinned in `wi3_sweeps.rs`, `wi4_sweeps.rs`,
//! `wi5_sweeps.rs`, and `wi9_sweeps.rs`.
//!
//! Split note (WI-9-pre): part 2 owns all future growth — new fixtures
//! (`87+`) gain their pin arms here, never in part 1 (whose `01`–`44`
//! range is closed).
//!
//! Requires a workspace build so the sibling `applyrs` binary sits next
//! to `diffrs` (`cargo test --workspace`).

mod common;

use common::{fixture_num, normalize_native, run_all, PART_BOUNDARY};

#[test]
fn native_to_semantic_to_native_round_trip_part2() {
    run_all("part2", |n| fixture_num(n) >= PART_BOUNDARY, pins_late);
}

fn pins_late(name: &str, native_input: &str, semantic: &str) {
    if name == "45-single-m2f-stays-ordinary" {
        // T483: single method-to-function site veto full-cycle miniature --
        // one `a.old_method(1);` -> `new_function(a, 1);` line costs more
        // as a semantic hunk than the one ordinary hunk it replaces, so
        // no rule= hunks: pure ordinary passthrough. Veto side of
        // fixture 14 (3-site calls_old_method_to_new_function fire).
        assert!(
            !semantic.contains("rule="),
            "single m2f edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "46-use-divergent-tail-stays-ordinary" {
        // T483: divergent-tail use-prefix veto full-cycle miniature --
        // `use crate::old::A/B;` -> `new::A`/`new::C` keeps per-site tails
        // byte-unequal on the second site, so the N>=2 prefix sweep
        // declines (unit gate use_prefix_sweep_divergent_tail_...). No
        // rule= hunks: pure ordinary passthrough. Veto side of fixture
        // 16 (3-site use_old_to_new fire); hetero-across-sites shapes
        // still fire, only per-site divergence vetoes.
        assert!(
            !semantic.contains("rule="),
            "divergent-tail use-prefix edit must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("-use crate::old::B;"),
            "divergent-tail minus lines must be preserved, got: {semantic}"
        );
        assert!(
            semantic.contains("+use crate::new::C;"),
            "divergent-tail plus lines must be preserved, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "47-single-m2m-stays-ordinary" {
        // T484: single method-to-method site veto miniature --
        // one `a.old_method(1);` -> `a.new_method(1);` line
        // costs more as a semantic hunk than the ordinary hunk
        // it replaces: pure ordinary passthrough (code gate
        // `minuses.len() < 2` in the m2m sweep, veto side of
        // fixture 15, the 4-site old_method_to_new_method fire).
        assert!(
            !semantic.contains("rule="),
            "single m2m edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "48-single-call-stays-ordinary" {
        // T484: single bare-call rename veto miniature --
        // one `foo(1);` -> `bar(1);` line costs more as a
        // semantic hunk than the ordinary hunk it replaces:
        // pure ordinary passthrough (unit gate
        // `single_call_site_stays_passthrough`, veto side of
        // fixture 01, the 3-site foo_to_bar fire).
        assert!(
            !semantic.contains("rule="),
            "single call edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "49-single-same-literal-stays-ordinary" {
        // T485: single same-literal site veto miniature --
        // one `agents.contains("skills/rsmpl-git/SKILL.md");` ->
        // `agents.contains("skills/gitrs/SKILL.md");` costs more
        // as a semantic hunk than the ordinary hunk it replaces:
        // pure ordinary passthrough (unit gate
        // `method_same_literal_single_site_stays_passthrough`,
        // veto side of fixture 17, the 3-site calls_contains_literal fire).
        assert!(
            !semantic.contains("rule="),
            "single same-literal edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "50-single-ref-strip-stays-ordinary" {
        // T485: single ref-strip site veto miniature --
        // one `copy_tree(&a.join("repo"), x);` -> `copy_tree(a, x);`
        // costs more as a semantic hunk than the ordinary hunk it
        // replaces: pure ordinary passthrough (unit gate
        // `ref_strip_sweep_single_site_stays_passthrough`, veto side
        // of fixture 18, the 2-site calls_join_strip fire).
        assert!(
            !semantic.contains("rule="),
            "single ref-strip edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "51-single-call-arg-add-stays-ordinary" {
        // T486: single call-arg-add site veto miniature --
        // one `make_widget(alpha, 1);` -> `make_widget(alpha, 1, false);`
        // costs more as a semantic hunk than the ordinary hunk it
        // replaces: pure ordinary passthrough (veto side of
        // fixture 24, the multi-site arg-add fire).
        assert!(
            !semantic.contains("rule="),
            "single call-arg-add edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "52-single-call-literal-stays-ordinary" {
        // T486: single call-literal site veto miniature --
        // one `make_call("alpha", first_count, false);` ->
        // `make_call("alpha", first_count, "100644");` costs more
        // as a semantic hunk than the ordinary hunk it replaces:
        // pure ordinary passthrough (veto side of fixture 25,
        // the multi-site literal fire).
        assert!(
            !semantic.contains("rule="),
            "single call-literal edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "53-single-let-stays-ordinary" {
        // T487: single let-sweep site veto miniature --
        // one `let out = run(dir.path(), &["--patch", patch]);` ->
        // `let out = run_stdin(dir.path(), patch, &[]);` costs more
        // as a semantic hunk than the ordinary hunk it replaces:
        // pure ordinary passthrough (veto side of fixture 20,
        // the 2-site let fire).
        assert!(
            !semantic.contains("rule="),
            "single let edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "54-single-call-peel-stays-ordinary" {
        // T487: single call-peel site veto miniature --
        // one `let a = run(dir, &["--patch", patch]);` ->
        // `let a = run_stdin(dir, patch, &[]);` costs more as a
        // semantic hunk than the ordinary hunk it replaces: pure
        // ordinary passthrough (veto side of fixture 21, the
        // multi-site peel fire).
        assert!(
            !semantic.contains("rule="),
            "single call-peel edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "55-slot-hetero-fires" {
        // T488: hetero slot-sweep fire miniature -- two long-signature
        // fns renamed DIFFERENTLY fire one per-site single rule each
        // (homo multi-site shapes share one rule as in fixture 13;
        // hetero deltas cannot share, so diffrs emits per-site singles,
        // each clearing the T375 byte guard on its own).
        assert!(
            semantic.contains("rule=r1"),
            "hetero slot first site must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1"),
            "hetero slot second site must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn old_function_name("),
            "renamed minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn brand_new_name("),
            "renamed plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "hetero slot sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "56-single-let-call-literal-stays-ordinary" {
        // T488: single let-bound call-literal site veto miniature --
        // one `let a = make_call("alpha", first_count, false);` ->
        // `let a = make_call("alpha", first_count, "100644");` costs
        // more as a semantic hunk than the ordinary hunk it replaces:
        // pure ordinary passthrough (veto side of fixture 26, the
        // let-bound cross-hunk fire; fixture 52 pins the same veto for
        // the bare-statement shape of fixture 25).
        assert!(
            !semantic.contains("rule="),
            "single let-bound call-literal edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "57-single-verbatim-stays-ordinary" {
        // T489: single verbatim site veto miniature --
        // one `foo(&state.parsed);` -> `foo(parsed);` costs more
        // as a semantic hunk than the ordinary hunk it replaces:
        // pure ordinary passthrough (N>=2 gate in the verbatim
        // identical-line sweep, veto side of fixture 19, the
        // multi-site verbatim fire).
        assert!(
            !semantic.contains("rule="),
            "single verbatim edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "58-hetero-m2m-stays-ordinary" {
        // T489: hetero-delta m2m veto miniature --
        // `a.old_method(1);` -> `a.new_method(1);` plus
        // `b.old_method(2);` -> `b.brand_new(2);`: different new
        // idents share no (old, new) class, so the m2m sweep
        // declines with no per-site singles (call sweeps need one
        // shared delta -- the contrast to fixture 55, where hetero
        // slot deltas fire one single rule per site). Veto side of
        // fixture 15 (homo multi-site m2m fire).
        assert!(
            !semantic.contains("rule="),
            "hetero-delta m2m edit must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("-    a.old_method(1);"),
            "hetero minus lines must be preserved, got: {semantic}"
        );
        assert!(
            semantic.contains("+    b.brand_new(2);"),
            "hetero plus lines must be preserved, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "59-hetero-m2f-stays-ordinary" {
        // T490: hetero-delta m2f veto miniature --
        // `a.old_method(1);` -> `new_function(a, 1);` plus
        // `b.old_method(2);` -> `brand_new(b, 2);`: different new
        // callees share no (old, new) class, so the m2f sweep
        // declines with no per-site singles (call sweeps need one
        // shared delta -- the contrast to fixture 55, where hetero
        // slot deltas fire per-site singles). Veto side of
        // fixture 14 (homo multi-site m2f fire).
        assert!(
            !semantic.contains("rule="),
            "hetero-delta m2f edit must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("-    a.old_method(1);"),
            "hetero minus lines must be preserved, got: {semantic}"
        );
        assert!(
            semantic.contains("+    brand_new(b, 2);"),
            "hetero plus lines must be preserved, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "60-hetero-samelit-stays-ordinary" {
        // T490: hetero-delta same-literal veto miniature --
        // two identical `agents.contains("skills/rsmpl-git/...");`
        // sites rewrite to DIFFERENT new literals (`gitrs` /
        // `other`): no shared (old, new) literal pair, so the
        // same-literal sweep declines with no per-site singles
        // (same shared-delta gate as fixtures 58/59 -- the contrast
        // to fixture 55, where hetero slot deltas fire per-site
        // singles). Veto side of fixture 17 (homo multi-site
        // same-literal fire).
        assert!(
            !semantic.contains("rule="),
            "hetero-delta same-literal edit must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("-    agents.contains(\"skills/rsmpl-git/SKILL.md\");"),
            "hetero minus lines must be preserved, got: {semantic}"
        );
        assert!(
            semantic.contains("+    agents.contains(\"skills/other/SKILL.md\");"),
            "hetero plus lines must be preserved, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "97-collection-move-fires" {
        // WI-6: one (minus hunk, plus hunk) cell with >=3 items shares a
        // single ranged `# allow=many` cut + one bare paste (the
        // trailing item degrades to ordinary under the absorbed-blank
        // gate, and the move still round-trips byte-exact).
        assert!(
            semantic.contains("@@ -2,15 @@ rule=r1\n# allow=many\n# bind=M1\n- I\n"),
            "collection cut must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("@@ -1,0 @@ rule=r2\n# use=M1\n+ M1\n"),
            "collection paste must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("-fn e() {"),
            "gated-out item stays ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "collection must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "98-partition-fires" {
        // WI-8: 8 hetero item/fn renames (adjacent const+fn pairs) compress
        // pair by pair — one hunk per pair, positional rules — and the
        // harness checks the tree legs (--check + lowered git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            8,
            "all 8 renames must partition, got: {semantic}"
        );
        assert!(
            semantic.contains("- const OLD_CONST_1\n"),
            "const pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ const NEW_CONST_1\n"),
            "const pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("- fn old_helper_1\n"),
            "fn pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ fn new_helper_1\n"),
            "fn pair must raise, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "partition must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
    } else if name == "99-partition-atomicity-stays-ordinary" {
        // WI-8 atomicity: 7 renames plus one ordinary field edit stay
        // fully ordinary (never half-semantic).
        assert!(
            !semantic.contains("rule="),
            "mixed section must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("+    x: u32,\n"),
            "edit line must be preserved, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    }
}
