# IMPROVE — RSmPL trust, lossless cycles, and raising breadth

## Goal

Make RSmPL the contract an agent can trust in both directions:
lowering (semantic → native, `applyrs`) must prove what it touched
and what it left alone, and raising (native → semantic, `diffrs`)
must compress the refactor shapes real Rust code actually takes,
so that semantic → native → semantic and native → semantic →
native both close losslessly under a tested canonical definition.
`gitrs` stays a thin dispatcher (`apply` → `applyrs`, `diff` →
`diffrs`); all semantics live in the crates. Backwards
compatibility is not required; breaking changes to
[SPEC.md](/home/ai/git/tools/rsmpl-git/SPEC.md) and emitted patch
text are allowed. Token efficiency matters, but never at the cost
of a guess: every new capability below fails loud or stays
ordinary when it cannot prove itself.

This review supersedes the previous IMPROVE.md. Its WI-1 (diet
RSmPL) landed and is closed; old WI-2/3/4/5 did not land and are
carried forward, re-decided where evidence changed, as WI-3/4/1/5
below. Six further items (WI-2, WI-6–WI-10) are new findings.

## Success criteria

1. Canonical cycles (WI-1) are green over the full corpus:
   state-lossless native → semantic → native (post-tree
   byte-identical) and canonical-lossless semantic → native →
   semantic (canonical forms equal; second cycle is a fixed
   point), with every exclusion named in SPEC D.5.
2. An agent can accept a patch from the JSON report alone
   (WI-2): per-rule bindings, postconditions, untouched-file
   guarantee, and per-hunk inference provenance are machine
   fields, not prose an agent must reconstruct edits to check.
3. Raising breadth grows without precision loss:
   `tools/history_sweep.py` stays at 0 failures (today 399/399
   convert) and the compressing set grows from 35 commits;
   `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`,
   and `cargo fmt --check` stay green; no `.rs` file exceeds
   2000 lines.
4. No bench in [example-benches/README.md](/home/ai/git/tools/rsmpl-git/example-benches/README.md)
   regresses, and each work item's new benches/fixtures meet the
   byte targets stated in that item (measured by
   `./example-benches/run.sh`, bytes as the token proxy).

## Review findings (evidence)

