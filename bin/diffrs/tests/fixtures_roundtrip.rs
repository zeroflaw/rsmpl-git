//! T433 fixture-driven diffrs full cycle, part 1 (fixtures `01`–`44`).
//!
//! Each `test-fixtures/diffrs/NN-name/` holds `repo/` (pre-state) and
//! `expected/` (post-state). There is no `patch.rsmpl` — the semantic
//! patch is the tool output, not the input. The six-step loop lives in
//! `common` (`run_all`); this part supplies the shape pins for fixtures
//! `01`–`44`. Patch bytes are never compared directly: header styles
//! differ by design (diffrs emits single-path `diff --git <path>` with no
//! `index` lines, T374; git emits `a/ b/` prefixes), so end-state
//! equivalence is the cycle equality (same lesson as T361: `--check`
//! alone is blind, byte-compare the applied tree).
//!
//! Split note (WI-9-pre): this file was the whole `fixtures_roundtrip`
//! test; the `45+` pins moved to `fixtures_roundtrip_part2.rs` and the
//! harness to `common`. Part 1's `01`–`44` range is closed — all future
//! fixtures (`87+`) land in part 2.
//!
//! Requires a workspace build so the sibling `applyrs` binary sits next
//! to `diffrs` (`cargo test --workspace`).

mod common;

use common::{fixture_num, normalize_native, run_all, PART_BOUNDARY};

#[test]
fn native_to_semantic_to_native_round_trip_part1() {
    run_all("part1", |n| fixture_num(n) < PART_BOUNDARY, pins_early);
}

