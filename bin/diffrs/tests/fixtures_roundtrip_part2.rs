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
        // T483 + WI-7 slice-3: single method-to-function rename veto
        // miniature -- one `a.old_method(1);` -> `new_function(a, 1);`
        // line with an untouched `b.old_method(9);` twin stays ordinary
        // via `method_holds` totality (2 sites vs 1 diff pair). Unique
        // singles now fire as allow-one precision singles (see
        // `methods::tests::wi7_m2f_*`); this fixture pins the twin veto,
        // veto side of fixture 14 (3-site calls_old_method_to_new_function
        // fire).
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
        // T484 + WI-7 slice-2: single method-to-method rename veto
        // miniature -- one `a.old_method(1);` -> `a.new_method(1);`
        // line with an untouched `b.old_method(9);` twin stays ordinary
        // via `method_holds` totality (2 sites vs 1 diff pair). Unique
        // singles now fire as allow-one precision singles (see
        // `methods::tests::wi7_*`); this fixture pins the twin veto,
        // veto side of fixture 15, the 4-site old_method_to_new_method fire).
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
        // T484 + WI-7 slice-1: single bare-call rename veto miniature --
        // one `foo(1);` -> `bar(1);` line with an untouched `foo(9);`
        // twin stays ordinary via `call_holds` totality (2 sites vs 1
        // diff pair). Unique singles now fire as allow-one precision
        // singles (see `calls::tests::wi7_*`); this fixture pins the
        // twin veto, veto side of fixture 01 (3-site foo_to_bar fire).
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
        // T485 + WI-7 slice-13: unique single method same-literal delta
        // now fires as an allow-one precision single (sweep-vs-single
        // canonically distinct until WI-8 site-set equality). The twin
        // veto is pinned by the `wi13_*_twin_vetoes` unit gates; this
        // fixture pins the unique fire (veto side of fixture 17, the
        // 3-site calls_contains_literal fire; contrast to fixture 60,
        // the hetero twin that stays ordinary via tuple totality).
        assert!(
            semantic.contains("rule=r1"),
            "unique method literal single must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("# allow=many"),
            "allow-one, no allow=many: {semantic}"
        );
        assert!(!semantic.contains("# sweep"), "ranged single: {semantic}");
        assert!(
            semantic.contains("- Receiver.contains(\"skills/rsmpl-git/SKILL.md\")\n"),
            "anchored minus template, got: {semantic}"
        );
        assert!(
            semantic.contains("+ Receiver.contains(\"skills/gitrs/SKILL.md\")\n"),
            "anchored plus template, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    agents.contains("),
            "native minus must be consumed, got: {semantic}"
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
        // T486 + WI-7 slice-12: unique single call-literal delta now
        // fires as an allow-one precision single (sweep-vs-single
        // canonically distinct until WI-8 site-set equality). The twin
        // veto is pinned by the `wi12_*_twin_vetoes` unit gates; this
        // fixture pins the unique fire (contrast to fixture 25, the
        // multi-site literal fire).
        assert!(
            semantic.contains("rule=r1"),
            "unique literal single must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("# allow=many"),
            "allow-one, no allow=many: {semantic}"
        );
        assert!(!semantic.contains("# sweep"), "ranged single: {semantic}");
        assert!(
            semantic.contains("- make_call(E0, E1, false)\n"),
            "anchored minus template, got: {semantic}"
        );
        assert!(
            semantic.contains("+ make_call(E0, E1, \"100644\")\n"),
            "anchored plus template, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    make_call("),
            "native minus must be consumed, got: {semantic}"
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
        // T488 + WI-7 slice-12: unique let-bound call-literal delta now
        // fires as an allow-one precision single via the shared T449
        // `let`-init entry (emission stays the bare-call pattern; the
        // engine rewrites the call sub-expression in place). The twin
        // veto is pinned by the `wi12_*_twin_vetoes` unit gates; this
        // fixture pins the unique let-init fire (contrast to fixture
        // 26, the let-bound cross-hunk fire).
        assert!(
            semantic.contains("rule=r1"),
            "unique let-init literal single must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("# allow=many"),
            "allow-one, no allow=many: {semantic}"
        );
        assert!(!semantic.contains("# sweep"), "ranged single: {semantic}");
        assert!(
            semantic.contains("- make_call(E0, E1, false)\n"),
            "anchored minus template, got: {semantic}"
        );
        assert!(
            semantic.contains("+ make_call(E0, E1, \"100644\")\n"),
            "anchored plus template, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    let a = make_call("),
            "native minus must be consumed, got: {semantic}"
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
        // T490 + WI-7 slice-13: hetero-delta same-literal veto miniature --
        // two identical `agents.contains("skills/rsmpl-git/...");`
        // sites rewrite to DIFFERENT new literals (`gitrs` /
        // `other`): no shared (old, new) literal pair, so the
        // same-literal sweep declines; the per-site singles decline too
        // (tuple-keyed totality: both sites share the same old-args
        // tuple, so each single over-counts 2 vs 1 — the contrast to
        // fixture 49, where the unique single fires). Veto side of
        // fixture 17 (homo multi-site same-literal fire).
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
    } else if name == "99-partition-partial-mixed" {
        // WI-19 partial partition: 6 rename hunks compress (definition
        // shapes under the precision guard) while the mixed hunk
        // (`struct OldA` rename + `x: i32` → `u32` type edit in one
        // native hunk) stays ordinary whole — hunk atomicity kept,
        // section never half-claims within a hunk.
        assert_eq!(
            semantic.matches("rule=").count(),
            6,
            "six rename hunks must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("- struct OldB\n"),
            "rename hunks must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("-struct OldA {\n-    x: i32,\n+struct NewA {\n+    x: u32,\n"),
            "mixed hunk must stay ordinary verbatim, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
    } else if name == "129-subhunk-partition-fires" {
        // WI-4 sub-hunk partition: one native hunk, two detector classes
        // (call rewrite + general-expression rewrite) — each contiguous
        // run raises under its own rule, in document order, and the
        // harness checks the tree legs (--check + lowered git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            2,
            "both runs must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("- foo(E0)\n"),
            "call run must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("- E0 + 1\n"),
            "expr run must raise, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "partition must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
    } else if name == "100-field-rename-fires" {
        // WI-9b: 1 field definition + 5 use lines compress to the
        // definition hunk with `# follow=uses`; the harness checks the
        // tree legs (--check + lowered git apply).
        assert!(
            semantic.contains("# follow=uses"),
            "member rename must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("  struct ConnectionConfig {"),
            "field header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("- request_timeout_milliseconds: u32"),
            "def minus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ response_deadline_milliseconds: u32"),
            "def plus must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    cfg.request_timeout_milliseconds"),
            "use lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "member rename must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "101-variant-rename-fires" {
        // WI-9b: 1 variant definition + 4 use lines compress to the
        // definition hunk with `# follow=uses`.
        assert!(
            semantic.contains("# follow=uses"),
            "member rename must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("  enum ConnectionState {"),
            "variant header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("- EstablishedConnection\n"),
            "def minus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ ActiveSession\n"),
            "def plus must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    let first = ConnectionState::EstablishedConnection(1);"),
            "use lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "member rename must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "102-attr-replace-fires" {
        // WI-10b: uniform derive migration over hetero structs raises to
        // one N-holed item hunk and beats native bytes.
        assert!(
            semantic.contains("- struct N\n"),
            "attr replace must raise the N-hole form, got: {semantic}"
        );
        assert!(
            semantic.contains("+ #[derive(Debug, Clone)]\n"),
            "plus attr must raise, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "attr replace must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "103-attr-insert-fires" {
        // WI-10b: uniform `#[inline]` insertions over plain fns raise to
        // the minimal-tier N-holed fn hunk.
        assert!(
            semantic.contains("- fn N(...) {\n"),
            "attr insert must raise the minimal fn form, got: {semantic}"
        );
        assert!(
            semantic.contains("+ #[inline]\n"),
            "plus attr must raise, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "attr insert must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "104-attr-twin-stays-ordinary" {
        // WI-10b: an untouched same-state twin vetoes the class — the
        // two migrated sites stay ordinary rather than rewriting the twin.
        assert!(
            !semantic.contains("rule="),
            "twin veto must stay ordinary, got: {semantic}"
        );
    } else if name == "105-let-attr-stays-ordinary" {
        // WI-10b: `let`-statement attr insertions have no raising detector
        // yet (authoring only) — the diff stays ordinary.
        assert!(
            !semantic.contains("rule="),
            "let-attr insert must stay ordinary, got: {semantic}"
        );
    } else if name == "106-definition-alias-fires" {
        // WI-12 slice 1: single-definition alias evolution raises to the
        // WI-11a authored spelling under the precision guard.
        assert!(
            semantic.contains("- type A = Vec<u8>;"),
            "alias minus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ type A = Box<[u8]>;"),
            "alias plus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1"),
            "single evolution must carry a rule, got: {semantic}"
        );
    } else if name == "107-definition-field-type-fires" {
        // WI-12 slice 1: single-definition field-type evolution raises
        // to the WI-11b member spelling with its container header.
        assert!(
            semantic.contains("  struct S {"),
            "field header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("- old: u32"),
            "field minus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ old: u64"),
            "field plus must raise, got: {semantic}"
        );
    } else if name == "108-definition-field-add-fires" {
        // WI-12b slice 1: single-insertion field addition raises to the
        // authored spelling (container header + one `+` member line).
        assert!(
            semantic.contains("  struct S {"),
            "field header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("+ b: bool,"),
            "field plus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1"),
            "single addition must carry a rule, got: {semantic}"
        );
    } else if name == "109-definition-variant-add-fires" {
        // WI-12b slice 1: single-insertion variant addition raises to
        // the authored spelling (container header + one `+` member).
        assert!(
            semantic.contains("  enum E {"),
            "variant header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("+ C,"),
            "variant plus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1"),
            "single addition must carry a rule, got: {semantic}"
        );
    } else if name == "110-field-add-follow-fires" {
        // WI-12c slice 1: definition-addition + struct-literal
        // follow-through (def hunk + 2 per-site concrete ranged
        // construction hunks, hetero values pinned per site).
        assert!(
            semantic.contains("  struct S {"),
            "def header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("+ b: bool,"),
            "def member must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1")
                && semantic.contains("rule=r2")
                && semantic.contains("rule=r3"),
            "def + 2 constructions carry rules, got: {semantic}"
        );
        assert!(
            semantic.contains("- S { a: 1 }"),
            "first construction minus, got: {semantic}"
        );
        assert!(
            semantic.contains("+ S { a: 1, b: true }"),
            "hetero true value stays per-site, got: {semantic}"
        );
        assert!(
            semantic.contains("+ S { a: 2, b: false }"),
            "hetero false value stays per-site, got: {semantic}"
        );
    } else if name == "111-field-add-rest-partial" {
        // WI-12c slice 1: `..`-rest site needs no edit and produces no
        // diff hunk — the group is def + the edited `let`-bound
        // literal; the rest site stays byte-identical.
        assert!(
            semantic.contains("  struct S {"),
            "def header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("+ b: bool,"),
            "def member must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("- let x = S { a: 1 };"),
            "let-bound construction minus, got: {semantic}"
        );
        assert!(
            semantic.contains("+ let x = S { a: 1, b: true };"),
            "let-bound construction plus, got: {semantic}"
        );
        assert!(
            !semantic.contains("..Default::default()"),
            "untouched rest site must not appear, got: {semantic}"
        );
    } else if name == "112-variant-add-arm-follow-fires" {
        // WI-12d: variant addition + qualified match-arm follow-through
        // (def hunk + per-site concrete ranged arm hunk).
        assert!(
            semantic.contains("  enum E {"),
            "variant header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("+ C,"),
            "variant plus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1") && semantic.contains("rule=r2"),
            "def + arm carry rules, got: {semantic}"
        );
        assert!(
            semantic.contains("  match e {"),
            "arm context must name the match, got: {semantic}"
        );
        assert!(
            semantic.contains("+ E::C => 3,"),
            "qualified arm must raise, got: {semantic}"
        );
    } else if name == "113-variant-add-glob-bare-fires" {
        // WI-12d: `use E::*` anchors the bare `C => 3,` arm to the
        // added variant — def hunk + bare concrete ranged arm hunk.
        assert!(
            semantic.contains("  enum E {"),
            "variant header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("+ C,"),
            "variant plus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1") && semantic.contains("rule=r2"),
            "def + arm carry rules, got: {semantic}"
        );
        assert!(
            semantic.contains("+ C => 3,"),
            "bare arm must raise verbatim, got: {semantic}"
        );
        assert!(
            !semantic.contains("E::C"),
            "glob-anchored arm stays bare, got: {semantic}"
        );
    } else if name == "114-variant-add-two-match-fires" {
        // WI-12d: hetero arm bodies ride per-site concrete (never
        // holes) across two matches.
        assert!(
            semantic.contains("rule=r1")
                && semantic.contains("rule=r2")
                && semantic.contains("rule=r3"),
            "def + 2 arms carry rules, got: {semantic}"
        );
        assert!(
            semantic.contains("+ E::C => 3,"),
            "first hetero body stays per-site, got: {semantic}"
        );
        assert!(
            semantic.contains("+ E::C => 30,"),
            "second hetero body stays per-site, got: {semantic}"
        );
    } else if name == "118-fn-joint-params-fires" {
        // WI-4a: `fn foo(x: i32)` → `fn bar(x: u32)` + callee-only call
        // updates raise to the single follow hunk whose header pair
        // carries the rename+slot delta (engine follow rewrites the
        // callees; the harness asserts byte-identical re-apply).
        assert!(
            semantic.contains("# follow=uses"),
            "joint must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("- fn foo(x: i32) { ... }"),
            "joint header minus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ fn bar(x: u32) { ... }"),
            "joint header plus must raise, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "119-fn-joint-body-stays-ordinary" {
        // WI-4a veto: rename+body-literal stays wholly ordinary — the
        // fragmented fallback (ordinary def + rename-claiming call
        // sweep) is suppressed, so no `rule=` hunk may appear.
        assert!(
            !semantic.contains("rule="),
            "body veto must stay wholly ordinary, got: {semantic}"
        );
    } else if name == "120-fn-joint-arg-add-fires" {
        // WI-4b B1: rename+param-add with uniform arg-adding calls
        // raises to the shared-rule two-hunk composition (def hunk +
        // call-template hunk, no follow); the re-apply is byte-identical
        // (harness asserts).
        assert!(
            semantic.matches("rule=r1").count() >= 2,
            "both hunks share one rule, got: {semantic}"
        );
        assert!(
            !semantic.contains("# follow=uses"),
            "no follow in a joint, got: {semantic}"
        );
        assert!(
            semantic.contains("+ fn bar(x: i32, y: i32) { ... }"),
            "def plus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ bar(E0, 0)"),
            "call template plus must raise, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "121-fn-joint-attr-fires" {
        // WI-4b B2: rename+attr-add with callee-only calls raises to
        // the shared-rule two-hunk composition (def hunk carrying the
        // attr line + call-template hunk); the re-apply is
        // byte-identical (harness asserts).
        assert!(
            semantic.matches("rule=r1").count() >= 2,
            "both hunks share one rule, got: {semantic}"
        );
        assert!(
            !semantic.contains("# follow=uses"),
            "no follow in a joint, got: {semantic}"
        );
        assert!(
            semantic.contains("+ #[must_use]"),
            "attr line must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ bar(E0)"),
            "call template plus must raise, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "116-assoc-rename-fires" {
        // WI-28 + WI-3: trait decl + 2 impl decls + `::Item` use tails
        // sharing one delta compress to the single trait follow hunk
        // (WI-1 claims the impl defs); the re-apply is byte-identical
        // (harness asserts).
        assert!(
            semantic.contains("  trait Tr {"),
            "trait header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("- type Item;"),
            "trait decl pair must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("- type Item = u8;"),
            "WI-3: impl decls ride the follow, got: {semantic}"
        );
        assert!(
            semantic.contains("# follow=uses"),
            "intent must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1"),
            "shared rule must carry, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "assoc intent must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "117-assoc-rename-bare-use-stays-ordinary" {
        // WI-28 boundary, WI-19b relaxed: a bare `Item` use (no `::`
        // tail) is opaque to the engine follow, so the whole-section
        // follow intent still declines — but the def pair compresses
        // per-hunk as a follow-free def-only single (the bare lines
        // stay ordinary verbatim in the same section; disjoint ranges).
        // Never a follow hunk: the opaque use must not be claimed.
        assert!(
            !semantic.contains("follow=uses"),
            "bare-use class must never carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("- type Item;"),
            "def pair must raise def-only, got: {semantic}"
        );
        assert!(
            semantic.contains("-    Item\n") && semantic.contains("+    Output\n"),
            "bare use stays ordinary, got: {semantic}"
        );
    } else if name == "115-variant-add-unanchored-bare-ordinary" {
        // WI-12d boundary: a bare arm with no `use E::*` glob in scope
        // declines the group — the definition still fires alone and the
        // arm stays ordinary.
        assert!(
            semantic.contains("  enum E {"),
            "variant header must be verbatim, got: {semantic}"
        );
        assert!(
            semantic.contains("+ C,"),
            "variant plus must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1"),
            "def addition must carry a rule, got: {semantic}"
        );
        assert!(
            semantic.contains("+        C => 3,") || semantic.contains("+C => 3,"),
            "unanchored arm stays ordinary, got: {semantic}"
        );
        assert!(
            !semantic.contains("rule=r2"),
            "no follow hunk without a glob anchor, got: {semantic}"
        );
    } else if name == "122-move-rename-fires" {
        // WI-6 slice 1: move+rename with an identical body raises as an
        // evolved cut/paste pair (WI-8 pairing cascade) — the F5 core.
        assert!(
            semantic.contains("# bind="),
            "rename-move cut must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("# use="),
            "rename-move paste must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ fn bar(...) { ... }"),
            "paste must carry the evolved header, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn foo"),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn foo"),
            "no stale plus lines, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "rename-move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "123-move-rename-caller-fires" {
        // WI-6 slice 1: move+rename with the caller updated composes as
        // an evolved cut/paste plus a call sweep over the renamed
        // callee (the WI-6 acceptance pins the follow form).
        assert!(
            semantic.contains("# bind="),
            "rename-move cut must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("# use="),
            "rename-move paste must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- foo(E0)") && semantic.contains("+ bar(E0)"),
            "caller update must ride a call sweep, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn foo"),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "composition must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "125-move-adjacent-caller-fires" {
        // WI-6 slice 2: the moved item pasted directly above an edited
        // one-line caller merges into one mixed `-U0` hunk (no unchanged
        // line separates insertion from edit). The virtual minus/plus
        // split lets the move fire as an evolved cut/paste; the edited
        // caller rides ordinary leftovers around the paste (the one-line
        // `fn` frame is not a sweepable call shape — WI-9 domain).
        assert!(
            semantic.contains("# bind="),
            "rename-move cut must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("# use="),
            "rename-move paste must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ fn bar(...) { ... }"),
            "paste must carry the evolved header, got: {semantic}"
        );
        assert!(
            semantic.contains("-fn caller() { let r = foo(1); }"),
            "caller minus rides leftover, got: {semantic}"
        );
        assert!(
            semantic.contains("+fn caller() { let r = bar(1); }"),
            "caller plus rides leftover, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn foo"),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "adjacent move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "126-move-minus-side-adjacent-fires" {
        // WI-6 slice 3a: the moved item deleted adjacent to an edited
        // one-line caller merges into one mixed `-U0` hunk. The ordered
        // run-split anchors the caller plus run past the cut, so the
        // move+rename fires as an evolved cut/paste; the edited caller
        // rides ordinary leftovers (the one-line `fn` frame is not a
        // sweepable call shape — WI-9 domain).
        assert!(
            semantic.contains("# bind="),
            "rename-move cut must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("# use="),
            "rename-move paste must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ fn bar(...) { ... }"),
            "paste must carry the evolved header, got: {semantic}"
        );
        assert!(
            semantic.contains("-fn caller() { let r = foo(1); }"),
            "caller minus rides leftover, got: {semantic}"
        );
        assert!(
            semantic.contains("+fn caller() { let r = bar(1); }"),
            "caller plus rides leftover, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "minus-side move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "124-move-body-evolve-fires" {
        // WI-6 slice 3b: move+rename whose body renames its self-reference
        // fires as one evolving cut/paste — the `as` template carries the
        // body delta, the splice renames the captured recursive call.
        assert!(
            semantic.contains("# bind="),
            "body-evolving move must cut, got: {semantic}"
        );
        assert!(
            semantic.contains("# use="),
            "body-evolving move must paste, got: {semantic}"
        );
        assert!(
            semantic.contains("+ fn fact as factorial(...) { ... }"),
            "paste must carry the `as` template, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn fact"),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn factorial"),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "body-evolving move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "128-joint-impl-rename-fires" {
        // WI-3: trait + self-type renames composed in one `impl` header
        // raise as one shared-rule intent (trait follow hunk first,
        // self-type second); the composed header is rewritten by the two
        // follows, never stated.
        assert!(
            semantic.contains("rule=r1\n"),
            "joint impl must share one rule, got: {semantic}"
        );
        assert!(
            semantic.contains("- trait Printable\n"),
            "trait pair must ride first, got: {semantic}"
        );
        assert!(
            semantic.contains("- struct Old\n"),
            "self-type pair must ride second, got: {semantic}"
        );
        assert!(
            !semantic.contains("impl Printable"),
            "composed header is follow-rewritten, never stated, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint impl must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "131-joint-impl-extras-fires" {
        // WI-3b: triple with a `type`-alias self-ty plus a follow-use
        // line raises as one shared-rule intent (trait pair first,
        // self-type second); the composed header and the use line are
        // follow-rewritten, never stated.
        assert!(
            semantic.contains("rule=r1\n"),
            "joint impl extras must share one rule, got: {semantic}"
        );
        assert!(
            semantic.contains("- trait Printable\n"),
            "trait pair must ride first, got: {semantic}"
        );
        assert!(
            semantic.contains("- type Old\n"),
            "type-alias pair must ride second, got: {semantic}"
        );
        assert!(
            !semantic.contains("impl Printable"),
            "composed header is follow-rewritten, never stated, got: {semantic}"
        );
        assert!(
            !semantic.contains("fn f(x: Old)"),
            "use line is follow-rewritten, never stated, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint impl extras must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "127-move-body-evolve-strings-kept" {
        // WI-6 slice 3b variant: the moved body renames its self-reference
        // but keeps a `"fact"` string literal and a `// fact` comment
        // byte-verbatim (the shared scanner skips both).
        assert!(
            semantic.contains("# bind="),
            "body-evolving move must cut, got: {semantic}"
        );
        assert!(
            semantic.contains("# use="),
            "body-evolving move must paste, got: {semantic}"
        );
        assert!(
            semantic.contains("+ fn fact as factorial(...) { ... }"),
            "paste must carry the `as` template, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn fact"),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn factorial"),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "body-evolving move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "130-fn-rename-visqual-joint-fires" {
        // WI-5 slice 1: a `fn` rename + single vis evolution joins its
        // call sites into one follow hunk carrying the concrete evolved
        // terse pair (never a `V`/`Q` hole); the harness checks the
        // tree legs (--check + lowered git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            1,
            "one joint intent must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("# follow=uses"),
            "joint must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("- pub fn foo\n"),
            "evolved minus pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ pub(crate) fn bar\n"),
            "evolved plus pair must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("# visibility"),
            "never a `V` hole, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    let a = foo(1);"),
            "use lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
    } else if name == "132-fn-rename-visqual-extern-fires" {
        // WI-5-R1: a `fn` rename + `extern "C"` qualifier addition joins
        // its call sites into one follow hunk carrying the concrete
        // evolved terse pair (never a `Q` hole); the harness checks the
        // tree legs (--check + lowered git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            1,
            "one joint intent must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("# follow=uses"),
            "joint must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("- fn foo\n"),
            "evolved minus pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ extern \"C\" fn bar\n"),
            "evolved plus pair must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    let a = foo(1);"),
            "use lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
    } else if name == "133-bare-visjoint-fires" {
        // WI-5-R2: a bare-item rename + vis narrowing joins its use
        // sites into one follow hunk carrying the concrete evolved
        // terse pair (never a hole); the harness checks the tree legs
        // (--check + lowered git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            1,
            "one joint intent must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("# follow=uses"),
            "joint must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("- pub struct Old\n"),
            "evolved minus pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ pub(crate) struct New\n"),
            "evolved plus pair must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn f(s: Old)"),
            "use lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
    } else if name == "134-multiline-fn-visjoint-fires" {
        // WI-5-R3: a `fn` rename + vis narrowing whose header spans
        // lines joins its call sites into one follow hunk carrying the
        // concrete evolved terse pair (never a hole); the harness
        // checks the tree legs (--check + lowered git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            1,
            "one joint intent must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("# follow=uses"),
            "joint must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("- pub fn foo\n"),
            "evolved minus pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ pub(crate) fn bar\n"),
            "evolved plus pair must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    let a = foo(1);"),
            "use lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "joint must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
    } else if name == "135-bare-vis-sweep-fires" {
        // Vis-sweep residue: a uniform closed-set vis step over two
        // hetero structs raises as one N-holed sweep hunk (not two
        // singles); the harness checks the tree legs (--check + lowered
        // git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            1,
            "one sweep intent must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("# allow=many"),
            "sweep keeps allow-many, got: {semantic}"
        );
        assert!(
            semantic.contains("- struct N\n"),
            "N-holed minus pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("+ pub struct N\n"),
            "N-holed plus pair must raise, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
    } else if name == "100-cross-file-assoc-fires" {
        // WI-12: trait decl + impl def + `::Item` uses across files join
        // one shared-rule intent (one section per def file, follow on
        // the document-first hunk); the harness checks the tree legs
        // (--check + lowered git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            2,
            "one intent in two def files must raise, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("# follow=uses").count(),
            1,
            "follow exactly once, got: {semantic}"
        );
        assert!(
            semantic.contains("- type Item;"),
            "trait decl pair must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("-    type Item = u8;") || semantic.contains("- type Item = u8;"),
            "impl def pair must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn f() -> S::Item"),
            "use lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "cross-file intent must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "101-cross-file-trait-method-fires" {
        // WI-12: trait decl + trait-`impl` def + anchored call across
        // files join the single trait-block hunk + follow (the follow
        // owns every remote def and call); the harness checks the tree
        // legs (--check + lowered git apply).
        assert_eq!(
            semantic.matches("rule=").count(),
            1,
            "one single-hunk intent must raise, got: {semantic}"
        );
        assert!(
            semantic.contains("# follow=uses"),
            "intent must carry follow, got: {semantic}"
        );
        assert!(
            semantic.contains("fn walk"),
            "renamed method must raise, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    fn run(&self);"),
            "decl lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "cross-file intent must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    }
}