Verified this run: 111 test suites, 1228 tests passing; clippy
and fmt clean; benches at diet targets (diffrs 01 at 69 B,
applyrs 09 at 20% saved, 10 at 10%); history sweep 399/399
convert, 0 failures, 35 compressing. The architecture is sound:
[rsmpl_patch](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/model.rs)
(document/parser),
[rsmpl_pattern](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/pattern.rs)
(pattern compiler),
[rsmpl_engine](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
(original-coordinate planner),
[rsmpl_git](/home/ai/git/tools/rsmpl-git/crates/rsmpl_git/src/apply.rs)
(0-context lowering + `git apply`),
[rsmpl_diff_source](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_source/src/lib.rs)
(`git diff -U0` + untracked), and
[rsmpl_diff_infer](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/mod.rs)
(detector chain behind pre-image totality in
[totality.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/totality.rs)),
facaded by [rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs);
[gitrs](/home/ai/git/tools/rsmpl-git/bin/gitrs/src/main.rs) only
re-execs siblings. The gaps are cycle identity, trust evidence,
and inference/engine breadth — not correctness of what fires.

- **F1 Cycles are still one-directional (old F5, open).** The
  harness proves native → semantic → native by real-apply
  byte-compare ([fixtures_roundtrip.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/fixtures_roundtrip.rs),
  history_e2e). Nothing tests semantic → native → semantic:
  diffrs always re-emits `@@ -1,1 @@` + `# sweep`, renames rules
  to positional `r1…` (unstable across file reorder, per the
  WI-1e review), and drops `index` lines, hunk titles, and
  authored ranges — locality and intent are unrecoverable, and
  no canonical form defines "same patch". SPEC has no D.5.
- **F2 The report asks agents to trust, not verify (new).**
  [applyrs](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
  JSON carries `rules[].spans`, per-file `added/removed`, the
  lowered `diff`, and a boolean `check` (SPEC D.2). It does not
  carry metavariable bindings, a post-tree digest, an
  untouched-outside-spans guarantee, or what `--check` actually
  proved (`git apply --check` on the lowering, which is blind
  to semantic over-match — the T361 lesson). `diffrs` JSON
  ([diffrs](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs))
  is thinner still: `{version, files, patch}` with no per-hunk
  detector, totality evidence, or byte-win margin. Validation
  (SPEC 24) is parse + reparse only: a definition-only rename
  that orphans every caller passes `--check` clean.
- **F3 Inference still vetoes idiomatic Rust (old F3/F2, open).**
  `call_parts` rejects multi-segment, leading-`::`, `<T>`
  turbofish, and `qself` callees
  ([mod.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/mod.rs:871));
  every method detector vetoes turbofish
  ([methods.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/methods.rs));
  macro benches 17/18 save only 14–19% because invocation
  renames and interiors stay ordinary. The engine already
  matches qualified paths, structural turbofish, and macro
  renames (applyrs benches 16/17), so this is infer-only work.
  Only 35/399 history commits compress at all; 18 of 60 diffrs
  fixtures are `*-stays-*` veto pins.
- **F4 No anti-unification (old F4, open).** Verbatim sweeps
  need byte-identical single lines
  ([verbatim.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/verbatim.rs));
  call/let/type changes whose sites differ in arguments, types,
  or patterns stay ordinary. There is no type sweep, no
  signature-evolution sweep (`fn_rename_pair` requires
  byte-equal suffixes), and no multi-line template.
- **F5 General matcher still stops at control flow (old F6).**
  SPEC 18.6 R1/R2/R4 permanently route `if`/`match`/loops/
  closures, `let-else` targets, and `P @` subpatterns to
  ordinary hunks; `match_expr` in
  [matcher_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general.rs:293)
  returns `false` for those `syn::Expr` variants, and general
  patterns reject `...` (C1). Body-level edits therefore
  cannot compress in either direction.
- **F6 Renames are definition-only by construction (new).**
  SPEC 14.8 rewrites one ident span per match; callers, type
  positions, struct literals, and imports keep the old name
  while the hunk applies cleanly. The follow-up table exists,
  but composing 3–5 hunks per rename is agent toil, and
  `diffrs` never composes them either. SPEC 19 `# symbol` is
  exact full-path text with no resolver (SPEC 19.3: "no
  semantic environment in v0.1"), so there is no checked way
  to say "rename this definition and its uses".
- **F7 Moves are byte-identical and scalar (new).** Captures
  hold exactly one item's text
  ([capture.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/capture.rs));
  diffrs move inference pairs only byte-identical items
  ([moves.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/moves.rs)).
  A move that also changes visibility, a name, or a
  `super::` → `crate::` path — the common case — stays a
  full ordinary delete+insert (the largest native patches).
- **F8 Cross-file intent is lost in raising (new).** Detectors
  run per section, then per hunk; a workspace rename surfaces
  as unrelated `rN` hunks per file with no shared identity.
  Git rename sections (`rename from/to`) stay ordinary
  passthrough (T377), so a file move plus its content edits
  never compresses as one intent, even though `applyrs`
  lowers rename sections today.
- **F9 The contract surface drifts (new).** SPEC is 6198
  lines; §28/§29 examples still mix diet and long spellings
  (`# lang=rust`, `# expr E` beside canonical diet), and no
  test executes the SPEC/AGENTS examples as written, so the
  documents an agent actually reads can silently stop being
  the language the parser accepts. Error `suggestion` fields
  are prose only; there is no machine-readable repair an
  agent can apply without re-parsing English. Three source
  files sit at the 2000-line gate
  ([parser.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/parser.rs)
  at 1999, moves.rs at 1956, semantic/mod.rs at 1826), so
  every item below must split before it grows (old D5).

## Key decisions

- **D1 Losslessness is judged on file trees + canonical
  patches, never raw native bytes.** The 0-context `similar`
  lowering cannot reproduce an arbitrary hand-written native
  patch byte-for-byte, and `index`/title stripping is
  deliberate. Binary/gitlink strips (warned, SPEC D.4) are
  declared lossy and out of scope. WI-1 writes this down as
  SPEC D.5 before any breadth work depends on it.
- **D2 Trust is evidence, not a bigger diff to eyeball.**
  WI-2 extends the JSON reports with bindings, postconditions,
  and provenance instead of asking agents to re-read files.
  Default verification stays cheap (parse, reparse, span and
  untouched-region checks inside the engine); a build-level
  check is opt-in only, so `--check` never shells out to
  `cargo` unasked.
- **D3 Inference stays precision-first.** Every new detector
  keeps pre-image totality (count must equal pair count, zero
  opaque hits) and the T375 byte-win guard measured in diet
  bytes. A detector that cannot prove totality stays
  ordinary — no guessing (SPEC 2.6).
- **D4 One anti-unification core, thin detectors.** WI-4
  builds a single least-general-generalization pass over
  `syn` trees; WI-8 and WI-9 reuse it instead of growing
  per-shape string surgery.
- **D5 Split before growing.** New code lands in new modules;
  no item pushes a file over 2000 lines (the T246 precedent).
  Parser work goes to `rsmpl_pattern` or new `rsmpl_patch`
  modules, never inline in `parser.rs`.
- **D6 Familiar syntax wins over clever syntax.** New
  surface reuses Git/SmPL/Rust spellings an agent already
  knows (`# follow=uses` reads like a SmPL directive; no
  positional cryptic flags). Diet spelling stays canonical;
  the parser keeps accepting the long form so `.history/`
  archives remain readable.

## Work plan

Order is dependency order: WI-1 → WI-2 → WI-3 → WI-4 → WI-5 →
WI-6 → WI-7 → WI-8 → WI-9 → WI-10. WI-1 first because every
later emitter and harness compares canonical forms; WI-2
second because breadth without provenance repeats F2 at
larger scale. Sizes are relative (S ≤ 1 day, M ≈ 2–3 days,
L ≈ 4–5 days of focused work).

### WI-1 (P0, M): Lossless canonical cycles — SPEC D.5, range-faithful emission, S→N→S harness [carries old WI-4]

**Problem.** F1/D1: "lossless" is half-defined and half-tested.
Authored ranges, rule identity, and directive intent cannot
survive a cycle, so `gitrs diff` output cannot be trusted as a
re-appliable, re-diffable artifact.

**Decision — canonical form and emission rules (exact).**
Canonical RSmPL (what `diffrs` emits and what cycles compare):
diet spelling (`# sweep`, inferred metavars, terse renames);
directives in fixed order; hunks sorted by (file, first matched
old-line); one hunk per detector class per file. Emission range
rule: if all sites of a class lie within one contiguous
original range of ≤ 64 lines, emit that real range with
`# allow=many` (no `# scope=file`); otherwise `# sweep`. Moves
and deletes keep today's real ranges/anchors. Rule ids are
`r1…` in canonical hunk order, making the form deterministic:
same pre/post trees ⇒ byte-identical canonical patch. (The
WI-1e positional-id instability is thereby scoped, not hidden:
ids are stable for a given tree pair, and WI-2 provenance —
not the id — carries intent.) Corpus in scope: all 38 applyrs
fixtures, 60 diffrs fixtures, 40 benches, plus history_e2e
pins. Out of scope by D1: binary/gitlink sections, non-UTF-8,
and CRLF semantic compression (whose passthrough cycle must
still be tree-lossless).

**Changes.** SPEC: new D.5 "Canonical form and lossless
cycles" (definitions, corpus, exclusions) and a D.4 range-rule
bullet; Appendix A already states the diet shape and stays.
Code: `rsmpl_diff_infer` emission takes site spans (detectors
already compute them via totality; thread them through instead
of discarding) and chooses range-vs-sweep; a `canonicalize`
function in `rsmpl_patch` (parse → re-emit canonical) backs
the harness. No new CLI flags: `diffrs` output is canonical by
construction and `canonicalize` is a library + test API only.
New test `bin/diffrs/tests/cycle_e2e.rs`: per corpus case,
(a) apply authored/emitted S to the pre-tree → post; (b)
`git diff -U0` pre→post → `diffrs` → S′; assert tree(S′) ==
post and `canonicalize(S′) == canonicalize(S)` for semantic
cases (plain tree equality for ordinary cases); (c) one more
iteration asserts S′′ == S′ byte-identical (fixed point).

**Acceptance.** Cycle harness green over the whole corpus;
every failure found while building it is either fixed here
(emission determinism, range choice) or moved to an explicit
exclusion list in D.5 with a one-line reason — the list, not
the harness, is the review surface. Ranged emission must not
reintroduce range-drift failures: the 64-line rule plus
totality-by-pre-image keep each range exactly over proven
sites, and the harness real-applies every case (never
`--check` alone, per T361).

**Tests.** The harness above; `canonicalize` idempotence on
every fixture patch; directive-order determinism. Gate: full
workspace + benches + fmt/clippy + history sweep.

**Non-goals / risks.** No attempt to make native bytes
round-trip (D1). No new end-user CLI surface.

### WI-2 (P0, M): Trust envelope — verified apply report and diffrs provenance [new]

**Problem.** F2: an agent that wants to trust a patch today
must reconstruct and inspect every edit manually — exactly
what RSmPL exists to avoid. `--check` proves the *lowering*
applies; nothing proves the *semantics* did only what the
pattern said, and `diffrs` gives no reason to believe a
compression.

**Decision — additive JSON fields, no new required flags.**
`applyrs --format json` (schema stays `version: 1`; additive
keys do not bump it, per SPEC D.2) gains, per rule: `bindings`
(metavariable → matched source text, truncated at 80 chars
with a `bindings_truncated` count, so the report stays
token-bounded), and per file: `post_sha` (SHA-256 of the
post-image the engine materialized). The payload gains
`verified: {reparse: true, untouched_outside_spans: true}`,
computed by the engine: after materializing, it re-derives
each file from the original plus only the reported spans and
byte-compares — a mismatch is an engine bug and fails the run
loud, never a warning. An opt-in `--verify build` additionally
runs `cargo check --quiet` in `--repo` after a successful
apply (never under `--check`, never by default; failure exits
2 with the compiler's first diagnostic in the error envelope).
`diffrs --format json` gains per-hunk provenance by emitting,
alongside `patch`, a `hunks` array: `{rule, file, detector,
sites, totality: "pre-image"|"section-visible", saved_bytes}`.
Text mode is unchanged in both binaries.

**Changes.** SPEC D.2 (new keys, truncation rule, `--verify`
semantics) and SPEC 24 (the untouched-outside-spans check as
validation item). Code: binding capture already exists inside
the matchers — thread a bounded summary out through
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
into `RuleReport` (new fields, default-empty so existing
constructors compile); post-image hashing and the re-derive
check in the planner's pass 3; `--verify` plumbing in
[applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs);
provenance threading in `rsmpl_diff_infer` (each detector
already knows its name — return it instead of only text).

**Acceptance.** A scripted agent flow — `diffrs --format json`,
inspect only `hunks` + `verified`, then `applyrs --format json
--diff-max-lines 0 --check` — reaches the same accept/reject
decision as full-manual inspection on every fixture, including
one deliberately over-broad authored patch that must show its
extra span in `bindings`/`spans` before apply. Report growth
on the largest bench stays ≤ 15% with `--diff-max-lines 0`.

**Tests.** JSON-shape pins for every new key; a tampered-edit
unit test proving the untouched check fires; `--verify build`
against a fixture crate that does and does not compile;
provenance pins per detector family. Gate as in WI-1.

**Non-goals / risks.** No type-level proof by default
(`syn` parse + reparse only); no rust-analyzer. Risk: binding
text leaks large bodies — the 80-char truncation is the
mitigation and is pinned by test.

### WI-3 (P0, M): diffrs — qualified-path, turbofish, and macro-rename sweeps [carries old WI-2]

**Problem.** F3: the most idiomatic Rust call sites —
`crate::api(x)`, `mod::f::<T>(x)`, `recv.m::<T>(x)`,
`old_mac!(x)` — are vetoed in inference although the engine
rewrites them today (applyrs benches 16/17 prove the lowering
side). Macro benches 17/18 keep 76–86% ordinary residue.

**Decision — three detectors, in chain order after the
existing method sweeps and before `try_call_sweep` (tightest
first).** (1) `try_qualified_call_sweep`: pairs parse as
`syn::Expr::Call` whose callee path has ≥ 2 segments (leading
`::` and `qself` included); one shared old/new path delta in
exactly the last segment, prefix segments token-equal, args
per-site token-equal, uniform arity; emission is the concrete
path form `- crate::old_api(E0, …)` / `+ crate::new_api(E0, …)`
under `# sweep` — explicitly not `# symbol` (SPEC 19.3 already
prefers concrete paths; no fallback-directive tax).
(2) `try_turbofish_sweep`: bare or qualified calls and method
calls carrying `::<…>`; turbofish arguments are either
token-equal across sites (emitted verbatim) or differ only by
one type position, bound with an inferred `type` metavariable.
A bare-without-turbofish pattern never matches a turbofish
site (SPEC 18.6 R3 parity preserved in totality keys).
(3) `try_macro_rename_sweep`: pairs are single macro
invocations `old!(…)` → `new!(…)` (also `![…]`, `!{…}`) with
token-equal bodies; emits `# macro=token_tree` +
`- old!(...)` / `+ new!(...)` under `# sweep`. Macro
*interiors* are never inferred here (WI-9 owns those).

**Changes.** SPEC D.4 (three bullets; the veto paragraph
shrinks). Code: generalize `call_parts` to return the full
path (keep the single-segment wrapper for old detectors); new
`semantic/paths.rs` for detectors 1–2 and `semantic/macros.rs`
for 3 (D5 split — semantic/mod.rs is at 1826 lines);
`totality.rs` gains path+arity+turbofish-keyed counters,
reusing `opaque_hit_lines` unchanged. No engine or pattern
changes: probe first with hand-authored patches proving each
emission `--check`s and real-applies (the T430 precedent).

**Acceptance.** New diffrs benches 21–23 (qualified, turbofish,
macro rename; modelled on applyrs 16/17 repos) each ≥ 40%
saved; benches 17/18 keep their ordinary macro-interior
residue but their invocation renames now compress (17: ≥ 35%
saved). History compressing set must not shrink; any new fire
is reported by the sweep and none may fail round-trip.

**Tests.** Unit fires + vetoes per detector (prefix-divergent,
turbofish-arity-divergent, body-divergent macro, distractor
`other::old_api` untouched — mirroring applyrs bench 16);
diffrs fixtures 61+ (one fire + one veto each); every fixture
through `fixtures_roundtrip` real-apply byte-compare.

**Non-goals / risks.** No `# symbol` resolution work (WI-6).
Risk: over-matching qualified names across modules — totality
counts matchable units by full path text in the pre-image, so
a second `crate::old_api` site vetoes exactly as bare callees
do today.

### WI-4 (P0, L): diffrs — anti-unification sweeps for let/type/signature evolution [carries old WI-3]

**Problem.** F4: sites that differ in more than one slot — the
normal shape of a real refactor — have no detector. Verbatim
needs identical lines; `fn_rename_pair` needs identical
suffixes; types and `let` patterns have no sweep at all.

**Decision — one core, three thin detectors (D4).** Core
`semantic/antiunify.rs`: given N ≥ 2 `(minus, plus)` `syn` tree
pairs of the same variant, compute the least general
generalization — a template pair where every position that is
not token-equal across all sites (within minus, within plus,
or between the sides) becomes a fresh inferred metavariable
(`E0…` expr, `T0…` type, `P0…` pat, by node kind), and every
token-equal position stays literal. Guards: same `syn` variant
at every generalized node; generalization depth ≤ 2 below the
root (deeper differences stay literal-or-veto, never a
template that is mostly holes); ≥ 1 literal anchor token must
survive per side (a pure-metavariable template would match
anything and is rejected); repeated positions that differ at
one site must differ identically at all sites or stay
independent holes (SPEC 15.7). Detectors, in chain order
before verbatim (verbatim stays the tighter winner for
identical lines, T422 precedent): (1) `try_let_sweep` — root
`Stmt::Local`, generalizing pattern, type ascription, and
initializer; (2) `try_type_sweep` — root `syn::Type`, e.g.
`Vec<T>` → `Box<[T]>` across sites with varying `T`;
(3) `try_sig_evolution_sweep` — root fn header, where name
plus any of vis/qualifier/generic/param/return/where slots may
carry the delta and bodies stay untouched; this strictly
generalizes today's rename-only slot sweep, which becomes its
zero-delta special case and is deleted.

**Changes.** SPEC D.4 (three bullets; delete the rename-only
slot bullet's suffix-equality clause) and a SPEC 15.7 note
that inferred holes follow the same structural-equality rule.
Code: new `antiunify.rs` (~600 lines, own module per D5);
detectors in `semantic/generalized.rs`; `totality.rs` gains a
generic template counter — parse the emitted minus template
with `rsmpl_pattern::compile` plus an engine match-count over
the pre-image, which must equal N (reusing
`apply_patch_to_tree` on a scratch tree; sound by construction
and replacing three hand-rolled counters). Existing
call/method detectors are not rewritten onto the core here;
migration is a follow-up only if the core proves out.

**Acceptance.** New benches: diffrs 24 (let sweep with varying
args), 25 (type sweep), 26 (evolved signatures; call sites
stay separate hunks) each ≥ 30% saved. Probe corpus before
merging: the ten largest ordinary-only history commits from
the sweep report are tried; at least three must gain a
semantic hunk with byte-identical real-apply, or the guards
are tightened until precision holds (report which, in the PR
description).

**Tests.** Anti-unify unit matrix: equal→literal,
differing→hole, kind-mismatch→veto, depth-3→veto,
pure-hole→veto, repeated-hole equality. Detector fires/vetoes
mirroring applyrs benches 07/09/15 repos converted through
`diffrs`. Every fixture through `fixtures_roundtrip`
(real-apply, since `--check` is blind to over-match, T361).

**Non-goals / risks.** No statement-sequence or control-flow
generalization (WI-5/WI-9). Main risk is template
over-generality; the anchor-token plus totality-by-engine-match
guards are acceptance gates, not aspirations.

### WI-5 (P1, L): Engine — control-flow, let-else, and @-pattern matching (SPEC 18.6 R1/R2/R4) [carries old WI-5]

**Problem.** F5: any edit whose target expression *is* or
*contains as its root* an `if`/`match`/loop/closure/block, any
`let-else`, and any `P @ sub` pattern forces an ordinary hunk
that restates the whole construct — the largest remaining
authored-token sink, and the reason body-level raising (WI-4)
stops at statement roots.

**Decision — bounded structural expansion (no `...` in
general patterns; C1 stands).** Extend
`match_expr` in [matcher_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general.rs)
with arms, each field matched by the existing `match_expr` /
`match_type` / pattern matching and binding exactly as
elsewhere: `If` (condition, then-block as a statement
sequence, else branch), `Match` (scrutinee plus arms — arm
pattern via the `let` pattern matcher, guard, body), `Block`
(statement sequences; lengths must be equal — sequences are
concrete, this is not ellipsis), `Loop`/`While`/`ForLoop`
(label token-equal, body as `Block`), `Closure` (parameter
patterns and types, return type, body), and `Let` expressions
(pattern plus scrutinee); `Return` already exists.
`match_local` accepts `let-else`: the diverge block is a
`Block` arm matched structurally (usually written concretely).
The pattern matcher accepts `P @ sub` by binding `P` to the
whole target pattern text and matching `sub` recursively.
Everything not listed (async blocks, try blocks, unsafe
blocks, yield, macro types R5) stays rejected. SPEC 18.6:
R1/R2/R4 move from "permanent" to implemented rows with exact
semantics; the reopen rule ("only with a dogfood case") is
satisfied by WI-4's probe corpus, whose ordinary residue is
the dogfood — cite the three largest in the SPEC change.

**Changes.** SPEC 18.6 table, the 18.1 bounded-set note, and
the 15 pattern note. Code: the new arms plus
`match_block`/`match_pat` helpers land in a new
`matcher_general_ctrl.rs` submodule (D5 — the parent file is
at 1751 lines); `pattern.rs` `check_let_pat` admits the `@`
form. `diffrs` needs no detector change: verbatim and WI-4
templates whose roots are now matchable stop being vetoed by
`verbatim_shape_ok` / engine-match totality — update those
shape gates to ask the engine (compile + count) instead of a
hard-coded variant list.

**Acceptance.** New applyrs benches 21 (`if` condition rewrite
over 4 sites), 22 (`match` scrutinee rewrite), 23 (`let-else`
initializer rewrite) each ≥ 35% saved vs native; diffrs
counterparts or fixture pins prove the raising direction.
History sweep: report the compressing-set delta; zero
round-trip failures is the hard gate, growth is reported.

**Tests.** Per-arm matcher units (fire, field-divergent veto,
label-divergent veto, arm-count veto), `let-else` fire/veto,
`P @` binding plus repeated-`P` equality, and the SPEC 18.6
verbatim e2e expectations moving rows (not deleted). Full
fixtures + benches + cycle harness (WI-1).

**Non-goals / risks.** No `...` inside blocks, no statement
ellipsis, no macro expansion. Risk: structural `Block`
equality makes long bodies brittle — intended (SPEC 2.4
explicit syntax); authors keep ordinary hunks for body edits
and use these arms for header/scrutinee/condition shapes.

### WI-6 (P0, L): Rename completeness — `# follow=uses` with a workspace syntactic use-graph [new]

**Problem.** F6: the flagship rename is definition-only. A
struct rename that leaves every `x: Old`, `Old { … }`, and
`use …::Old` untouched "succeeds" while breaking the build,
and the agent pays 3–5 follow-up hunks (SPEC 14.8 table) plus
the file reads to aim them. `# symbol` promises precision but
delivers exact-text matching with no resolver (SPEC 19.3).

**Decision — explicit opt-in, syntactic, fail-loud.** One new
directive, `# follow=uses`, legal only on fn and item rename
hunks (any other shape is `RSMPL_PATTERN_CONFLICT` with the
repair in the reason). Semantics: after the definition match,
the engine builds a per-workspace syntactic index from `syn`
alone (no cargo, no rust-analyzer): every definition site and
every use site (type paths, value paths, struct literals,
enum-variant paths, `use` trees, impl self/trait paths) keyed
by final segment plus enclosing module path as written. A use
rewrites iff its path as written resolves — by module-path
prefix matching against the index, the same segment-wise rule
as SPEC 14.9 — to exactly the renamed definition; ambiguous
(same final segment, two candidate definitions in scope) or
opaque (inside macro bodies, string literals) sites veto the
whole hunk with `RSMPL_SYMBOL_CONFLICT` naming the site,
unless the hunk also carries `# fallback=syntax`, in which
case ambiguous sites are left untouched and listed in the
WI-2 `bindings` report as `follow_skipped`. Default behavior
without `# follow=uses` is byte-for-byte today's
definition-only rename. `diffrs` emits `# follow=uses` when a
native diff's renamed definition and its rewritten uses share
one ident delta across the patch (detected by the WI-7
grouping pass; until WI-7, `diffrs` emits today's composed
follow-up hunks unchanged).

**Changes.** SPEC 14.8 (follow semantics + veto), 19.3
(resolver scope: workspace-syntactic, explicitly not
type-directed), 9.3 (directive row), 18.6 (new C-row for
misplaced `# follow=uses`). Code: new
`crates/rsmpl_engine/src/use_graph.rs` (index + resolver,
own module per D5); planner pass between matching and
materialization that turns resolved uses into additional
ident-span edits under the same rule (so `spans` and WI-2
provenance cover them); pattern-compiler gate for the
directive. No new crate and no build-system dependency.

**Acceptance.** Applyrs bench 10 (item rename) with
`# follow=uses` rewrites definition + type + literal + import
sites in one hunk and the fixture crate still compiles
(`--verify build` from WI-2); the same bench without the
directive is unchanged. A shadowed-namesake fixture (two
`Old` types in different modules) vetoes loud without the
fallback and skips-with-report with it. History: renames that
today need the 14.8 follow-up table compress to one hunk.

**Tests.** Use-graph units per site kind; shadowing veto;
macro-opaque veto; fallback skip reporting; diffrs unchanged
until WI-7 (pinned). Full workspace + cycle harness.

**Non-goals / risks.** No cross-crate resolution, no
re-export chasing beyond one `pub use` hop (documented in
SPEC), no type inference. Risk: syntactic resolution
mis-attributes a glob-imported use — glob sites are opaque
and veto/skip, never guessed (D3).

### WI-7 (P1, M): Cross-file intent in raising — patch-level grouping and rename sections [new]

**Problem.** F8: `diffrs` thinks per section, then per hunk.
A rename whose definition sits in one file and whose call
sites sit in three others becomes four unrelated `rN` hunks;
re-diffing after a partial apply cannot tell they were one
intent, and WI-1 canonical ordering cannot group what was
never grouped. Git rename sections stay ordinary (T377), so
the single most semantic native event — a file move — is the
one thing raising never compresses.

**Decision — a grouping pass after per-section inference.**
After today's detectors run, a patch-level pass in
`rsmpl_diff_infer` (new `semantic/group.rs`) collects hunks
sharing one ident delta (`Old` → `New` as definition, callee,
use-prefix, or — once WI-6 lands — `# follow=uses` evidence)
across files and re-emits them under one shared rule name
(`rename_Old_to_New`, the pre-diet descriptive spelling,
restored for grouped hunks only: cross-file review is exactly
where a human/agent reads names, and the byte cost is paid
once per intent, not per hunk). Grouped hunks are ordered
definition-first, then uses by (file, line), which is also
their WI-1 canonical order. Rename sections: when a
`rename from/to` section's content hunks compress under any
existing detector, emit the semantic hunks against the
*new* path inside the rename section (the engine already
seeds rename content from the old side, SPEC 6/T377) instead
of declining the whole section; a rename with no compressible
content stays today's passthrough byte-for-byte.

**Changes.** SPEC D.4 (grouping bullet + rename-section rule;
the "rename sections stay ordinary" veto shrinks to the
no-compressible-content case). Code: `group.rs` pass over the
emitted patch (re-parse, regroup, re-emit — reusing WI-1
`canonicalize` ordering); `compress_section` stops declining
rename headers when the section carries hunks, keeping the
decline for headers-only renames. Provenance (WI-2) gains
`group: <rule>` on grouped hunks. No engine changes.

**Acceptance.** A two-file rename fixture (definition +
callers + file rename) raises to one named intent and cycles
green under WI-1; ungrouped output for the same input is
pinned as the veto case (divergent deltas never group).
History compressing set must not shrink; grouped re-emission
must be byte-identical under re-diff (fixed point).

**Tests.** Group fire/veto units (shared delta, divergent
delta, same ident different kinds), rename-section fire +
headers-only veto, cycle-harness coverage. Gate as WI-1.

**Non-goals / risks.** No grouping across different ident
deltas (that is WI-4 template territory). Risk: descriptive
rule names cost bytes — bounded by the T375 guard applied to
the grouped emission as a whole; a group that loses on bytes
falls back to today's `rN` hunks.

### WI-8 (P1, L): Move + evolve — near-identical moves and paste-side evolution [new]

**Problem.** F7: real moves edit while moving (visibility,
name, `super::` → `crate::` prefixes). Byte-identical pairing
misses all of them, so the largest diffs in any refactor —
whole items changing files — get zero compression, and an
agent authoring a move+evolve by hand must restate the item
or compose cut/paste with separate rename hunks and separate
range discovery.

**Decision — anti-unify the item pair, evolve at paste.**
`diffrs`: extend the move pre-pass pairing key from byte
equality to WI-4 anti-unification over the item pair — items
pair when their anti-unified template has ≥ 80% literal tokens
(measured on token counts, pinned by test) and every hole is
in an evolvable slot (fn name/vis/qualifiers, item name,
leading path segments of the header). Emission keeps today's
scalar cut (`# bind=Name`, `- I`) and makes the paste an
evolution paste: `+ Name` stays for identical moves; for
evolved moves the paste hunk carries the plus-side header
template (e.g. `+ pub fn NewName G (...) W { ... }` with the
captured body riding the existing `...` body rule), so the
engine splices captured body text into the evolved header
instead of restating it. Engine: captures gain an optional
header-template splice — the captured item's header span is
replaced by the instantiated template, body bytes preserved
verbatim. Anything outside the evolvable slots (body edits)
stays ordinary delete+insert: a move that rewrites the body
is two changes, not one move.

**Changes.** SPEC 20 (evolution paste shape + splice rule),
D.4 (move bullet: byte-identical → anti-unified pairing, 80%
gate). Code: pairing in
[moves.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/moves.rs)
moves to the WI-4 core (the file is at 1956 lines — extract
pairing into `semantic/move_pair.rs` per D5); splice in
[capture.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/capture.rs)
plus the planner paste path. Scalar captures stay scalar;
no collection syntax is introduced.

**Acceptance.** New diffrs bench 27 (move + vis/name evolve,
20-line fn) ≥ 70% saved; fixture 33 (move-evolved composition)
keeps passing. A body-editing move fixture stays ordinary
(veto pin). Real-apply byte-identical on every fire.

**Tests.** Pairing matrix (identical, header-evolved,
body-edited veto, twin veto preserved), splice units
(name/vis/path-prefix), paste blank-join unchanged
(SPEC 20.2). Full fixtures + cycle harness.

**Non-goals / risks.** No multi-item captures, no statement
moves (items only, as today). Risk: the 80% threshold admits
a lookalike pair — totality (cut-file uniqueness, as today)
plus real-apply harness coverage is the mitigation; the
threshold is a named constant with a pin, not a heuristic
buried in code.

### WI-9 (P2, L): Multi-line templates and macro-interior raising [new]

**Problem.** F3/F4 residue: verbatim is single-line by
construction, so the repeated multi-line idioms (builder
chains, `assert!` bodies, `vec![…]` literals, derive lists)
and every macro interior stay ordinary in raising, while the
engine can already rewrite macro invocation bodies it is
never handed. Benches 17/18 (14–19% saved) are this gap
measured. Authored patches hit the mirror gap: no pattern
says "this statement sequence, with these holes".

**Decision — two bounded shapes, both totality-gated.**
(1) Statement-sequence templates in `diffrs` only: the WI-4
core generalized from one root to a sequence of 2–4
consecutive statements inside one function body, emitted as a
general `let`/expr sequence hunk only when WI-5's `Block`
matching can consume it (until WI-5, this half does not ship —
the dependency is explicit, not aspirational). (2) Macro
interiors in both directions: extend the engine token-tree
matcher with one metavariable per top-level comma-separated
argument position (`old_mac!(E0, lit)` already works by
invocation matching; the new part is `diffrs` emitting it) —
`diffrs` pairs macro invocations whose name and delimiters
match and whose argument token-trees anti-unify (WI-4 core
over token trees, string/macro-opaque positions literal-only).
Interiors that do not anti-unify cleanly stay ordinary;
expansion is never attempted (SPEC 21.5 stands).

**Changes.** SPEC D.4 (two bullets), 21.3 (the "interiors go
through ordinary hunks" composition is narrowed to
non-anti-unifiable interiors, with the dogfood benches cited).
Code: sequence generalization in `semantic/antiunify.rs`
(shared with WI-4); macro pairing in `semantic/macros.rs`
(from WI-3); engine shape gates ask the compiler (as in
WI-5) instead of veto lists.

**Acceptance.** Benches 17/18 reach ≥ 40% saved; a new bench
28 (repeated 3-statement builder sequence) ≥ 40% saved.
Every fire real-applies byte-identical; the history sweep
reports the compressing delta with zero failures as the hard
gate.

**Tests.** Sequence fire/veto (length mismatch, control-flow
member veto until WI-5 shapes exist), macro-interior fire/
veto (name mismatch, delimiter mismatch, opaque-string veto),
cycle-harness coverage per fixture.

**Non-goals / risks.** No macro expansion, no `...` sequence
wildcard (C1 stands; sequences are concrete-length). Risk:
token-tree anti-unification over-generalizes inside string
literals — strings are literal-only positions by decision,
pinned by the opaque-string veto test.

### WI-10 (P1, M): Executable contract — tested examples, machine-readable repair, file splits [new]

**Problem.** F9: agents author against
[AGENTS.md](/home/ai/git/tools/rsmpl-git/AGENTS.md) and
[SPEC.md](/home/ai/git/tools/rsmpl-git/SPEC.md), but nothing
executes those examples, §28/§29 still model superseded
spellings, and every repair path is an English `suggestion`
string the agent must re-interpret. The docs are the API;
untested docs are an untested API. The 2000-line gate
(parser.rs at 1999) also means each prior item risks landing
unsplit.

**Decision — three deliverables, no new user syntax.**
(1) Example conformance: a test (`bin/applyrs/tests/spec_examples.rs`)
extracts every fenced RSmPL block from SPEC §29, Appendix A,
and AGENTS.md recipes, and asserts each parses and — where it
carries a "Matches / Does not match" claim — applies or fails
as claimed against its inline Rust snippet. §28/§29 are
rewritten in canonical diet as part of this item (closing the
WI-1e review's follow-up), and stale `…_stays_passthrough`
test/fixture names that now fire as diet are renamed to
`…_fires_as_diet` (the other WI-1e follow-up).
(2) Machine repair: the JSON error envelope gains an additive
`repair` object — `{action: "add_directive"|"widen_scope"|"read_file"|"fix_range"|"none", directive?: string, file?: string, range?: string}` —
populated for the five highest-frequency conflicts
(zero-match, too-many, range, pattern-shape, capture), with
the prose `suggestion` kept verbatim for humans. Text mode
unchanged; `version` stays 1 (additive, per D.2 rules).
(3) Splits: `parser.rs` directive parsing moves to
`rsmpl_patch::directives.rs` and `semantic/mod.rs` detector
registration moves to `semantic/chain.rs`, bringing both
under 1600 lines so WI-3–WI-9 land without gate pressure.

**Changes.** SPEC D.2 (repair object), §28/§29 diet rewrite.
Code: the extractor test (no new dependencies — fence
scanning over the two Markdown files), error-envelope
population at the existing `ensure_match_count` / range /
compile sites, and the two module splits (pure moves,
no behavior change, proven by an unmodified green suite).

**Acceptance.** Every extracted example passes as written;
deleting a directive from a SPEC example in a scratch edit
fails the suite (mutation-checked once, then reverted).
Repair objects present and pinned for the five conflicts in
both `applyrs` and `gitrs` envelopes. No file over 1600 lines
among the split set; full gates green.

**Tests.** The conformance suite itself; JSON repair pins
(text + JSON, success silence preserved); split = existing
suite unmodified.

**Non-goals / risks.** No SPEC redesign, no grammar
formalization beyond the examples, no removal of prose
suggestions. Risk: extractor brittleness on Markdown edits —
fences carry an explicit `rsmpl` info string after this item,
so extraction is by tag, never by heuristics.

## Sequencing and gates

| Item | Depends on | Size | Gate before next item |
|---|---|---|---|
| WI-1 canonical cycles | — | M | `cycle_e2e` green over corpus, or D.5 exclusion list reviewed |
| WI-2 trust envelope | WI-1 spans | M | JSON pins green; agent-flow acceptance on all fixtures |
| WI-3 paths/turbofish/macros | WI-1, WI-2 provenance | M | New benches ≥ targets; history sweep 0 failures |
| WI-4 anti-unification | WI-1, WI-2 | L | Probe-corpus criterion met; fixtures real-apply green |
| WI-5 control flow | WI-4 gates | L | 18.6 verbatim suites updated; sweep 0 failures |
| WI-6 follow=uses | WI-2 | L | Bench 10 follow fire + shadowing veto pinned |
| WI-7 cross-file grouping | WI-1, WI-6 emission | M | Grouped cycle fixed point; no compressing-set shrink |
| WI-8 move+evolve | WI-4 core | L | Bench 27 ≥ target; body-edit veto pinned |
| WI-9 multi-line/macro interiors | WI-4, WI-5 | L | Benches 17/18 ≥ 40%; sweep 0 failures |
| WI-10 executable contract | — (any time) | M | Example suite green; splits under 1600 lines |

Global validation per item: `cargo test --workspace`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`./example-benches/run.sh` (README tables regenerated only when
bytes change), and `python3 tools/history_sweep.py` (0 failures;
compressing set reported, never asserted to a fixed number
beyond "does not shrink without a named reason"). No item
lands with a red gate, a new `.rs` file over 2000 lines, or an
existing file pushed over 2000 lines.

## Risks / open questions

Per-item risks are stated with each item. Open questions:
none — scope, canonical form, detector order, guards,
thresholds (64-line range rule, 80% move pairing, 80-char
binding truncation), and acceptance numbers are decided here.
WI-4's probe-corpus outcome tightens guards empirically but
does not reopen the design; if a threshold proves wrong, the
fix is a new pin plus a SPEC note, not a silent constant edit.