fn pins_early(name: &str, native_input: &str, semantic: &str) {
    if name == "01-call-sweep" {
        assert!(
            semantic.contains("rule="),
            "sweep must compress, got: {semantic}"
        );
        assert!(semantic.contains("- foo("), "got: {semantic}");
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "02-ordinary-edit" {
        assert!(
            !semantic.contains("rule="),
            "ordinary edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        // T434 slice 10: ordinary passthrough is exactly normalized
        // native (the three infer rewrites), so patch bytes compare
        // directly — not just end-states.
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "03-deleted-file-move" {
        assert!(
            semantic.contains("rule=r1"),
            "deleted-file move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "deleted-file move must paste, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn big("),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn big("),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "04-struct-move" {
        assert!(
            semantic.contains("rule=r1"),
            "struct move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "struct move must paste, got: {semantic}"
        );
        assert!(
            !semantic.contains("-struct Big"),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+struct Big"),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "05-move-trailing-leftover" {
        assert!(
            semantic.contains("rule=r1"),
            "trailing-leftover move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "trailing-leftover move must paste, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn big("),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn big("),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("+// trailing"),
            "trailing leftover must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "06-move-prefix-leftover" {
        assert!(
            semantic.contains("rule=r1"),
            "prefix-leftover move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "prefix-leftover move must paste, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn big("),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn big("),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("-fn gone() {"),
            "prefix leftover must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "07-move-fanout" {
        assert!(
            semantic.contains("rule=r1"),
            "fan-out move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "fan-out move must paste, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r3"),
            "fan-out second paste must fire, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("+ M1\n").count(),
            2,
            "fan-out pastes must share the bind, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn big("),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn big("),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "08-move-eof-cap" {
        assert!(
            semantic.contains("rule=r1"),
            "eof-cap move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "eof-cap move must paste, got: {semantic}"
        );
        assert!(
            !semantic.contains("rule=r3"),
            "second pair must cap, got: {semantic}"
        );
        assert!(
            !semantic.contains("rule=r4"),
            "second pair must cap, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn one("),
            "paired lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn one("),
            "paired lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("-fn two() {"),
            "capped run must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("+fn two() {"),
            "capped run must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "09-move-dedup" {
        assert!(
            semantic.contains("rule=r1"),
            "dedup move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "dedup move must paste, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r1\n").count(),
            1,
            "first claim wins, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r2\n").count(),
            1,
            "single paste, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("+ M1\n").count(),
            1,
            "single paste shares the bind once, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r3"),
            "losing twin minus compresses (T472 EOF relaxation), got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn big("),
            "losing twin minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn big("),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "10-move-same-hunk-fanout" {
        assert!(
            semantic.contains("rule=r1"),
            "same-hunk fan-out move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "same-hunk fan-out move must paste, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r3"),
            "same-hunk fan-out second paste must fire, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("+ M1\n").count(),
            2,
            "both same-hunk pastes share the bind, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r").count(),
            3,
            "one cut + two pastes, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn big("),
            "minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn big("),
            "plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "11-move-same-file-reorder" {
        assert!(
            semantic.contains("rule=r1"),
            "same-file reorder must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "same-file reorder must paste, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("+ M1\n").count(),
            1,
            "single paste shares the bind once, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn aaa("),
            "moved lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn aaa("),
            "moved lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("fn bbb("),
            "untouched middle item stays out of sem, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "12-move-interior-blanks" {
        assert!(
            semantic.contains("rule=r1"),
            "interior-blank move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r1"),
            "interior-blank second item must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "interior-blank move must paste, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "interior-blank second paste must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn big("),
            "moved lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn big("),
            "moved lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn spaced("),
            "interior-blank lines must ride the cut, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn spaced("),
            "interior-blank lines must ride the paste, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "13-slot-sweep" {
        assert!(
            semantic.contains("rule=r1"),
            "slot sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- V Q fn old_function_name G (...) W {"),
            "slot sweep minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ V Q fn new_function_name G (...) W {"),
            "slot sweep plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn old_function_name("),
            "renamed lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn new_function_name("),
            "renamed lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "14-method-to-function" {
        assert!(
            semantic.contains("rule=r1"),
            "m2f sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- Receiver.old_method(E0)"),
            "m2f sweep minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ new_function(Receiver, E0)"),
            "m2f sweep plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    a.old_method(1);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    new_function(a, 1);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "15-method-to-method" {
        assert!(
            semantic.contains("rule=r1"),
            "m2m sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- Receiver.old_method(E0)"),
            "m2m sweep minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ Receiver.new_method(E0)"),
            "m2m sweep plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    a.old_method(1);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    a.new_method(1);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "16-use-prefix" {
        assert!(
            semantic.contains("rule=r1"),
            "use-prefix sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- use crate::old"),
            "use-prefix minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ use crate::new"),
            "use-prefix plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-use crate::old::Thing;"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+use crate::new::Thing;"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "17-same-literal" {
        assert!(
            semantic.contains("rule=r1"),
            "same-literal sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- Receiver.contains(\"skills/rsmpl-git/SKILL.md\")"),
            "same-literal minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ Receiver.contains(\"skills/gitrs/SKILL.md\")"),
            "same-literal plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    agents.contains(\"skills/rsmpl-git/SKILL.md\");"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    agents.contains(\"skills/gitrs/SKILL.md\");"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "18-ref-strip" {
        assert!(
            semantic.contains("rule=r1"),
            "ref-strip sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- &Receiver.join(\"repo\")"),
            "ref-strip minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ Receiver"),
            "ref-strip plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    copy_tree(&a.join(\"repo\"), x);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    copy_tree(a, x);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "19-verbatim" {
        assert!(
            semantic.contains("rule=r1"),
            "verbatim sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- foo(&state.parsed)"),
            "verbatim minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ foo(parsed)"),
            "verbatim plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    foo(&state.parsed);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    foo(parsed);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "20-let-sweep" {
        assert!(
            semantic.contains("rule=r1"),
            "let-statement verbatim sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- let out = run(dir.path(), &[\"--patch\", patch]);"),
            "let minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ let out = run_stdin(dir.path(), patch, &[]);"),
            "let plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    let out = run(dir.path(), &[\"--patch\", patch]);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    let out = run_stdin(dir.path(), patch, &[]);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "21-call-peel" {
        assert!(
            semantic.contains("rule=r1"),
            "peel sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- run(D, &[\"--patch\", M])"),
            "j=0 peel minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ run_stdin(D, M, &[])"),
            "j=0 peel plus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- run(D, &[\"--patch\", M, T0])"),
            "j=1 peel minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ run_stdin(D, M, &[T0])"),
            "j=1 peel plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    let a = run(dir, &[\"--patch\", patch]);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    let d = run_stdin(dir, patch, &[\"--check\"]);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic
                .contains("-    let f = run(dir, &[\"--patch\", patch, \"--format\", \"json\"]);"),
            "singleton j=2 site must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "22-verbatim-stranded" {
        assert!(
            semantic.contains("@@ -2,19 @@ rule=r1"),
            "stranded verbatim sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- validated(&text)"),
            "stranded minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ validated_live(&text, &pre)"),
            "stranded plus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("@@ -20,0 +21,3 @@"),
            "stranded leftover insertion must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    validated(&text);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    validated_live(&text, &pre);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "23-macro-nested-fires" {
        // WI-9b2b (was `23-macro-veto`, the T445 stays-ordinary pin):
        // the env!-in-init lets (e06313c miniature,
        // verbatim_init_ok macro gate) now compress to one nested
        // concrete hunk spelling just the macro pair, while the
        // format!-interior sites (33a8cbb miniature, engine-blind
        // macro bodies) stay ordinary — macro-body descent is WI-9b2
        // remainder territory.
        assert!(
            semantic.contains("rule=r1"),
            "nested interior must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- env!(\"CARGO_BIN_EXE_gitrs\")\n"),
            "nested minus missing, got: {semantic}"
        );
        assert!(
            semantic.contains("+ env!(\"CARGO_BIN_EXE_applyrs\")\n"),
            "nested plus missing, got: {semantic}"
        );
        assert!(
            !semantic.contains("E0"),
            "no holes expected, got: {semantic}"
        );
        assert!(
            !semantic.contains(
                "-    let out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));"
            ),
            "nested minus lines must be consumed, got: {semantic}"
        );
        assert!(
                semantic.contains("-        header = rsmpl_patch::collapse_git_header(&format!(\"diff --git a/{path} b/{path}\"))"),
                "format-body minus lines must be preserved, got: {semantic}"
            );
        assert!(
            semantic.contains("+        header = super::git_header(path)"),
            "format-body plus lines must be preserved, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "nested sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "24-call-arg-add" {
        // T447: homo trailing addition compresses (a.rs), hetero
        // trailing (b.rs, 09d3446 miniature) stays ordinary.
        assert!(
            semantic.contains("rule=r1"),
            "arg-add sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- make_widget(E0, E1)"),
            "arg-add minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ make_widget(E0, E1, false)"),
            "arg-add plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    make_widget(alpha, 1);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    make_widget(alpha, 1, false);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("-    make_widget(theta, 4);"),
            "hetero-trailing minus lines must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("+    make_widget(theta, 4, symlink);"),
            "hetero-trailing plus lines must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "25-call-literal" {
        // T448: homo trailing literal delta compresses (a.rs, the
        // 7025a14 prize miniature); hetero trailing (b.rs) stays
        // ordinary without vetoing via tuple-keyed totality.
        assert!(
            semantic.contains("rule=r1"),
            "literal sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- make_call(E0, E1, false)"),
            "literal minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ make_call(E0, E1, \"100644\")"),
            "literal plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    make_call(\"alpha\", first_count, false);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    make_call(\"alpha\", first_count, \"100644\");"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("-    make_call(\"gamma\", third_count, true);"),
            "hetero-trailing minus lines must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("+    make_call(\"delta\", fourth_count, symlink);"),
            "hetero-trailing plus lines must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "26-call-literal-cross-hunk" {
        // T449: spaced `let` sites split across singleton `-U0`
        // hunks (a pure addition vetoes whole-section balance, the
        // 7025a14 shape) group into one literal sweep via the
        // cross-hunk section driver; the addition stays ordinary
        // in place, and hetero-trailing b.rs stays ordinary.
        assert!(
            semantic.contains("rule=r1"),
            "literal cross-hunk sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- make_call(E0, E1, false)"),
            "literal minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ make_call(E0, E1, \"100644\")"),
            "literal plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    let a = make_call(\"alpha\", first_count, false);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    let a = make_call(\"alpha\", first_count, \"100644\");"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("+    let cached = extra();"),
            "pure-addition leftover must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("-    let c = make_call(\"gamma\", third_count, true);"),
            "hetero-trailing minus lines must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.contains("+    let d = make_call(\"delta\", fourth_count, symlink);"),
            "hetero-trailing plus lines must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "27-macro-interior-fires" {
        // WI-9a (was T450 veto) + WI-9b1 hetero: same-name macro
        // interiors now compress hetero — 2x assert_eq!(..) sites join
        // with the stranded claim (the pure addition forces the
        // unbalanced section). The stable `out.rsmpl` arg holes to a
        // diet `E0` (both-sides rule; single-side `TEXT_*` stay literal
        // deltas); the T450 engine-blind gate (verbatim macro veto) is
        // bypassed by the interior section driver, whose patterns route
        // to the macro matcher (probed).
        assert!(
            semantic.contains("rule=r1"),
            "macro interior must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("# macro"),
            "plain `!` routing, no directive, got: {semantic}"
        );
        assert!(
            semantic.contains("- assert_eq!(E0, TEXT_SECTION)"),
            "interior minus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("+ assert_eq!(E0, TEXT_STRIPPED)"),
            "interior plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("+#[test]"),
            "pure-addition leftover must stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "interior sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "28-verbatim-return" {
        // T452: return-shape verbatim (b9c00d5/T418 miniature) — two
        // identical `return make_widget("a", 1);` lines sweep via the
        // engine Expr::Return arm (bare-call sweeps decline: `return`
        // is not a bare call).
        assert!(
            semantic.contains("rule=r1"),
            "return verbatim sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- return make_widget(\"a\", 1)"),
            "return minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ return make_widget(\"a\", 2)"),
            "return plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    return make_widget(\"a\", 1);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    return make_widget(\"a\", 2);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "29-verbatim-struct-field" {
        // T452: struct-field verbatim (c48a6e6/T416 miniature) — two
        // identical `text: make_text(sub),` field lines sweep on the
        // stripped inner call (labels match, engine drops the label).
        assert!(
            semantic.contains("rule=r1"),
            "struct-field verbatim sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- make_text(sub)"),
            "struct-field minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ make_text_new(sub)"),
            "struct-field plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-        text: make_text(sub),"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+        text: make_text_new(sub),"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "30-verbatim-unary" {
        // T452: unary verbatim (b208847/T415 miniature) — two bare
        // `!ready(flag);` lines sweep via leading-`!` Raw bypass and
        // the structural Unary match (bare-call sweeps decline: the
        // top-level node is Unary, not Call).
        assert!(
            semantic.contains("rule=r1"),
            "unary verbatim sweep must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- !ready(flag)"),
            "unary minus pattern must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("+ !ready_done(flag)"),
            "unary plus pattern must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    !ready(flag);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+    !ready_done(flag);"),
            "rewritten lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "31-delete-whole-file" {
        // T473: whole-file item deletion (e062dda/T464 miniature) —
        // the deleted file's items sweep into one scope=file
        // delete_items hunk; module-doc lines ride ordinary leftovers.
        assert!(
            semantic.contains("rule=r1"),
            "whole-file delete sweep must fire, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r1").count(),
            1,
            "exactly one delete hunk, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn old_"),
            "deleted item lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("-//! Old helpers"),
            "non-item doc lines ride ordinary leftovers, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "32-partial-delete" {
        // T473: partial-file item deletion (7945da1/T465 prize-shape
        // miniature) — one minus-heavy unbalanced hunk (whole helper
        // items out, `mod common;` + `use common::*;` in) compresses
        // to one real-range delete_items hunk plus an ordinary
        // insertion leftover with the compensating trailing blank.
        assert!(
            semantic.contains("rule=r1"),
            "partial delete sweep must fire, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r1").count(),
            1,
            "exactly one delete hunk, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn binary"),
            "deleted item lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn init_repo"),
            "deleted item lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("+mod common;"),
            "plus lines ride the insertion leftover, got: {semantic}"
        );
        assert!(
            semantic.contains("+use common::*;"),
            "plus lines ride the insertion leftover, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "33-nested-partial-delete" {
        // T474: nested partial-file item deletion (b0da138/T466
        // prize-shape miniature) -- two mod-tests fns out, minus
        // hunk starting with a separator blank the engine never
        // absorbs: one shrunk-range delete_items hunk plus an
        // ordinary minus-blank leftover ahead of it.
        assert!(
            semantic.contains("rule=r1"),
            "nested partial delete sweep must fire, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r1").count(),
            1,
            "exactly one delete hunk, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    fn first"),
            "deleted item lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    fn second"),
            "deleted item lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("\n-\n"),
            "leading blank rides an ordinary leftover, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "34-eof-touch-delete" {
        // T474: EOF-touching item deletion (3baecc1/a2fc9b3/T472
        // prize-shape miniature) -- trailing helper out through EOF,
        // separator blank absorbed inside the minus range: one
        // real-range delete_items hunk, no ordinary leftover.
        assert!(
            semantic.contains("rule=r1"),
            "EOF-touch delete sweep must fire, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r1").count(),
            1,
            "exactly one delete hunk, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("\n@@ ").count(),
            1,
            "no ordinary leftover hunks, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn stale_helper"),
            "deleted item lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "35-impl-method-partial-delete" {
        // T475: impl-method partial-file item deletion (d426215/T466
        // prize-shape miniature) -- doc'd `lowercase_warnings`
        // method out of `impl Catalog`, minus hunk starting with a
        // separator blank the engine never absorbs: one shrunk-range
        // delete_items hunk plus an ordinary minus-blank leftover
        // ahead of it. First dedicated cover for the
        // collect_impl_method_spans path (33 covers mod-nested fns).
        assert!(
            semantic.contains("rule=r1"),
            "impl-method partial delete sweep must fire, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r1").count(),
            1,
            "exactly one delete hunk, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    pub fn lowercase_warnings"),
            "deleted item lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    /// Warn (not error)"),
            "deleted doc lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("\n-\n"),
            "leading blank rides an ordinary leftover, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "sweep must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "36-eof-touch-kept-blank-veto" {
        // T475: EOF-touch kept-preceding-blank veto (T472 veto-side
        // miniature) -- same repo shape as 34 but the separator blank
        // is kept, so the minus hunk covers only the fn lines and the
        // preceding kept blank vetoes the sweep: no rule=delete_items,
        // ordinary hunk only, cycle still byte-identical.
        assert!(
            !semantic.contains("rule=r3"),
            "kept preceding blank must veto the sweep, got: {semantic}"
        );
        assert!(
            semantic.contains("-fn stale_helper"),
            "vetoed lines stay ordinary, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "ordinary must still beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "37-move-evolved-composition" {
        // T476: move + partial-delete composition (c649b34/T471
        // prize-shape miniature) -- one verbatim fn moves cross-file
        // (cut + paste) while two in-transit-evolved fns (`super::`
        // -> `crate::`) ride one real-range delete_items hunk plus
        // an ordinary addition leftover. First full-cycle cover for
        // sweep interaction (09 covers the same-file dedup-loser).
        assert!(
            semantic.contains("rule=r1"),
            "verbatim move cut must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "verbatim move paste must fire, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("rule=r1").count(),
            1,
            "exactly one delete hunk, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn sweep_a"),
            "evolved minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("-    super::helper();"),
            "evolved minus body must be consumed, got: {semantic}"
        );
        assert!(
            semantic.contains("+    crate::helper();"),
            "evolved plus lines ride the ordinary leftover, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "composition must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "38-new-file-move" {
        // T477: verbatim cross-file move to a NEW file (the shape the
        // old unstaged harness could not host: untracked `new.rs`
        // never appeared in `git diff`, and plain `stash` left it
        // behind for --check/--output). Staged collection covers it:
        // one `move_parse_pair` cut + one `paste_parse_pair` new-file
        // paste, every minus/plus line consumed, no ordinary leftover.
        assert!(
            semantic.contains("rule=r1"),
            "new-file move cut must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "new-file move paste must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("rule=r3"),
            "verbatim move needs no delete sweep, got: {semantic}"
        );
        assert_eq!(
            semantic.matches("\n@@ ").count(),
            2,
            "no ordinary leftover hunks, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn parse_pair"),
            "moved minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn parse_pair"),
            "moved plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "new-file move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "39-tiny-move-stays-ordinary" {
        // WI-1e diet (no `# item I`): the 3-line move now wins T375
        // (byte-guard flip pinned, same as the unit tiny-move pin).
        assert!(
            semantic.contains("rule=r1"),
            "diet tiny move must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "diet tiny move must paste, got: {semantic}"
        );
    } else if name == "40-tiny-move-fires" {
        // T480: the same 3-line shape with a longer body fires —
        // emitted cut+paste is ~constant (the item rides a
        // metavar, never respelled) while replaced ordinary grows
        // with content, so the byte guard clears: no fixed line
        // threshold. First sub-4-line firing-move cover.
        assert!(
            semantic.contains("rule=r1"),
            "tiny move cut must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("rule=r2"),
            "tiny move paste must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn m() {"),
            "moved minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn m() {"),
            "moved plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "tiny firing move must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "41-single-rename-stays-ordinary" {
        // WI-1e diet terse (`- fn foo` + `# sweep`, no lang): the
        // short single now wins T375 (byte-guard flip pinned).
        assert!(
            semantic.contains("rule=r1"),
            "diet short single must fire, got: {semantic}"
        );
        assert!(
            semantic.contains("- fn foo\n"),
            "diet terse minus must fire, got: {semantic}"
        );
    } else if name == "42-single-rename-fires" {
        // T481: the same single-rename shape with a long signature
        // fires -- the `fn Old(...)` / `fn New(...)` pattern lines
        // are far shorter than the ordinary lines, so the byte
        // guard clears (unit-test `fn_rename_compresses` shape,
        // first full-cycle cover).
        assert!(
            semantic.contains("rule=r1"),
            "long single rename must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn foo("),
            "renamed minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn bar("),
            "renamed plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "single rename must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    } else if name == "43-single-use-stays-ordinary" {
        // T482: single use-prefix site veto full-cycle miniature --
        // one `use crate::old::Thing;` -> `new::Thing` line costs more
        // as a semantic hunk (rule + scope overhead) than the one
        // ordinary hunk it replaces, so no rule= hunks: pure ordinary
        // passthrough, still shorter via 0-context + header collapse.
        // Veto side of fixture 16 (3-site use_old_to_new fire).
        assert!(
            !semantic.contains("rule="),
            "single use-prefix edit must stay ordinary, got: {semantic}"
        );
        let context: Vec<&str> = semantic.lines().filter(|l| l.starts_with(' ')).collect();
        assert!(context.is_empty(), "zero-context, got: {context:?}");
        let normalized = normalize_native(native_input);
        assert!(
            semantic == normalized,
            "[{name}] ordinary passthrough must equal normalized native"
        );
    } else if name == "44-single-slot-fires" {
        // T482: single slot-sweep site with a long signature fires --
        // the `fn Old(...)` / `fn New(...)` slot pattern lines are far
        // shorter than the ordinary lines, so the byte guard clears
        // (13-shape minus one site + keep wrapper; first single-site
        // slot-sweep full-cycle cover, mirrors 42 for the slot family).
        assert!(
            semantic.contains("rule=r1"),
            "single slot rename must fire, got: {semantic}"
        );
        assert!(
            !semantic.contains("-fn old_function_name("),
            "renamed minus lines must be consumed, got: {semantic}"
        );
        assert!(
            !semantic.contains("+fn new_function_name("),
            "renamed plus lines must be consumed, got: {semantic}"
        );
        assert!(
            semantic.len() < native_input.len(),
            "slot rename must beat native bytes ({} vs {})",
            semantic.len(),
            native_input.len()
        );
    }
}
