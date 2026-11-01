# IMPROVE — RSmPL as a contract that cannot say `verified` and leave code behind, round 5: verdict completeness, frame-aware raising, spelling-total binder honesty

## Goal

Make RSmPL the contract an agent can trust in both directions without
reconstructing and inspecting every edit by hand:

- **Lowering** (semantic → native, `applyrs` via `gitrs apply`) must never
  report `verified` while its own evidence lists code the rename left
  behind. `verified` means: every edit re-proved at its span, **and**
  zero orphans, zero follow-skips, zero truncated evidence in every
  rename domain the patch opened. Anything else is `partial` with the
  blocking counts named, or a loud failure.
- **Raising** (native → semantic, `diffrs` via `gitrs diff`) must compress
  the Rust agents actually write — calls, methods, fields and operator
  deltas inside one-line `fn … { … }` frames, plain-`fn` renames with
  their call sites, inherent-method renames with theirs — and must emit
  templates a human can audit (`E + 1` → `E + 2`), not bare fragments
  (`1` → `2`) whose scope only a totality proof knows.
- **Cycles must close under stated equalities**: semantic → native →
  semantic and native → semantic → native, with every remaining
  divergence named, owned, and pinned — never silent. Spelling
  differences over the same site set must compare equal; only site-set
  differences may diverge.

`gitrs` stays a thin dispatcher (`apply` → `applyrs`, `diff` → `diffrs`);
all semantics live in the crates. Backwards compatibility is not
required: breaking changes to [SPEC.md](/home/ai/git/tools/rsmpl-git/SPEC.md),
the JSON envelopes, and emitted patch text are allowed where this plan
says so. Token efficiency matters, but never at the cost of a guess or
of auditability: the cheapest *trustworthy and readable* form wins
(SPEC 2.6). Syntax stays familiar — Git hunk shape, SmPL-style `#`
directives, Rust-shaped patterns — because agents and humans already
know those spellings from training data and daily use. Diet spelling
(`# sweep`, inferred metavars, terse renames) remains canonical.
Unit tests stay inline (`mod tests` at the bottom of the owning file);
no new sibling `*_tests.rs` files (WI-19).

This review supersedes the previous IMPROVE.md (round 4) in full.
Round 4's rename-completeness core landed and is re-probed below
(WI-1 assoc definitions, WI-2 binders, WI-3 trait methods, WI-6 follow
verification, WI-7 moves); its raising and honesty halves largely did
not (frame blindness, bare fragments, `verified`-with-leftovers,
unrepairable build failures). This round carries every open residue
forward explicitly, sharpens two decisions round 4 left soft, and
adds the fresh gaps this review probe-verified at HEAD. Work stops
at 20 items.

## Success criteria

1. **No `verified`-with-leftovers (WI-1, WI-2, WI-3).** A definition-only
   `fn`/`struct` rename with live uses, a follow rename with skipped
   sites, and a binder rename in any spelling (let, closure, match arm,
   `for`/`if let`/`while let`, fn param, let-wrapped closure) each
   either complete, or report `partial` naming the blocking counts —
   never `verified`. Build failures carry `RSMPL_BUILD_CONFLICT` plus
   a machine `repair`. Probe corpora apply for real and pass
   `cargo check`; the dishonest corpora fail the verdict, not the build.
2. **Everyday Rust raises (WI-4, WI-5, WI-9, WI-18).** Calls, methods,
   fields, and operator/literal deltas inside one-line `fn` frames
   compress at the same precision as their own-line twins;
   `python3 tools/history_sweep.py` stays at 0 failures and the
   compressing set does not shrink without a named reason in the
   landing change. Emitted general templates carry their anchor frame;
   a bare-literal hunk is emitted only when the literal *is* the frame.
3. **Follow-through is complete, not asserted (WI-1, WI-6, WI-14, WI-15).**
   Inherent-method renames follow anchored call sites in one hunk, as
   trait-method renames already do; chain/capture/bound receivers
   prove or become evidence; evidence budgets report full counts
   (`orphans_total`) and lift under `--bindings full`.
4. **Cycles (WI-7, WI-8, WI-10, WI-11, WI-12, WI-18).** The `cycle_e2e`
   harness stays green; the SPEC D.5 exclusion list and the
   `KNOWN_DIVERGENT` table shrink by named families (frame-blind
   singles first, then spelling-only divergences, then the joint
   double-delta) — they never grow silently. Cross-file assoc intents,
   token-tree holes, and mixed sections re-diff to the shape that was
   applied, or stay pinned with a sharpened owner.
5. **Self-correction (WI-2, WI-15, WI-16).** Every patch/IO/build
   failure carries a machine `repair` or joins a named allow-list in
   the conformance test. `diffrs` output can be verified in one
   `gitrs diff --check` round-trip (real apply + byte-compare, T361
   doctrine: `--check` alone never proves a sweep) and raised from
   patch text (`diffrs --patch`) instead of a hand-rolled two-binary
   dance.
6. **Contract hygiene (WI-19, WI-20).** One report version across
   `applyrs`, `diffrs`, and `gitrs` errors (v5, bumped once); unit
   tests inline again with a gate against sibling test files; the
   workspace test gate stays green; every RSmPL fence in SPEC.md,
   AGENTS.md, and `skills/gitrs/SKILL.md` is machine-linted; SPEC
   corpus counts and the D.5 exclusion prose match the fixtures and
   the harness table on disk.
7. **Gates.** `cargo test --workspace`, `cargo clippy --all-targets --
   -D warnings`, `cargo fmt --check`, and `./example-benches/run.sh`
   stay green per item; no `.rs` file exceeds 2000 lines (split
   production code before growing, D5; tests stay inline per WI-19);
   bench byte targets in each item are met or the item does not land.

## Review findings (evidence)

Verified this run at HEAD `3c03eb6` (626 commits; 53 since round 4):
**2838 tests listed; `cargo test --workspace` fully green; clippy
and fmt clean; `python3 tools/history_sweep.py` converts 626/626
commits with 0 failures.** Probes below were run with the debug
binaries against throwaway `/tmp` git repos (authored
`applyrs --format json --check`, real applies followed by
`cargo check`, and pending-change `diffrs` conversions); each finding
names the probe so the landing change can re-run it as a fixture.
The architecture is sound and stays:
[rsmpl_patch](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/model.rs)
(document/parser/canonical form),
[rsmpl_pattern](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/pattern.rs)
(pattern compiler + inference),
[rsmpl_engine](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
(original-coordinate planner),
[rsmpl_git](/home/ai/git/tools/rsmpl-git/crates/rsmpl_git/src/apply.rs)
(0-context lowering + `git apply --unidiff-zero`),
[rsmpl_diff_source](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_source/src/lib.rs)
(`git diff -U0` + untracked), and
[rsmpl_diff_infer](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/mod.rs)
(detector chain behind pre-image totality in
[totality.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/totality.rs)),
facaded by [rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs);
[gitrs](/home/ai/git/tools/rsmpl-git/bin/gitrs/src/main.rs) only re-execs
siblings. What round 4 fixed, it fixed well (see the audit). What
remains is verdict completeness, frame blindness in raising, and the
single-site/spelling mass — not correctness of what fires on
own-line, follow-complete shapes.

### Carry-forward audit of round 4

| Old item | Verdict | Evidence / residue |
|---|---|---|
| WI-1 assoc impl definitions | Landed | Probe re-run with the SPEC spelling (`# sweep` + trait context + `# follow=uses`): trait decl and `impl Tr for S { type Item = u32; }` both rewrite (`span_edits: [rename, rename]`); real apply + `cargo check` passes |
| WI-2 binding honesty | Landed with spelling residue | `let total` → `sum` rewrites both block uses in-hunk, `verified` (re-probed). Residue: binder detection is spelling-keyed — a let-wrapped closure-param rename escapes it entirely (F3) |
| WI-3 method follow | Half-landed | Trait side landed beyond spec (WI-21 anchored calls, WI-23/24 call-inclusive raising): trait decl + impl def + `s.run()` move in one hunk, `verified` (re-probed). Inherent `impl S {` + `# follow=uses` still C17-rejected (F6) |
| WI-4 frame raising | Not landed | One-line-`fn` call and operator probes still re-diff as ordinary native hunks, verbatim (F4) |
| WI-5 anchored templates | Not landed | Three-site `x + 1` → `x + 2` still emits `- 1` / `+ 2` under `# allow=many` (F5) |
| WI-6 follow verification | Landed | Two-file `struct S` → `T` follow: both entries `instantiated`, global `verified` (re-probed); byte-tightened by WI-6b |
| WI-7 move verification | Landed | Cut/paste move: both rules `postcondition`, global `verified` (re-probed); WI-7a–e cover scalar/collection/evolving/insertion/member cases |
| WI-8 build truth | Not landed | No completeness downgrade (F1) and no build repair (F2): `--verify build` failure is still `RSMPL_IO_CONFLICT` + `repair: null` (re-probed) |
| WI-9 precision singles | Not landed | `KNOWN_DIVERGENT` holds 67 entries, 55 tagged `t375` (counted in [cycle_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/cycle_e2e.rs)) |
| WI-10 site-set canonical | Not landed | `01-rename-fn`, `90-macro-rules-nofollow`, `51-drift-stale-range` still divergent; `51` now precision-shapes at the actual range but canonical equality over site sets is still not the comparator |
| WI-11 joint impl intent | Not landed | Fixture `14` still pins the `impl Printable for Old` double-delta as atomic-ordinary |
| WI-12 line partition | Not landed | Fixture `12` unchanged; SPEC D.5 still calls mixed-hunk splitting a “permanent non-goal” |
| WI-13 plain-fn follow raising | Not landed | `fn foo` + two call sites re-diffs as three ordinary hunks (re-probed; not even separate sweeps — single-site demotion compounds it); `46-fn-follow` still hetero |
| WI-14 macro singles | Not landed | Literal edit inside a `macro_rules!` body still re-diffs ordinary (re-probed, single site) |
| WI-15 `# symbol` widening | Not landed | `# symbol` in a `let` hunk still C14-rejected with the generic `write_instead` (re-probed) |
| WI-16 anchors + budgets | Not landed | `a.clone().timeout` is `off-target` evidence (re-probed); 12 orphans report 8 entries + `orphans_truncated: 4`, no `orphans_total`, and `--bindings full` does not lift the cap (re-probed) |
| WI-17 patch-input raising | Not landed | [diffrs CLI](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs) still takes only `--repo`/`--staged`/`--format`/`[rev]`; no `--patch`, no `--check` |
| WI-18 evidence closure | Quarter-landed | `diffrs` JSON now lists semantic hunks (`item_intent` probe carries detector/sites); ordinary sections still emit `"hunks": []` (re-probed) and no `origin` key exists |
| WI-19 everyday batch | Quarter-landed | Menu (a) arrived as evidence-only WI-2c/2d (no completion, spelling gaps per F3); (b) `?`/`.await`, (c) derive singles, (d) use-group deltas, (e) mod+content composition untouched |
| WI-20 hygiene + v5 | Half-landed | Gate fixed (scale fixture excluded by name, counts pinned to disk: 121/116 fixtures, 32/42 benches — re-counted); `REPORT_VERSION` is still `4` and the watch files still crowd the cap (F12) |

Fixture counts on disk today: 121 applyrs fixtures, 116 diffrs
fixtures, 32 applyrs benches, 42 diffrs benches — SPEC D.5 now agrees.
Divergent table: 67 entries (55 `t375`, 6 `hetero`, 3 `partial`,
remainder named-WI notes).

### Findings

- **F1. `verified` still ships with leftovers — the verdict ignores its
  own evidence.** [global_semantics](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs:1054)
  derives the global word from rule semantics words plus
  `has_ordinary` only; the provisional-semantics gate in
  [plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
  downgrades to `ReparseOnly` only for local-binder renames
  (`is_local_binder_rename`). Probe-verified three ways: (a) `pub fn
  foo` renamed definition-only with a live `foo()` call below reports
  the call in `orphans` and still `semantics: verified`; (b) `struct
  S` → `T` definition-only with a live `S` use does the same; (c) a
  field follow with one `shadowed` entry in
  `follow_skipped_details` reports rule `postcondition`, global
  `verified`. In each case a real apply breaks the build while the
  contract word says trust it. Round 4 WI-8(1) named exactly this
  fix; only the binder slice of it landed (via WI-2's gate).
- **F2. Build failure is still the one unrepairable failure.**
  Probe-verified: `--check --verify build` on the F1(a) corpus fails
  `{"code": "RSMPL_IO_CONFLICT", "repair": null, "suggestion": null}`
  with the E0425 text buried in `reason`. There is no
  `RSMPL_BUILD_CONFLICT` in
  [errors.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/errors.rs)
  (8 codes); the WI-3 conformance allow-list still needs no entry for
  build failures only because no test drives one. The compiler is
  doing its job; the envelope gives the agent nothing to do next.
- **F3. Binder honesty is spelling-keyed, and one common spelling
  escapes it.** `closure_param_rename` in
  [use_graph_local.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_local.rs)
  recognises bare closure shapes; the let-wrapped spelling agents
  actually write does not reach it. Probe-verified:
  `- let g = |total| E;` / `+ let g = |sum| E;` rewrites the binder,
  leaves the body use `total + 1` bound inside `E` untouched, reports
  no orphans, `instantiated`/`verified` — then `cargo check` fails.
  The same keying risk covers every binder position the WI-2c/2d
  detectors enumerate by surface shape (`for`/`if let`/`while let`/
  fn-param): each new spelling is a new chance to verify-while-broken.
- **F4. Raising is still blind inside `fn … { … }` frames.**
  [side_expr](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/subline.rs:55)
  parses a changed line as one `Expr` or one `let`; a line carrying a
  `fn` header parses as neither, so every family declines.
  Probe-verified: three `foo(n)` calls inside one-line fns and three
  `v > 0` → `v >= 0` deltas inside one-line fns both re-diff as
  verbatim ordinary hunks. Compact Rust — test helpers, adapters,
  this repo's own fixtures — stays uncompressed no matter how uniform.
- **F5. The cheapest emission still wins, and it is unauditable.**
  Three sites of `x + 1` → `x + 2` (receivers `x`/`y`/`z`) still raise
  as `# allow=many` + `- 1` / `+ 2` (re-probed). Sound by totality,
  unreadable by inspection: neither human nor agent can see which
  `1`s it owns without re-running the proof. The anchored
  `- E + 1` form the engine equally accepts states its scope for a
  few bytes. SPEC 2.6 forbids guessing; nothing yet forbids
  unauditable minimality (round-4 D10, unenacted).
- **F6. Inherent-method follow is the last C17 holdout.** The gate in
  [validate.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/validate.rs)
  admits `ImplContext` follow only for trait headers
  (`is_trait_header` + `method_follow_new_name`); probe-verified:
  `impl S { fn run }` + `# follow=uses` fails C17 while the trait
  twin rewrites decl + impl def + anchored call in one hunk. The
  fallback — a bare `- E.run()` sweep — matches every `run` receiver
  in the file, including other types': precisely the guess D8 forbids.
  Everyday inherent impls are the common case; trait impls were the
  easy half.
- **F7. Two different masses still share one divergent table.** 55 of
  67 entries are `t375` single-site demotions (sound, token-wasteful);
  `01` (terse vs sweep), `90` (`allow=many` vs directive-free) and
  `51` (stale cited vs actual range) diverge on spelling over the
  same sites. Plain-`fn` raising (F8) is taxed twice: its definition
  and its calls are each single-site, so both demote and the intent
  never assembles. One mechanism cannot fix both masses; loosening
  either to shrink the other reintroduces guessing.
- **F8. Plain-`fn` intents still do not raise as intents.**
  Probe-verified: `fn foo` → `bar` plus two own-line call sites
  re-diffs as three separate ordinary hunks — the definition hunk,
  then each call alone. `item_intent` emits `# follow=uses` for items;
  for plain fns there is still “no follow emission” (`46-fn-follow`).
  The most common rename in Rust is the one the raiser cannot state.
- **F9. Evidence budgets still sample instead of counting.** The
  orphan list caps at 8 entries with `orphans_truncated: N` but no
  total; probe-verified with 12 orphans: 8 shown, `orphans_truncated:
  4`, and `--bindings full` — the documented expensive-evidence flag
  — lifts binding values only, orphans stay capped. `diffrs` JSON
  emits `"hunks": []` for ordinary sections (re-probed on the F4
  operator corpus), so “which detector claimed this, at which sites”
  is unanswerable exactly where the agent most needs it (the sections
  that did *not* compress). Follow entries still carry no `origin`;
  provenance is inferred from `semantics`, which F1 shows is the
  wrong thing to infer from.
- **F10. Anchors still stop at the first receiver.** The
  [AnchorIndex](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_anchor.rs)
  (269 lines) proves params, annotations, literal inits and `self`;
  probe-verified: `a.clone().timeout` with `a: A` proven is
  `off-target` evidence, so a field rename leaves the chained use
  behind (honestly reported, but unrewritten — and under F1 the run
  still says `verified`). Closure captures and generic bounds
  (`T: Tr`) are likewise unproven. Each unproven class is a rename
  the agent must finish by hand.
- **F11. The raising loop still has no single verify command and no
  text input.** `diffrs` accepts only git state; an agent holding
  native patch *text* (from `git diff`, a PR, another tool) must
  manufacture repo state to raise it, and checking a raise still
  takes the manual `diffrs | applyrs --check` pipe — which T361
  already proved cannot detect sweep over-match. The trustworthy
  check (real apply + byte-compare) exists only inside test
  harnesses and `tools/history_sweep.py`.
- **F12. Splits exiled the tests, and the watch files never shrank.**
  Five sibling test files now hold 2683 lines outside their owners
  (`plan_tests.rs` 650, `postcheck_move_tests.rs` 1076,
  `use_graph_tests.rs` 512, `use_graph_follow_tests.rs` 321,
  `postcheck_move_sweep_tests.rs` 124, wired by `#[path]`), against
  this project's convention that unit tests live inline in
  `mod tests` at the bottom of the owning file. Meanwhile
  `matcher_fn.rs` (1946), `postcheck.rs` (1900),
  `semantic/mod.rs` (1845), `generalized.rs` (1783), `infer.rs`
  (1766), `moves.rs` (1725), `matcher_expr.rs`/`macros.rs` (1711)
  all sit within 300 lines of the 2000-line gate, so the next
  features in this plan still cannot land without splits — and the
  last round's splits optimised line counts over navigability.
- **F13. Cross-file assoc intents cannot raise.** SPEC D.4 states it
  plainly: the `assoc_rename` detector is single-file and “cross-file
  intents stay ordinary (future work)”, while the *authored* assoc
  follow already works workspace-wide (round-4 WI-1/WI-6 machinery).
  Raising lags lowering by one file boundary for associated types,
  consts, and — by the same single-file keying — the trait-method
  grouping that WI-22c only joined within a section. Dogfood commits
  in this very repo rename assoc items across files routinely.

## Key decisions

- **D1. Losslessness is judged per origin corpus, in writing.** Native-origin
  diffs owe state-losslessness (post-tree byte-identical). Semantic-origin
  patches owe canonical-losslessness for multi-site shapes and a *defined*
  fixed-point equality (`S″ == S′`) for single-site shapes the byte guard
  demotes. Raw native bytes never round-trip (0-context lowering,
  `index`/title stripping) — declared, as today. Binary/gitlink/non-UTF-8/
  CRLF semantic compression stays out of scope; passthrough cycles must
  remain tree-lossless.
- **D2. No success field may be true by construction, and no proof may be
  a substring search.** Every `checked` claim must be computed by a check
  that can fail on real input *at the reported span* (engine re-match of
  the plus pattern, per-site bindings, orphan enumeration). Round 5
  completes the round-4 extension: a verdict computed over an *incomplete
  domain* — orphans listed, follow sites skipped, evidence truncated —
  is also true-by-construction. `verified` therefore requires zero
  leftovers in every domain the patch opened (WI-1); the rule word keeps
  describing the edit proof, the global word describes completeness.
  Both halves are pinned; neither may be inferred from the other.
- **D3. Inference stays precision-first.** Every new detector keeps
  pre-image totality (count must equal pair count, zero opaque hits) and
  the T375 byte-win guard measured in diet bytes, except a bounded,
  pinned precision guard (definition shapes today; uniqueness-proved
  singles in WI-7). A detector that cannot prove totality stays
  ordinary — no guessing (SPEC 2.6).
- **D4. One anti-unification core, thin detectors.** New raising work
  reuses and extends the existing
  [antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
  core and the sub-line extractors instead of growing per-shape string
  surgery. Engine work is structural `syn` matching — concrete,
  fail-loud, no expansion, no guessing.
- **D5. Split production code before growing; never exile tests.** New
  code lands in new modules; no item pushes a file over 2000 lines, and
  any file over 1600 lines touched by an item leaves under 1600 or with
  its new code in a sibling module. Unit tests stay inline (`mod tests`
  at the bottom of the owning file, however large the owner grows —
  split the *production* code, not the tests); integration/e2e tests
  under `bin/*/tests/` and `crates/*/tests/` are unaffected. The watch
  list is F12's eight files plus
  [chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs)
  (1557) and [main.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs) (1599).
- **D6. Familiar syntax wins over clever syntax.** New surface reuses
  Git/SmPL/Rust spellings (`...`, `#` directives, concrete paths, context
  headers that look like the block they select, attr lines written where
  Rust writes them). Diet remains canonical; the parser keeps accepting
  long forms so `.history/` archives stay readable. Any new spelling must
  be explainable in one AGENTS.md row. Round 5 adds no new directive
  keywords: inherent-method follow reuses `# follow=uses`; frame raising
  changes no spelling at all; the only new surface is two `diffrs`
  flags (`--patch`, `--check`, WI-16) mirroring `applyrs` spellings the
  agent already knows.
- **D7. Breaking changes are taken where they buy trust.** JSON schema
  stays at its current version for additive keys; a key whose *meaning*
  changes bumps the version with the old key removed, per SPEC D.2's own
  rule. SPEC text moves with code in the same item. This round bumps the
  unified report to `version: 5` exactly once (WI-20) after WI-1/WI-2/WI-15
  have settled the verdict and evidence shapes it carries. The tightened
  `verified` meaning (D2/WI-1) is the headline v5 change and is called
  out as breaking in SPEC D.2.
- **D8. Anchors before indirection.** Follow, symbol resolution, and
  member anchoring prove site ownership from text the change already
  contains — fn-signature param types, `let` annotations, struct-literal
  paths, `use` chains, the enclosing `impl`/`trait` header — before
  consulting any wider graph. A site no anchor proves is evidence
  (`orphans`/`follow_skipped` with a reason code), never a rewrite and
  never a veto on the proven sites.
- **D9. A rename owns its secondary definitions.** For every rename kind,
  SPEC enumerates the *definition set* (trait decl + impl definitions
  for associated items and trait methods; the binding + its scoped uses
  for locals) and the follow/orphan machinery must account for every
  member: rewritten, skipped with a reason, or the run degrades (WI-1).
  “Use sites” never silently excludes definitions again. Round 5 adds
  the inherent-method definition set (the `impl S` method + its anchored
  calls, WI-6).
- **D10. Auditability outranks byte-minimality.** When two emissions
  prove the same site set, `diffrs` emits the one whose pattern states
  its frame (holes + anchors), even at a small byte cost. Bare-fragment
  emissions survive only where the fragment is the whole matched unit.
  Token targets in benches are re-pinned to the auditable form, never
  met by stripping anchors (WI-5).
- **D11. Binder honesty is keyed by binding, not by spelling.** Any
  pattern whose minus/plus pair renames exactly one bound binder —
  however spelled (let, closure, arm, loop, parameter, nested inside a
  `let` init) — enters the same completion/evidence/downgrade path
  (WI-3). Detection reads the compiled pattern's binder identity, never
  a list of surface shapes, so the next spelling cannot silently opt
  out. A binder rename the compiler cannot identify as such is a
  general template and must not touch binder-shaped text at all.
- **D12. `partial` must name its blockers.** A non-`verified` run states,
  per blocking rule, the counts that block it (orphans, follow-skips,
  ordinary hunks, unprobed shapes) in text (WI-1) and as JSON totals
  (WI-15, formalised at v5 in WI-20). “Somewhere in these 40 files” is
  not evidence; the agent's next action must be computable from the
  report alone.

## Work plan

Order is dependency order: WI-1 → WI-2 → WI-3 → WI-6 → WI-4 → WI-5 →
WI-7 → WI-8 → WI-9 → WI-14 → WI-15 → WI-16 → WI-18 → WI-10 → WI-11 →
WI-12 → WI-13 → WI-17 → WI-19 → WI-20. The verdict fixes (WI-1–WI-3)
precede everything that must be certified by them; inherent follow
(WI-6) precedes the anchor growth (WI-14) that widens its receiver
proof; frame raising (WI-4) precedes the anchored-emission and
single-site work (WI-5, WI-7) that spend its site sets; WI-20 last
because it bumps the unified schema once. Sizes are relative
(S ≤ 1 day, M ≈ 2–3 days, L ≈ 4–5 days of focused work).

### WI-1 (P0, M): `verified` means complete — leftovers block the word [carries F1; completes R4-WI-8(1), D2/D12]

**Problem.** F1/D2: the global verdict counts edit proofs and ignores
rename completeness, so the three most common agent operations —
definition-only rename, follow rename with skips, wide rename with
truncated evidence — report `verified` while listing the very code
that breaks the build. The binder-only downgrade that landed proves
the mechanism works; it was simply never generalised.

**Decision — completeness is part of the global word, computed from
engine counts.** `checked.semantics` is `verified` iff (a) every rule
is `instantiated`/`postcondition` (today's rule), (b) every rule's
engine orphan count is zero, (c) every rule's follow-skip count is
zero, and (d) no rule's evidence was truncated (orphan, site, or
binding caps all unhit). Otherwise the word is `partial` and the
report names blockers: text mode prints one
`incomplete: rule <name> (<file>): <n> orphans, <m> follow-skipped`
line per blocking rule on stderr (JSON mode stays stdout-only, one
object); JSON adds nothing in this item — counts are read from the
engine report before JSON truncation, and WI-20's v5 envelope carries
them as first-class keys. Rule `semantics` words do not change (they
describe the edit proof, D2); follow hunks, which carry no orphans
by construction, block through their skip counts. A rename with
`# optional` and zero matches stays `not-applicable` and never blocks.
This is a breaking meaning change: SPEC D.2/24.6 state it under v5
(WI-20 executes the bump; this item lands the derivation behind the
current version with the SPEC note written here).

**Changes.** SPEC 24.6 (completeness clause), D.2 (global-word
definition). Code: `global_semantics` in
[main.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
learns orphan/skip/truncation inputs from `ApplyReport` (thread the
pre-truncation counts through `RuleReport` in
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs);
do not parse the capped JSON arrays back). No engine matching changes.

**Acceptance.** All three F1 probes now report `partial` with correct
blocking counts and apply byte-identically to today; the clean twins
(same corpora with `# follow=uses`, or uses deleted) still report
`verified`; the WI-2 binder matrix keeps its exact current words in
both directions (completion → `verified`, leftovers → `partial`).

**Tests.** Verdict matrix inline in `bin/applyrs/src/main.rs`
`mod tests` (clean / orphaned / skipped / truncated × fn / item /
member / assoc / binder) plus e2e pins in
`bin/applyrs/tests/checked_semantics_e2e.rs` extended, never forked;
bench semantics words (40–42, 02/31) re-pinned where — and only
where — leftovers exist, each named in the landing change.

**Non-goals / risks.** No always-on build (rejected below); no new
rule words. Risk: honest patches that intentionally leave namesakes
now read `partial` — that is the point; the blocker line tells the
agent the leftover is expected or not, which silence never could.

### WI-2 (P0, S): Build failures are repairable — `RSMPL_BUILD_CONFLICT` [carries F2; completes R4-WI-8(2)]

**Problem.** F2: `--verify build` is the only failure class with
`repair: null`, under a code (`RSMPL_IO_CONFLICT`) that means
something else. An agent that did everything right — checked,
verified, escalated to the compiler — gets no next action.

**Decision — one new code, one repair shape, two call sites.**
`RSMPL_BUILD_CONFLICT` (new `ErrorCode::BuildConflict` in
[errors.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/errors.rs))
covers `cargo check` failures after a successful lowering, in both
the `--check` temp-copy path and the post-apply path, with identical
envelopes. `repair` is
`{"action": "read_file", "file": <first diagnostic path>,
"range": "<line>..<line>"}` parsed from the first `cargo check`
diagnostic; when no diagnostic parses, the repair falls back to the
first orphan's file/line (the orphans hint already computed), and
only when neither exists may `repair` stay null — that residue joins
the WI-3 conformance allow-list by name in this item. A missing
`Cargo.toml` or missing `cargo` binary keeps `RSMPL_IO_CONFLICT`
(environmental, allow-listed). `applied: true/false` keeps its
current meaning; the orphan-context suffix in `reason` is kept.

**Changes.** SPEC 25 (new code + repair shape), D.2 (error table).
Code: diagnostic parser beside `run_verify_build_with_orphans` in
[main.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
(extract to a sibling production module if the file would cross
1600, D5 — tests stay inline in the owner); `CliError::verify`
carries the new code.

**Acceptance.** The F2 probe fails with `RSMPL_BUILD_CONFLICT` + a
`read_file` repair naming the E0425 call site, in both check-mode
and apply-mode; a green `--verify build` is byte-identical to today;
missing-`Cargo.toml` keeps its allow-listed shape and test.

**Tests.** Build-repair pins (diagnostic-parse / orphan-fallback /
environmental) in `bin/applyrs/tests/verify_build_e2e.rs` extended;
conformance enumeration in `wi3_repair_complete_e2e.rs` gains the
build case explicitly (no silent allow-list growth).

**Non-goals / risks.** No test/clippy verify modes, no diagnostic
translation beyond the first diagnostic. Risk: `cargo` output format
drift — the parser keys on the stable `error[E…]`/` --> file:line`
lines and degrades to the orphan fallback, pinned with a recorded
fixture, never a panic.

### WI-3 (P0, L): Binder honesty keyed by binding — every spelling completes or confesses [carries F3; completes R4-WI-19(a), D11]

**Problem.** F3/D11: WI-2/2c/2d enumerate binder shapes by surface
form, so each spelling is a separate honesty implementation — and
the let-wrapped closure spelling already escaped. Closure/arm
binders are still evidence-only: they confess (when recognised) but
never complete, forcing hand-edits for a rename the engine can scope
exactly.

**Decision — one binder path, keyed on the compiled binder.**
`rsmpl_pattern` records, for every `PatternKind` that renames exactly
one binder and nothing else, a `BinderRename { old, new, scope }`
(scope: enclosing block / closure body / arm body / loop body / fn
body) at compile time; spellings that cannot prove single-binder
identity (destructuring deltas, init/type/body changes, `move`
qualifier deltas) record `None` and keep general-template behaviour
with no binder privileges. The engine path is then singular:
same-scope unshadowed value uses rewrite in-hunk (labelled
`follow`); shadowed, macro-hidden, nested-fn and out-of-scope uses
become orphans; leftovers block `verified` through WI-1 (the
binder-local `ReparseOnly` shortcut is removed — one mechanism, D12).
Closure-param and match-arm binders gain completion over their body
scope in this item (round-4 WI-19(a)); `for`/`if let`/`while let`
and fn-param binders gain completion over their already-evidenced
scopes. The F3 let-wrapped closure is the headline case: the binder
inside a `let` init is found by walking the compiled init pattern,
not by adding a sixth surface detector.

**Changes.** SPEC 18.1/`let` rows + pattern rows (binder definition
set per D9, completion scopes, D11 statement). Code: binder identity
in `crates/rsmpl_pattern/src/pattern.rs` (compile-time extraction,
new sibling `pattern_binder.rs` if needed, D5); single engine path
consolidating `use_graph_local.rs`, `use_graph_scope.rs`,
`use_graph_binder.rs` call sites in
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs);
scope completion reuses WI-2b's block walker.

**Acceptance.** The F3 probe completes (`|sum| sum + 1`) and reports
`verified`, or — with a shadowed inner use added — rewrites the
proven uses, lists the rest, and reports `partial`; every WI-2/2c/2d
fixture keeps its verdicts (completion cases upgrade from
evidence-only to rewritten, each listed); a destructuring rename
with a same-name use left behind reports `partial` via WI-1 orphans
even though it takes no binder path (general orphan scan covers it).

**Tests.** Spelling × outcome matrix (6 binder positions ×
complete / shadowed / macro / escaped) inline in the owning modules'
`mod tests`; durable e2e fixture mirroring WI-1's (real apply +
`cargo check`) for the let-wrapped closure; no test moves out of its
owner file (WI-19).

**Non-goals / risks.** No dataflow/borrow analysis — scopes stay
syntactic (block/body + shadowing, D8). Risk: a general template
misread as a binder rename would rewrite uses it never promised —
the compile-time gate requires token-equality outside the one binder
ident, veto-pinned per position.

### WI-4 (P0, L): Frame-aware raising — detectors see inside `fn` items [carries F4; R4-WI-4 unchanged in substance]

**Problem.** F4: any changed line that also carries `fn`, braces, or
a second statement is invisible to raising, so compact Rust — the
default style of tests, closures-heavy code, and this repo's own
fixtures — stays ordinary no matter how uniform the rewrite is.

**Decision — frame extraction before line classification.** A new
pre-pass in [chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs)
splits each balanced minus/plus line pair that fails whole-line
classification into (frame, sites): parse the line as a `syn::Item`
or statement sequence; when minus and plus frames are token-equal
outside a set of same-family subterm positions (the existing sub-line
families plus binary/unary/field/index general-expression subterms),
the pair contributes those subterms as sites and the frame joins the
totality veto (any frame token delta, or a same-line site left
unchanged, vetoes the pair). Extraction reuses the existing family
collectors in [subline.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/subline.rs);
only the *line splitter* is new (sibling production module
`frame.rs`, D5). Emission is unchanged — the subterm hunk over the
proven site set — except as WI-5 reorders template preference.
Totality counts subterms in the pre-image exactly as today; frames
add no new trust, only new sight.

**Changes.** SPEC D.4 sub-line paragraph (frame rule); no authoring
change (engine matching already handles these sites — the emitted
hunks re-apply today, probe-verified in the landing change). Code:
`frame.rs` splitter; `side_expr` gains an item/statement-sequence
entry; chain wiring behind the existing “whole-line failed uniformly”
gate so no own-line class changes winner.

**Acceptance.** Every F4 probe pair compresses (call, let-inline,
tail-call, method, field, operator-delta) and each emitted patch
real-applies byte-identical to the true post-tree (T361 compare,
not `--check` alone); a frame with a header delta (`fn f0` →
`fn g0` alongside the call rewrite) stays ordinary; benches gain a
compact-style diffrs bench (43-series) at the byte target set in
the landing change.

**Tests.** Frame matrix (fn-brace / let-sequence / tail / guard /
distractor-twin veto / frame-delta veto) inline in `frame.rs`
`mod tests`; totality twins inside frames veto; history sweep re-run
with no shrink.

**Non-goals / risks.** Multi-line items whose changed lines are
proper own-line statements already work and keep their winners —
frames only claim lines nothing else parsed. Risk: over-splitting a
line holding two different families — mixed-family lines veto
(single-family rule, as today).

### WI-5 (P1, M): Anchored templates beat bare fragments [carries F5; enacts D10]

**Problem.** F5/D10: chain order lets the literal detector emit
`- 1` / `+ 2` for an `E + 1` rewrite. Sound, minimal, and unreadable;
an agent reviewing the semantic patch — the product's core promise —
cannot tell what it owns.

**Decision — template preference by auditability, pinned.** Detector
chain order for overlapping general/literal classes becomes:
anchored general template (≥1 hole + ≥1 anchor token) → arg/method
templates → literal-frame → bare literal. A bare-literal (or
bare-ident-only) emission is legal only when the matched unit *is*
the literal (whole-line literal sites, macro-arg literal sites) —
never when it was extracted as a subterm of a larger expression
that an anchored template also proves. Where both prove the same
site set, the anchored form's extra bytes are spent deliberately;
bench expectations re-pin to the anchored bytes and the delta is
reported in the landing change (expected: single-digit bytes per
site). Canonical form (D.5) treats the two spellings as distinct —
cycles therefore pin the anchored `S′` as the fixed point, and
authored bare forms re-diff to anchored (documented, like
terse→sweep normalisation).

**Changes.** SPEC D.4 (preference rule + rationale), D.5 (fixed-point
note). Code: chain ordering + a shared “same site set” comparison
in [chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs);
literal detector gains the “extracted-subterm” veto in
[literal_frame.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/literal_frame.rs).

**Acceptance.** The F5 corpus emits `- E0 + 1` / `+ E0 + 2`
(diet holes, SPEC 15.8) and real-applies byte-identical; whole-line
literal fixtures (the `17-same-literal` family) still emit the bare
pair; no fixture's site set changes — only spellings, each listed.

**Tests.** Preference pins per overlapping class inline in the
owning detectors; site-set equality assertions (old vs new emission
match identical pre-image spans); bench table re-pin with byte
deltas named.

**Non-goals / risks.** No change to authored-pattern matching: a
hand-written `- 1` hunk still applies exactly as today. Risk:
anchored templates cost more on huge literal sweeps — bounded by
the ≥3-site bench (log-message bench 36) holding its target within
the re-pinned tolerance.

### WI-6 (P0, M): Inherent-method follow — the last C17 holdout closes [carries F6; completes R4-WI-3, D9]

**Problem.** F6: trait-method follow is complete (decl + impl defs +
anchored calls in one hunk, re-probed), but the inherent twin — the
common case — is C17-rejected, so agents hand-write the unanchored
`- E.run()` sweep that matches every same-named receiver in the file.

**Decision — one method domain, trait requirement dropped where it
was never load-bearing.** `# follow=uses` becomes legal on
inherent `impl S {` fn renames (`ImplContext` with a non-trait
header). The claim set is: the impl method definition itself
(`rename` label), plus method-call sites whose receiver type is
proven to be `S` by the existing anchor order (fn-param `s: S` /
`s: &S`, `let x: S`, `let x = S { … }`, `self` inside `impl S`,
`S::old` / `<S as …>::old` tails). An inherent same-name method on
another proven type is `shadowed` evidence; unproven receivers are
`off-target`; neither vetoes (D8). Trait-header renames keep WI-21
behaviour exactly (trait-tail proof additionally required for impl
defs). Evolution shapes and name-holed (`N`) method renames keep
the C17 rejection. Definition-only inherent renames gain orphan
parity via `method_orphans_for`: leftover anchored callsites report
as orphans and block `verified` through WI-1. Raising: the
trait-method detector's inherent sibling emits the follow form when
its scratch-apply probe proves the call-site set, else today's
definition hunk alone.

**Changes.** SPEC 14.8 (inherent-method follow paragraph, definition
set), 18.6 C17 row (legal list gains inherent `ImplContext`).
Code: gate widening in
[validate.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/validate.rs)
and the mirrored check in `use_graph_follow.rs`; inherent arm in
[use_graph_method.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_method.rs)
reusing `AnchorIndex` unchanged; detector emission in
`trait_methods.rs` / `methods.rs` under
`crates/rsmpl_diff_infer/src/semantic/`.

**Acceptance.** The F6 probe rewrites def + `s.run()` while a
same-name method on another struct is untouched `shadowed`
evidence, and a real apply + `cargo check` passes; definition-only
inherent rename lists the anchored call as an orphan and reports
`partial` (WI-1); trait probes are byte-identical to today.

**Tests.** Inherent anchor matrix (param / annotation / literal /
`self` / tail × claim / shadow / off-target) inline in
`use_graph_method.rs` `mod tests`; C17 accept/reject pins (inherent
rename accepts, inherent evolution and `N`-hole still reject);
durable apply+build e2e fixture.

**Non-goals / risks.** No dynamic-dispatch proof: `dyn` receivers
prove through their declared type only. Risk: generic self types
(`impl<S> Vec<S>`) — claimed only on syntactic last-segment equality
with the header's self tail, else evidence; pinned.

### WI-7 (P1, L): Precision singles II — safe shapes fire at N=1 [carries F7-precision; R4-WI-9]

**Problem.** F7: 55 `t375` entries demote single-site sweeps whose
only sin is having one site. The precision guard proved the pattern
for definition shapes (the template *is* the whole unit, so it
cannot match wider than the diff); call/method/general singles were
left under the byte guard because a bare `foo(E)` *can* match
siblings the diff never touched.

**Decision — uniqueness-proved singles.** A single-site class in the
call, method-call, general-expression, and let families fires under
a new `PrecisionUnique` guard iff (a) the emitted template, run
through the real engine probe against the pre-image, matches
*exactly one* site workspace-file-wide (not merely section-wide),
(b) that site is the diff site, and (c) the template carries at
least one anchor token beyond the hole(s) (post-WI-5 spellings).
The guard is strictly stronger than totality-at-N (which counts
only the changed file's section); the probe reuses
`template_pair_holds` over the whole file. Shapes failing (a)–(c)
stay ordinary — the byte guard is untouched for them. Emission
follows the ranged rule (tight range, `allow=one` semantics: no
`# allow=many` line; the hunk is a single-site semantic hunk,
documented in D.4 as the precision-single spelling).

**Changes.** SPEC D.4 (guard definition + spelling), D.5
(single-site equality extended to the new families). Code: guard
plumbing in [totality.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/totality.rs)
and per-family gates in `calls.rs`/`methods.rs`/`general_expr.rs`
under `crates/rsmpl_diff_infer/src/semantic/`; no engine change
(the authored single-site forms already apply — probe-verified
class by class in the landing change).

**Acceptance.** Each landing slice names the `KNOWN_DIVERGENT`
entries it flips (expected first slice: single call/method/general
entries whose probes prove uniqueness — the change lists them, the
table shrinks by exactly that list); a single-site diff with an
untouched same-shape sibling anywhere in the file stays ordinary
(veto pin); `S″ == S′` fixed point holds for every flipped entry.

**Tests.** Guard matrix per family (unique fires / sibling vetoes /
anchorless vetoes) inline in each detector's `mod tests`; cycle
harness re-pin; history sweep (new compression reported, never
asserted).

**Non-goals / risks.** No N=1 for verbatim/macro/token-tree shapes
(WI-12 owns macro singles) and none for hetero classes. Risk:
whole-file probes cost time on large files — bounded by the WI-20
scale gate; the probe parses once per file (existing discipline).

### WI-8 (P1, M): Site-set canonical equality — spelling stops diverging [carries F7-spelling; R4-WI-10]

**Problem.** F7: `01` (terse vs sweep), `90` (`allow=many` vs
directive-free), and `51` (stale cited range vs actual range)
diverge although pre/post trees and matched site sets are
identical. The canonicaliser compares spellings; the contract
cares about sites.

**Decision — canonicalise by proven site set.**
`canonicalize_with_tree` already re-emits `allow == many` hunks by
matched spans; extend the same treatment to (a) terse/directive-free
rename hunks whose single matched span set equals a sweep's (emit
the diet sweep form — the form `diffrs` emits — regardless of
authored spelling), and (b) ranged hunks whose proven spans differ
from their cited range (emit the actual-span range under the
existing 64-line rule). Two hunks compare equal iff their site sets,
templates (post-diet), and file sets are equal. Pure
`canonicalize` (no tree) keeps preserving authored locality — only
the tree-aware comparator, which already promises this
normalisation, learns the two cases. SPEC D.5 states the site-set
definition explicitly and removes `01`/`90`/`51` from the exclusion
prose.

**Changes.** SPEC D.5. Code:
[canonical.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/canonical.rs)
(tree-aware re-emission widened) via the engine entry in
`canonical_tree.rs`; harness expectations in
[cycle_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/cycle_e2e.rs).

**Acceptance.** `01`, `90`, `51` leave `KNOWN_DIVERGENT` with tree
legs unchanged; an authored patch and its re-diff over the same
sites canonicalise byte-identical in both authored directions
(terse→sweep and sweep→terse probes).

**Tests.** Canonical equality matrix (spelling pairs × range
staleness) inline in `canonical.rs` `mod tests`; negative pin:
different site sets never unify.

**Non-goals / risks.** No change to application semantics — ranges
still cite original coordinates at apply time (SPEC 10); only the
*comparison* normalises. Risk: unifying a hunk whose range was
load-bearing for disambiguation — the tree-aware path only fires
when the engine proves a unique site set, else authored form
stands (existing fallback).

### WI-9 (P1, M): Plain-`fn` intents raise with follow [carries F8; R4-WI-13]

**Problem.** F8: `struct` renames raise as one `# follow=uses`
hunk; the identical `fn` rename + call sites re-diffs as three
ordinary lines (each single-site half demotes separately, so the
intent never assembles). The authored follow has existed since
WI-6a; only the raiser cannot state it.

**Decision — `item_intent` learns the fn case.** When a section's
changed lines contain exactly one `fn` definition pair
(`fn old…` → `fn new…`, same slots) plus call/method sites of
`old` that the follow domain (the landed anchor order + bare-call
sites) proves belong to it, and the scratch-apply probe (the existing
T361 real-apply byte-compare over the inferred follow patch)
reproduces the post-tree, emit one hunk: the terse fn rename
with `# follow=uses` (plus `# scope=file` when the site spread
exceeds the 64-line rule). Any unproven same-name call site in
the file vetoes to today's behaviour (definition sweep + call
sweep separately, or ordinary) — never a partial follow claim.
Cross-file fn intents use the WI-18 workspace probe exactly as
item intents do. Single-site fn intents (def + one call) fire
under WI-7's `PrecisionUnique` guard with the same veto discipline.

**Changes.** SPEC D.4 (`item_intent` fn arm). Code:
[item_intent.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/item_intent.rs);
follow-domain reuse from the engine via the existing probe
facade in [rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs).

**Acceptance.** The F8 probe compresses to one follow hunk and
cycles (`46-fn-follow` leaves the hetero list or the landing
change names its remaining divergence; `89-fn-follow-single`
flips with it or is named); a same-name call on an unrelated
receiver vetoes to separate hunks (pin); bench 40-series gains
the fn counterpart at its byte target.

**Tests.** Intent matrix (bare calls / anchored methods / mixed
veto / cross-file / N=1) inline in `item_intent.rs` `mod tests`;
fixed-point pins; history-sweep spot pins.

**Non-goals / risks.** No follow emission for fn *evolution*
(signature changes rename nothing — C17 stands). Risk: call
sites that are actually macro-generated — opaque veto, as today.

### WI-10 (P2, M): Joint double-delta intent — `impl Trait for Type` renames as one [carries F7-joint; R4-WI-11]

**Problem.** `impl Printable for Old` → `impl Labeled for New`
changes two names in one intent; every single-delta detector
declines and partition atomicity keeps the whole section ordinary
(fixture `14` residue: 8 semantic + 1 ordinary is the ceiling
today).

**Decision — one joint detector, two renames, one hunk pair.**
A new `impl_joint` detector claims a minus/plus line pair (or
impl-header context pair) iff it parses as `impl <Tr> for <Ty>`
on both sides with exactly those two segment deltas and nothing
else, *and* the section (or, cross-file, the WI-18 workspace
probe) also contains the corresponding `trait Tr` → `trait Tr2`
and/or `struct/enum Ty` → `Ty2` definition pairs. Emission is the
grouped form agents already know: one definition hunk per renamed
item with `# follow=uses` (the `item_intent` grouping), where the
impl header line itself is carried as a follow site of *both*
renames (its two segments labelled accordingly) instead of a
third hunk. If either definition pair is absent, the impl pair
stays ordinary (an impl-only retarget is a different, rarer edit
and keeps today's spelling). Authoring needs no new syntax: two
ordinary rename hunks with `# follow=uses` already produce this
lowering; WI-10 is raising + the SPEC statement that the impl
header is a shared follow site.

**Changes.** SPEC D.4 (`impl_joint`), 14.8 (shared follow site).
Code: detector in
[impl_single.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/impl_single.rs)
sibling `impl_joint.rs`; follow-site labelling reuses the landed
follow span probe (round-4 WI-6) (two labels, one span —
`span_edits` gains the pair form, additive until WI-20's v5).

**Acceptance.** The `14` corpus re-diffs to 9 semantic + 0 ordinary
(or the landing change names the surviving ordinary line and
why); the three-line probe (`trait`+`struct`+`impl`) compresses
and real-applies byte-identical; impl-only retarget stays
ordinary (pin).

**Tests.** Joint matrix (both defs / one def / neither / generic
impl veto) inline in `impl_joint.rs` `mod tests`; cycle flip for
`14`; partition interaction pins.

**Non-goals / risks.** No general multi-delta anti-unification —
exactly two named segments, exactly the impl-header shape.
Risk: an impl pair whose trait/type pairs live in other files —
the WI-18 probe decides; unproven stays ordinary.

### WI-11 (P2, L): Partition degrades per-pair, mixed hunks split at detector boundaries [carries F7-hetero; R4-WI-12]

**Problem.** Round 3 declared “a single mixed native hunk never
splits” a permanent non-goal (fixture `12`, header+let). The cost
is real: one unclaimable pair in a native hunk keeps every
claimable pair beside it ordinary, because `git diff -U0` merges
adjacent changed lines into one hunk regardless of shape boundaries.
SPEC D.5 still enshrines the non-goal, so the ceiling is contractual,
not accidental.

**Decision — split by lines, never by guessing.**
`compress_section_per_hunk` learns a line-level partition: within
one balanced native hunk, maximal runs of consecutive minus/plus
line pairs sharing one detector verdict (whole-line detectors
only — no sub-line, no frame extraction inside a split) emit as
semantic hunks with tight sub-ranges; runs with no verdict stay
ordinary *in place*, preserving original line order and SPEC 10.6
disjointness by construction. The `12` class (fn-header pair +
`let` pairs in one hunk) therefore compresses its let run and
keeps the header pair ordinary until a header detector claims it.
Atomicity is kept where it matters: a single *pair* never splits
(the actual WI-19c invariant — one line pair, one verdict), and
any run whose totality depends on section-wide counting stays
whole-section or ordinary. SPEC D.4/D.5 rewrite the “permanent
non-goal” prose to state this boundary.

**Changes.** SPEC D.4 (line-partition rule), D.5 (`12` exclusion
rewritten). Code: partitioner in
[partial.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/partial.rs);
per-hunk driver in
[chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs).

**Acceptance.** Fixture `12` re-diffs mixed (semantic let run +
ordinary header pair) with byte-identical real apply; every
existing partial-partition fixture keeps its verdicts; no
whole-section winner changes (regression pins).

**Tests.** Partition matrix (pure / mixed / interleaved veto —
alternating verdicts emit alternating hunks in order) inline in
`partial.rs` `mod tests`; overlap-freedom property (emitted ranges
pairwise disjoint) over the fixture corpus.

**Non-goals / risks.** No splitting inside a line pair and no
joint header+body template (still future work, now honestly
scoped as such instead of “permanent”). Risk: hunk-count growth
hurting token targets — measured per bench; a split that does
not beat its ordinary bytes under T375 stays ordinary.

### WI-12 (P2, M): Macro interiors at the depth the engine already proves [carries audit-WI-14; R4-WI-14]

**Problem.** Token-tree *argument* holes and rule-body interiors
raise multi-site, but a literal edit inside a `macro_rules!` body
(`inner!(1)` → `inner!(2)`, re-probed) and single-site token-tree
holes still re-diff ordinary — the macro shapes agents generate
most (fixture `48` family, `09`/`86`/`97` singles).

**Decision — spend WI-7's guard inside macros.** (1) The
`macro_rule_body` detector gains the `PrecisionUnique` guard
(WI-7): a single rule-body pair whose holed template matches
exactly the diff sites under the existing one-level token-tree
matcher fires at N=1. (2) Token-tree argument holes
(`old_m!(E, 1)` family) gain the same guard. Depth stays at one
argument level and rule-body interiors stay non-recursive
(round-3 rejection stands); anything deeper or ambiguous stays
ordinary with the depth veto pinned. Emission reuses the holed
spellings verbatim — no new syntax (D6).

**Changes.** SPEC D.4 (macro singles), 21 (depth statement).
Code: guard wiring in
[macro_rule_body.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macro_rule_body.rs)
and [macros.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macros.rs);
engine token-tree matcher unchanged.

**Acceptance.** The rule-body probe compresses and cycles;
fixture `48`'s single-site siblings flip as named in the landing
change; depth-2 nested probe stays ordinary (pin); benches 17/18
hold their (WI-5-re-pinned) bytes.

**Tests.** Macro single matrix (unique / twin veto / depth-2
veto) inline in the owning detectors' `mod tests`; `S″ == S′`
pins per flipped fixture.

**Non-goals / risks.** No macro expansion, no `macro_rules!`
*pattern*-side holes. Risk: token-tree totality on large macro
bodies — scale gate (WI-20) covers the worst bench.

### WI-13 (P2, L): `# symbol` beyond call shapes [carries audit-WI-15; R4-WI-15]

**Problem.** C14 confines `# symbol` to call shapes, so the
directive's promise — “this name is that item in this crate” —
cannot be stated for the `let`/type/general positions where API
renames actually bite, and method idents are unstateable. The
rejection's `write_instead` still offers only “use a call-shape
hunk” (re-probed), which for a `let` rename is no alternative at all.

**Decision — same proof, more positions.** The crate graph
([crate_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/crate_graph.rs))
already resolves `crate::`/`self::`/`super::` seeds; extend the
C13 seed-verification gate to general-expression, `let`, and type
hunks whose symbol occupies a *path-head* position (the positions
the general matcher already compares as paths), and to method-call
*receiver-path* symbols (the receiver's head path — the method
ident itself stays a bare ident and the C14 method-ident rejection
stands, now with a `write_instead` naming the receiver form).
Matching stays exact-path-text once the seed verifies (SPEC 19.3
discipline); only the gate widens. `# fallback=syntax` semantics
unchanged. The symbol report key (`resolved`/`syntax`) covers the
new shapes identically, and WI-1 completeness applies: a symbol
hunk that verifies `syntax` (unresolved) with same-text namesakes
in the crate reports its skips as blockers, never silent `verified`.

**Changes.** SPEC 19.3 (position list), 18.6 C13/C14 rows. Code:
gate widening in `pattern_classify.rs` + `plan_symbol.rs`; matcher
path-head checks reuse the existing path comparisons (no new
matcher).

**Acceptance.** Probes: `let x = Old::make()`, `Vec<Old>` type,
and general-position paths verify `resolved` against a vendored
namesake that text-matching would have hit (veto pin from round 3
still passes); method-ident spelling fails with the new
`write_instead` naming the receiver form.

**Tests.** Symbol position matrix (fires / vendored veto /
fallback-syntax parity with round-3 pins) inline in the owning
modules; C14 narrowed-row verbatim pins updated in the same change.

**Non-goals / risks.** No trait solving, no glob-path resolution
beyond the existing one-hop rule. Risk: path-head detection
disagreeing with the general matcher on exotic paths — the gate
only *admits*; the matcher still decides matches.

### WI-14 (P2, L): Anchors grow up — chains, captures, and generic bounds [carries F10; R4-WI-16(1)]

**Problem.** The landed anchors prove receivers from params, annotations,
literals, and `self`. Everyday Rust also proves them from method
chains (`a.clone().old()` — re-probed `off-target`), closure
captures, and generic bounds (`fn f<T: Tr>(t: T)` for assoc/member
claims); today all of these fall to evidence, and under pre-WI-1
verdicts the run still said `verified` over the leftovers.

**Decision — three bounded anchor classes, in order.** The anchor
order gains, after the existing classes: (1) chain-base propagation
(a method chain claims iff its *base* receiver is anchored and every
intermediate method is name-stable under the rename — i.e. not the
renamed ident; `clone()`-style hops therefore prove, hops *through*
the renamed method do not); (2) closure-capture (a use inside a
closure in the same block claims iff the captured binding is
anchored at the closure's definition, shadowing discipline per
WI-3); (3) generic-bound (`T: Tr` / `T = S` bindings in the
enclosing fn/impl prove `T::Old` and `t.old()` sites for the
corresponding domain). Each class is syntactic, ordered after the
existing classes, and individually vetoable in tests. WI-6's
inherent-method domain and WI-3's binder scopes consume the same
classes through the shared `AnchorIndex` — no per-domain copies.

**Changes.** SPEC 14.8 (anchor order). Code:
[use_graph_anchor.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_anchor.rs)
chain/capture/bound classes (sibling production module if the file
would cross 1600 with tests inline, D5).

**Acceptance.** Chain/capture/bound probes rewrite (the F10
`clone()` probe is the headline); a chain through the renamed
method itself stays evidence (pin); a capture shadowed inside the
closure stays evidence (pin); every existing anchored-follow
fixture keeps its verdicts.

**Tests.** Anchor matrix per new class (claim / shadow / veto)
inline in `use_graph_anchor.rs` `mod tests`; cross-domain pins
(member / assoc / method / binder share one verdict per corpus).

**Non-goals / risks.** No return-type inference through arbitrary
methods — chains propagate the *base* anchor only. Risk: capture
claims across `move` closures with shadowing — shadowing ends
claims (WI-3 discipline), pinned.

### WI-15 (P1, M): Evidence you can act on without re-reading files [carries F9; completes R4-WI-16(2)/WI-18, D12]

**Problem.** F9: follow-only entries carry spans but no origin;
`diffrs` JSON provenance (`hunks`) is empty for ordinary sections;
orphan budgets sample (8 entries, no total) and the expensive flag
does not lift them. An agent correlating “which detector claimed
this, at which sites, with what left behind” still re-reads files —
and after WI-1 it must, because `partial` without blockers is a
dead end.

**Decision — close the holes additively, count everything.** (1)
Every `applyrs` rule entry gains `origin: "definition" | "follow"
| "ordinary"` (follow-only entries are the `follow` case; consumers
stop inferring provenance from `semantics`). (2) Orphans and
follow-skips gain `orphans_total` / `follow_skipped_total` (full
engine counts, always present when the capped arrays are) and
`--bindings full` lifts the 8-entry cap and the per-entry text cut
for orphans/skips exactly as it already lifts binding values (one
expensive-evidence flag; default reports keep today's byte pins,
recomputed in WI-20). (3) `diffrs --format json` populates `hunks`
for *every* emitted hunk (semantic, ordinary-passthrough, rename)
with `{rule|null, detector|"ordinary", file, spans}`; today's
semantic-only population is the bug. (4) WI-1's text blockers and
these JSON totals share one engine counting code path (WI-1 counts
engine-side before truncation; this item exposes the same counts),
so the two surfaces can never disagree.

**Changes.** SPEC D.2 (both reports; keys additive, formalised at
v5 in WI-20). Code: report assembly in
[applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
and [diffrs main](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs);
provenance threading from
[rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs)
(`HunkProvenance` already exists — emit it for all hunks).

**Acceptance.** The 12-orphan probe reports `orphans_total: 12`,
8 entries by default, all 12 under `--bindings full`; two-file
follow JSON distinguishes definition vs follow entries without
reading `span_edits`; the F4 ordinary corpus's `diffrs` JSON lists
its hunks with `detector: "ordinary"`; budget pins recomputed for
the additive keys, never loosened silently.

**Tests.** Provenance pins per hunk class and evidence-lift pins
inline in the owning `main.rs` `mod tests` where the code lives,
plus e2e pins in `report_budget_e2e.rs` (both binaries); single-
JSON-object pin for `diffrs` (guards the stdout contract).

**Non-goals / risks.** No pagination protocol. Risk: additive keys
tempting consumers to depend on detector *names* — SPEC marks
detector names diagnostic, not contractual (rule names are).

### WI-16 (P1, M): Raise from patch text, verify in one command [carries F11; R4-WI-17]

**Problem.** F11: raising requires git state, and verifying a raise
requires hand-piping two binaries through a check that cannot see
over-match. The agent loop for the second direction is strictly
worse than the first (`gitrs apply --check`).

**Decision — `diffrs` learns the two missing verbs.** (1) Input:
`diffrs --patch <FILE|->` (also via `gitrs diff --patch`) raises
from native patch *text*: sections parse with the same parser
`applyrs` uses (SPEC 6), pre-images come from `--repo` at
`--base <REV>` (default: worktree files as checked out — the
caller guarantees the patch applies there; a patch that does not
apply to the stated base fails as `GIT_HUNK_CONFLICT`, never
silently downgrades totality). (2) Verification: `gitrs diff
--check` (forwarded to `diffrs`) runs the T361 loop internally —
materialise pre-tree in a temp dir, real-apply the raised patch
with the sibling `applyrs` engine path, byte-compare against the
post-tree implied by the input (git state or `--patch` applied
natively) — and reports `{version, checked: {raising:
"verified"|"partial", tree: "identical"|"diverged"}, …}` in JSON;
text mode prints the patch only when identical. A diverged check
exits 2 with the first differing path in `error.reason` and the
raised patch still printed (review aid, marked diverged in JSON).
Flag spellings mirror `applyrs` (`--patch`, `--check`) so the
agent learns one vocabulary (D6); `gitrs` forwards both untouched.

**Changes.** SPEC D.1/D.4 (input + check contracts), CLI help.
Code: source adapter in
[rsmpl_diff_source](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_source/src/lib.rs)
(patch-text sections → the existing section model); check loop
in [diffrs main](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs)
reusing `rsmpl_git::apply` primitives — no new apply logic;
`gitrs` dispatcher needs no flag knowledge (passthrough, pinned).

**Acceptance.** A native patch saved from `git diff` raises
identically via git state and via `--patch`; `gitrs diff --check`
on the full fixture corpus reports identical trees; a hand-broken
raise (sibling site the patch would over-match) reports
`diverged` naming the file — the case `--check`-piping missed.

**Tests.** Input parity matrix (staged / rev / patch-text) inline
in the owning modules plus e2e in `bin/diffrs/tests/`; check-loop
pins (identical / diverged / unappliable base); dispatcher
passthrough pins in `gitrs`.

**Non-goals / risks.** No PR/URL fetching — patch text only.
Risk: temp-tree cost on large repos — bounded to changed paths
(the source model already enumerates them); scale gate (WI-20)
covers.

### WI-17 (P2, M): Everyday completeness batch — the next uniform diffs [carries R4-WI-19(b–e)]

**Problem.** Beyond F4/F6, dogfood diffs still strand uniform edits
as ordinary. This item is a *batch with a fixed menu*, each entry
landing only with its pre-image-totality proof and a fixture:
(a) `?` / `.await` chain deltas as general-template anchors
(operator-column hetero currently vetoes — classify `?` and
`.await` as anchor positions, not holes); (b) single-site
derive/attr deltas under `PrecisionUnique` (today: multi-site only
via the `N`-hole sweep); (c) use-group member addition/removal
(`use a::{x}` → `use a::{x, y}` as a use-shape template, not a text
hunk); (d) `mod` file-content moves where the file rename already
fires but its inner items also moved (compose `mod_rename` with
WI-7-verified moves instead of choosing one).

**Decision.** Each menu entry is an independent slice with its own
detector-or-matcher gate, fixture pair (`*-fires` /
`*-stays-ordinary`), and bench-or-history evidence; an entry that
cannot prove totality is cut from the batch and recorded in the
landing change — the batch never ships a guessed shape to meet
its count. Order inside the batch is (a)→(d) (trust before
tokens). Entry (a) touches the general matcher only at anchor
classification; (b) reuses WI-7's guard verbatim; (c) reuses the
use-pattern compiler; (d) reuses WI-7's move probe — no slice
invents a second proof discipline (D4).

**Changes.** SPEC rows per slice (18.6/D.4 as applicable). Code:
slices land in the owning modules named per slice; no slice grows
a D5-watch file (new sibling production modules where needed,
tests inline).

**Acceptance.** Each landed slice's fixture pair passes with
real-apply byte-compare; the batch's net effect on
`tools/history_sweep.py` is reported (compressing set grows or
the cut entries are named); `KNOWN_DIVERGENT` flips are listed
per slice.

**Tests.** Per-slice pairs + veto pins inline in each owner;
sweep re-run.

**Non-goals / risks.** Anything needing type inference, macro
expansion, or formatting normalisation stays ordinary. Risk:
batch sprawl — capped at the four menu entries; a fifth waits
for the next round.

### WI-18 (P1, M): Cross-file intents raise — assoc, trait-method, member [carries F13]

**Problem.** F13: authored follow is workspace-wide, but raising
is section-bound: assoc intents whose definitions and uses span
files stay ordinary (SPEC D.4 admits it), and trait-method
grouping joins only within a section (WI-22c's method-delta join
is per-intent, single-section). The raiser therefore cannot state
the exact patches the lowerer already verifies best.

**Decision — one workspace intent probe for the three grouped
detectors.** `assoc_rename`, `trait_method_rename`, and
`member_rename` gain a workspace stage mirroring `item_intent`'s
existing cross-file probe: when per-file sections carry the same
ident delta (same kind, same old→new, suffix/token discipline
unchanged), the detector may join them under one shared rule
with `# follow=uses` on the document-first definition hunk iff
the scratch-apply probe (real apply of the joined patch to the
pre-tree, byte-compare per file) reproduces every post-tree.
Any file whose delta does not join (different suffix, hetero
kind, unproven tail) stays in its own section with today's
verdict — joining is all-or-named, never partial-and-silent.
Emission keeps per-block hunks and verbatim headers; only the
rule sharing and the follow flag change. Single-file behaviour
is byte-identical.

**Changes.** SPEC D.4 (workspace stage for the three detectors).
Code: shared join helper in
[rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs)
facade (the probe already lives there for `item_intent`); detector
wiring in `assoc.rs`, `trait_methods.rs`, `member.rs` under
`crates/rsmpl_diff_infer/src/semantic/`.

**Acceptance.** A two-file assoc rename (trait in `a.rs`, impl +
uses in `b.rs`) compresses to the follow form and cycles; the
WI-22c cross-file trait-method corpus joins on the method delta
alone (trait-name mismatch still declines, pinned); a mismatched-
suffix third file stays ordinary and is named in provenance
(WI-15 hunks).

**Tests.** Join matrix per detector (joins / suffix veto / hetero
veto / probe-failure fallback) inline in each detector; cycle
pins for the new cross-file fixtures; history sweep reported.

**Non-goals / risks.** No cross-crate intents (workspace = the
`--repo` tree, as authored follow already defines it). Risk:
probe cost over many files — the probe parses each changed file
once (existing discipline); scale gate (WI-20) bounds it.

### WI-19 (P1, M): Inline tests restored, production splits finished [carries F12; enacts D5]

**Problem.** F12: the last round's size discipline exiled 2683
lines of unit tests into five `#[path]` sibling files, against
the project convention (and this plan's standing instruction)
that unit tests live inline in `mod tests` at the bottom of the
owning file — an agent reading a module no longer finds its tests
where Rust convention puts them. At the same time the production
watch files never actually shrank below the danger zone, so the
next features still cannot land without splits.

**Decision — fold tests home, split code, gate both.** (1) The
five sibling test files (`plan_tests.rs`,
`postcheck_move_tests.rs`, `postcheck_move_sweep_tests.rs`,
`use_graph_tests.rs`, `use_graph_follow_tests.rs`) are folded back
into `mod tests` at the bottom of their owning files verbatim
(no assertion changes; the move is reviewable as a pure
relocation diff). (2) A workspace gate test (extending the
existing 2000-line gate) fails on any new `*_tests.rs` sibling
under `crates/*/src/` or `bin/*/src/` — e2e/integration files
under `tests/` directories are explicitly exempt and stay put.
(3) The two largest production watch files the round touches
(`matcher_fn.rs`, `postcheck.rs`, or `semantic/mod.rs` per
landing state) are split along the seams this plan already
created, leaving each under 1600 lines *including* its inline
tests. (4) AGENTS.md/SKILL.md state the rule in one row each
(“unit tests inline; split production modules, not tests”) so
the next split cannot re-exile them silently.

**Changes.** SPEC 31 (implementation note, one paragraph);
AGENTS.md, `skills/gitrs/SKILL.md` (one row each, within budgets:
180/150 lines today — move detail to SPEC, never merge rows
unreadably). Code: relocations + splits only; gate in the
existing fixtures gate test.

**Acceptance.** `find crates bin -path '*/src/*_tests.rs'` returns
nothing; workspace tests pass with identical test count
(± split-module unit tests, listed); no `.rs` file over 2000
lines and touched watch files under 1600; the gate fails when a
sibling test file is reintroduced (self-test).

**Tests.** The gate itself; relocation diff reviewed as
move-only (test list before/after byte-compared via
`cargo test -- --list`).

**Non-goals / risks.** No test rewrites, no e2e moves. Risk:
owner files temporarily exceed 2000 lines mid-fold — fold and
split land in one change per owner so no intermediate state
violates the gate.

### WI-20 (P1, L): One contract, green gates — schema v5 [carries F-version; bumps D7]

**Problem.** `REPORT_VERSION` is still `4` while WI-1 changes the
meaning of `verified`, WI-2 adds an error code, and WI-15 adds
evidence keys; SPEC D.5's exclusion prose no longer matches the
67-entry harness table family-for-family; scale benches 43/44 are
review artifacts rather than assertions. Consumers cannot tell
which contract they are reading.

**Decision — one hygiene landing after the evidence settles.**
(1) Bump `REPORT_VERSION` to 5 in
[lib.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/lib.rs:36)
with exactly the meaning changes WI-1 (completeness verdict),
WI-2 (`RSMPL_BUILD_CONFLICT`), WI-10 (`span_edits` pair form) and
WI-15 (`origin`, `*_total`) carry; remove nothing else; tri-way
parity test (applyrs/diffrs/gitrs error + success envelopes) pins
v5. (2) SPEC D.5 exclusion prose is regenerated from the harness
table (each prose family names its entries; the doc-lint test
fails when table and prose drift, extending the counts lint that
already pins 121/116/32/42). (3) Every RSmPL fence in SPEC.md,
AGENTS.md, SKILL.md is linted against the final grammar (the
fence lint gains the WI-6 inherent-follow and WI-9 fn-follow
spellings). (4) Scale gates (benches 43/44 + the WI-7 whole-file
probe worst case) become CI assertions with recorded budgets
instead of review artifacts. (5) Report-budget pins are recomputed
once for v5 additive keys and the WI-5 anchored bytes.

**Changes.** SPEC D.2/D.5 (v5 + prose), AGENTS.md, SKILL.md.
Code: version constant, parity + lint tests in
`bin/applyrs/tests/fixtures.rs` / `spec_examples.rs` and
`bin/diffrs/tests/`; no new semantics in this item — anything
semantic found here goes back to its owning WI.

**Acceptance.** `REPORT_VERSION == 5` everywhere with parity pins;
`cargo test --workspace` fully green including the D.5 prose
lint; doc-lint fails when a fixture or divergent entry is added
without updating prose (self-test); v4 archives in `.history/`
still parse and apply (long-form acceptance, D6).

**Tests.** The gates themselves; v5 envelope pins across all
three binaries; split-diff review (no semantic change).

**Non-goals / risks.** No new semantics. Risk: v5 churn for
consumers — accepted per D7 (prototype, breaking changes allowed,
one bump only, headline change documented in D.2).

## Sequencing and gates

| Item | Depends on | Size | Gate before next item |
|---|---|---|---|
| WI-1 verdict completeness | — | M | F1 probes report `partial` + blockers; clean twins still `verified` |
| WI-2 build repair | WI-1 orphans (fallback) | S | Build failure carries new code + repair in both modes |
| WI-3 binder totality | WI-1 (one downgrade mechanism) | L | F3 completes or confesses; WI-2/2c/2d verdicts preserved or listed |
| WI-6 inherent follow | WI-1, WI-3 scopes | M | Inherent def + anchored calls move; other-type namesake is evidence; C17 pins updated |
| WI-4 frame raising | — (parallel-safe with WI-1–WI-3) | L | F4 taxonomy compresses byte-identical; frame-delta veto pinned |
| WI-5 anchored templates | WI-4 site sets | M | F5 emits anchored form; site sets unchanged corpus-wide |
| WI-7 precision singles | WI-5 spellings | L | Named `t375` entries flip; sibling veto pins hold |
| WI-8 site-set canonical | WI-7 spellings | M | `01`/`90`/`51` leave the divergent table |
| WI-9 fn follow raising | WI-6 domain, WI-7 guard | M | `46`/`89` flip or divergences named; veto pin holds |
| WI-14 anchor growth | WI-6, WI-3 domains | L | Chain/capture/bound matrix green; F10 probe rewrites |
| WI-15 evidence closure | WI-1 counts | M | Totals + lifts pinned; ordinary hunks listed; budgets recomputed |
| WI-16 patch-input raising | WI-15 provenance (soft) | M | Patch-text parity + diverged-check pin |
| WI-18 cross-file intents | WI-9 machinery, WI-15 | M | Two-file assoc joins or vetoes named; single-file bytes unchanged |
| WI-10 joint impl intent | WI-18 probe, WI-9 | M | `14` reaches 9+0 or survivor named |
| WI-11 line partition | WI-4 (frames excluded from splits) | L | `12` compresses its let run; whole-section winners unchanged |
| WI-12 macro singles | WI-7 guard | M | Rule-body probe cycles; depth-2 veto pinned |
| WI-13 symbol widening | WI-14 anchors (soft) | L | Vendored veto holds on new positions |
| WI-17 everyday batch | WI-3, WI-7 | M | Per-slice fixtures; cut entries named |
| WI-19 inline tests + splits | — (any time; before WI-20) | M | No sibling test files; watch files under 1600 incl. tests |
| WI-20 one contract v5 | WI-1, WI-2, WI-10, WI-15 | L | Full workspace green; v5 parity; D.5 prose linted |

Global validation per item: `cargo test --workspace`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`./example-benches/run.sh` (README tables regenerated only when
bytes change), and `python3 tools/history_sweep.py` (0 failures;
compressing set reported, never asserted to a fixed number beyond
“does not shrink without a named reason”). No item lands with a
red gate, a new `.rs` file over 2000 lines, an existing file
pushed over 2000 lines, or a new sibling `*_tests.rs` file (D5).
Every raising item proves itself by real apply + byte-compare
(T361), never by `--check` alone; every lowering trust item
(WI-1, WI-2, WI-3, WI-6) carries a durable apply-then-`cargo check`
fixture, not only JSON pins.

## Risks / open questions

Per-item risks are stated with each item. Open questions: none —
scope, canonical forms, detector order, guards (64-line range
rule, precision-uniqueness probe, one-level macro descent),
anchor proof order, verdict derivation (WI-1), schema versioning
(unified v5 in WI-20), and acceptance numbers are decided here.
If a threshold proves wrong against the probe corpora, the fix
is a new pin plus a SPEC note, not a silent constant edit. Three
deliberate non-goals bear repeating because they bound several
items: no type inference or trait solving anywhere (anchors are
syntactic — WI-6/WI-13/WI-14 prove from text, never from `rustc`),
no macro expansion (SPEC 21.5 stands; WI-12 deepens
rendering-level matching by guard only), and no
minus-must-be-zero postcondition (widening rewrites legitimately
re-match their own output — verification stays at the plus/move
spans, as round 4 established). A fourth joins them: no default
`cargo check` (WI-1/WI-2 make the *evidence* honest and the
compiler repairable instead of making it mandatory).

## Considered and rejected

- **Always-on build verification (default `--verify build`).**
  Rejected: couples every `--check` to a toolchain and a
  `Cargo.toml`, breaks the text-cost model, and teaches agents to
  route around the tool. WI-1 buys the honest signal (domain
  completeness → `partial`) at text cost; WI-2 makes the compiler
  an explicit, repairable escalation.
- **Full type-directed resolution (rust-analyzer integration).**
  Rejected as in every round: un-auditable in a prototype and
  unnecessary for the provable majority — WI-14's syntactic
  anchors cover chains/captures/bounds without it.
- **Dropping T375 entirely (always emit semantic).** Rejected:
  sweep-class templates whose minus matches wider than the diff
  are exactly where over-match risk lives; WI-7 narrows the guard
  to uniqueness-proved singles instead of abolishing it.
- **Emitting frames in templates (`fn f0() { foo(E) }` as the
  pattern) to fix F4 from the authoring side.** Rejected: the
  engine already matches the subterm; frames are a *raising*
  blind spot, not a language gap. Authoring new frame syntax
  would tax every agent to fix one detector.
- **Renaming `verified` instead of tightening it (e.g. a new
  `complete` word alongside).** Rejected: agents already branch
  on `verified`; a second word splits the contract and lets the
  old lie persist under a familiar name. WI-1 tightens the one
  word, WI-20 versions the change honestly.
- **A separate `verifyrs` binary.** Rejected: trust rides the
  existing `checked` envelopes (WI-1/WI-2) and the one new check
  verb lives on `gitrs diff --check` (WI-16), keeping `gitrs` a
  two-subcommand dispatcher.
- **Recursive token-tree macro descent.** Rejected in WI-12 as
  before: one argument level is provable with the existing
  token-tree matcher; deeper descent invents a second macro
  engine inside the first.
- **CRLF/binary semantic compression.** Stays declared-lossy
  (D1): detectors strip `\r` by construction and patches are
  UTF-8; passthrough tree-losslessness is the correct contract.
- **Keeping sibling `*_tests.rs` files and amending the
  convention instead.** Rejected: the convention is the
  instruction (tests inline at the bottom of the owning file);
  the exile was a line-count expedient whose cost — agents
  reading modules without their tests in place — recurs on
  every future read, while WI-19's production splits solve the
  size problem at its root.
