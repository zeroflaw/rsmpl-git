# IMPROVE — RSmPL as an intent-complete contract, round 8: close the N=1 schism, prove intents over bytes, finish the carried contract

## Goal

Make RSmPL the contract an agent can trust in both directions without
reconstructing and inspecting every edit by hand:

- **Lowering** (semantic → native, `applyrs` via `gitrs apply`) must never
  report `verified` while its own evidence lists code the rename left
  behind, and every machine `repair` it offers must be safe to paste
  verbatim. That half is now largely true and must stay true while the
  raising half widens.
- **Raising** (native → semantic, `diffrs` via `gitrs diff`) must compress
  the Rust agents actually write as *intents*, not fragments: a rename
  that also changes a signature, a body, an attribute, a field set, or a
  file is one intent; a call on a one-line `fn`, behind `async`, through
  a `Box`, or inside a macro body is the same intent as the call on its
  own multi-line `fn`. Half-semantic output (ordinary definition +
  semantic calls) is a failure mode, not a partial success.
- **Cycles must close under stated equalities**: semantic → native →
  semantic and native → semantic → native, with every remaining
  divergence named, owned, and pinned — never silent. Two spellings
  that prove the same site set compare equal; only site-set
  differences may diverge.

`gitrs` stays a thin dispatcher (`apply` → `applyrs`, `diff` →
`diffrs`); all semantics live in the crates. Backwards compatibility
is not required: breaking changes to [SPEC.md](/home/ai/git/tools/rsmpl-git/SPEC.md),
the JSON envelopes, and emitted patch text are allowed where this plan
says so. Token efficiency matters, but never at the cost of a guess or
of auditability: the cheapest *trustworthy and readable* form wins.
Syntax stays familiar — Git hunk shape, SmPL-style `#` directives,
Rust-shaped patterns — because agents and humans already know those
spellings from training data and daily use. Diet spelling (`# sweep`,
inferred metavars, terse renames) remains canonical. Unit tests stay
inline (`mod tests` at the bottom of the owning file); no new sibling
`*_tests.rs` files. Work stops at 25 items.

This review supersedes the previous IMPROVE.md (round 7) in full.
Round 7's core landed: 17 of its 25 items landed whole or as named
slices (WI-1–WI-5, WI-11–WI-19, WI-21 below), and its two P0
corrections (hint layering, binding-aware `as` paste) are closed and
must not regress. What remains is concentrated, not diffuse: the N=1
canonical schism (WI-1), three unlanded precision families (WI-2,
WI-3), one unlanded language widening (WI-4), two header-shape blind
spots found by this review (WI-5, WI-6), a byte guard that still
outvotes proved intents at small site counts (WI-7–WI-9), and the
four carried contract items (WI-19, WI-22–WI-24 in new numbering).
Every finding below names the probe that proves it, so the landing
change can re-run that probe as a fixture.

## Success criteria

1. **Cycles close at N=1 (WI-1).** An authored templated single and
   the literal or holed single the raiser proves for the same one
   site compare equal under tree-canonical form. The 13
   `verbatim-single`/`holed-single` entries leave
   `KNOWN_DIVERGENT`; no entry ever leaves it by weakening the tree
   legs.
2. **No ordinary singles without a named missing proof (WI-2, WI-3,
   WI-16).** Every `t375` entry in `KNOWN_DIVERGENT` (20 at HEAD)
   either flips to `CYCLE_OK` or its reason is rewritten to name the
   specific proof that fails (totality, anti-unification ambiguity,
   or by-design exclusion). The tag `t375` alone never closes an
   item.
3. **Headers classify by delta, not by spelling luck (WI-5, WI-6,
   WI-8).** A pure rename whose header carries stable `async` /
   `unsafe` / `const` / `extern` qualifiers, and a rename whose
   definition sits on one line, each raise to the same follow intent
   as the plain multi-line twin. A rename plus a two-slot signature
   delta raises as one joint or stays wholly ordinary — never as an
   ordinary definition beside semantic calls.
4. **Proved intents outrank byte savings (WI-7, WI-9).** When the
   scratch-apply probe proves the exact site set, a joint or method
   intent fires under a bounded, per-site precision allowance even
   when the semantic form is larger than the native form. Bytes
   remain the guard only where no proof exists.
5. **Follow-through follows Rust, not the first receiver (WI-10,
   WI-11).** `Box`/`Rc`/`Arc`/reference receivers, trait-method
   return chains (`c.clone().get()`), and two-hop constructor chains
   prove or skip with their named reasons; no proven site strands as
   `off-target` because its wrapper was unmodelled.
6. **Joints ignore file boundaries (WI-12).** The joint `impl` and
   the rename+deletion joints fire across files under the same
   workspace probe as the assoc/trait-method/member intents, or stay
   wholly ordinary. Single-file purity is never the reason a joint
   declines.
7. **Bodies and moves complete (WI-13, WI-14).** A rename plus a
   `let` or multi-expression body delta, and a move plus an
   attribute, trait, or container delta, each raise to the stated
   composed form under one joint probe, or stay wholly ordinary.
8. **Evidence answers without re-reading files (WI-17).** Every
   ordinary section names every detector that tried, why it
   declined, and whether it never tried, in text and JSON alike.
   `partial` continues to name its blockers with full counts.
9. **Robustness is measured, not reviewed (WI-19, WI-20, WI-21).**
   A seeded property harness runs both cycle directions in CI; the
   full-history sweep runs as a gate on a pinned rev-range with a
   compression floor; diffrs raising has scale benches beside the
   applyrs ones.
10. **Contract hygiene (WI-22, WI-23, WI-24, WI-25).** One report
    version across `applyrs`, `diffrs`, and `gitrs` errors (v5,
    bumped once); unit tests inline again with a gate against
    sibling test files; SPEC D.5 prose generated from the harness
    table; the agent contract (error table + directive table) has
    one source and is machine-linted in all three documents; every
    RSmPL shape in SPEC.md has an AGENTS.md row and a fixture.
11. **Gates.** `cargo test --workspace`, `cargo clippy
    --all-targets -- -D warnings`, `cargo fmt --check`, and
    `./example-benches/run.sh` stay green per item; no `.rs` file
    exceeds 2000 lines (split production code before growing; tests
    stay inline); bench byte targets in each item are met or the
    item does not land.

## Review findings (evidence)

Verified this run at HEAD `d2b4a55` (79 commits since the round-7
review base `18275b1`): **3964 tests listed, `cargo test
--workspace` fully green; clippy and fmt clean;
`./example-benches/run.sh` green (32 applyrs benches, 48 diffrs
benches, scale budgets met); `python3 tools/history_sweep.py
--limit 20` converts 20/20 with 0 failures (6 compressing).**
Probes were run with the debug binaries against throwaway `/tmp`
git repos (authored `applyrs --format json --check`, real applies
followed by tree byte-compare, and pending-change `diffrs`
conversions); each finding names its probe so the landing change
can re-run it as a fixture. The architecture is sound and stays:
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
[gitrs](/home/ai/git/tools/rsmpl-git/bin/gitrs/src/main.rs) only
re-execs siblings. Corpus on disk today: 121 applyrs fixtures, 144
diffrs fixtures, 32 applyrs benches, 48 diffrs benches; `CYCLE_OK`
holds 72 entries (61 at round 7), `KNOWN_DIVERGENT` holds 41 (52 at
round 7).

### Carry-forward audit of round 7

| Old item | Verdict | Evidence / residue |
|---|---|---|
| WI-1 hint layering | Landed | `8f0f0a5` + `b521660`; suspect named, `expected_header: null` + `fix_line` |
| WI-2 binding-aware `as` | Landed | `a0e9f89`; shared `syn` occurrence classifier, loud vetoes |
| WI-3 joint `impl` | Landed narrowly | `db309d0` + `4d5031d`, fixture `128`, bench `47`; cross-file and `impl<…>`-generic joints stay ordinary (F7) |
| WI-4 partition | Landed narrowly | `08dda13`, fixture `129`, bench `48`; interleaved order and fixture `12` header+`let` stay ordinary (F11) |
| WI-5 vis/qual joints | Landed narrowly | `a047e16` + R1–R3, fixtures `130`/`132`/`133`/`134`; two-slot deltas (vis+return) fragment (F5) |
| WI-6 precision I | Partial | Slices 1–2 only (`a4ef107`, `6d0fb52`): `93`/`50`/`62`/`54`/`80` flipped; templated singles re-infer as literal/holed singles and diverge canonically (F1) |
| WI-7 precision II | Partial | Slices 1–2 only (`de5cc9c`, `d2b4a55`): `106`/`72`/`73`/`84`/`98`/`107` flipped; holed pat/len/infer singles remain (F1) |
| WI-8 precision III | Not landed | No commits; `18`/`58`/`69`/`76`/`113`/`117`/`13`-ret still divergent (F2) |
| WI-9 precision IV | Not landed | No commits; macro/use/symbol/trait singles still ordinary (F2, F8) |
| WI-10 `# symbol` widening | Not landed | Re-probed: `let` hunk still fails C14 verbatim (F9) |
| WI-11 anchors | Landed narrowly | `5d4d67e`; one hop verified, deeper/Deref/trait-return classes skip (F6) |
| WI-12 cross-file intents | Landed narrowly | `93ba120`, fixtures `100`/`101`; joint and deletion joints still single-file (F7) |
| WI-13 binder completeness | Landed | `218510a` + six discovered slices; field/capture completion in SPEC 14.8 |
| WI-14 everyday | Landed | Slices 1–5 + seven discovered slices; use-split is the only named residue (F12) |
| WI-15 evidence | Landed | `618ff58` + four follow-ups; totals, `origin`, declined (subset, F10) |
| WI-16 text raising + `--check` | Landed | `3b2821e` + `077586e`; `--patch`/`--file`/`--check` in `--help` and green |
| WI-17 `--verify` print | Landed | `d2d0ee0`; `--help`, SPEC D.2, and behaviour agree |
| WI-18 repair gate | Landed | `0cb764e` + 12 slices, ceiling 96 → 0 over `StructuredError` sites |
| WI-19 move completion | Landed narrowly | Slices 1–4 + impl residue (`d0727ca`…`2a51b89`); attr/trait/container-delta moves ordinary (F8) |
| WI-20 rename+body | Slice 1 only | `6d3a900`, diffrs fixture `119`; `let`/multi-expression bodies fragment (F4) |
| WI-21 deletion joints | Landed narrowly | Slices 1–5, fixtures `142`/`143`; cross-file deletion joint ordinary (F7) |
| WI-22 inline tests | Not landed | Five siblings still hold 2699 lines (2683 at round 7 — grew) (F13) |
| WI-23 property harness | Not landed | No generator in the workspace (F14) |
| WI-24 v5/D.5/diet | Not landed | `REPORT_VERSION` still `4`; SPEC 9827 lines; benches `45`–`48` at 37/21/31/26% (F12) |
| WI-25 single-source | Not landed | No generator; tables still hand-copied in three documents (F12) |

### Findings

- **F1. The largest divergent class is now a spelling schism, not a
  detector gap.** 13 of 41 `KNOWN_DIVERGENT` entries (`47`, `49`,
  `63`, `64`, `65`, `66`, `67`, `83`, `94`, `95`, `96`, `118`,
  `119`) carry `verbatim-single`/`holed-single` reasons: the native
  single *does* fire semantically and both tree legs close, but the
  authored templated spelling (`E + 1 → E.wrapping_add(1)`, holed
  struct-pat swap, control template) and the inferred literal or
  holed diet single are different canonical texts, so
  `canonicalize_with_tree(S′) != canonicalize_with_tree(S)`. Round 7
  sliced detectors; nobody sliced equality. Until equality is
  site-set-based at N=1, every future templated single re-creates
  this class. Correction is WI-1.
- **F2. Twenty `t375` entries still re-infer ordinary, in three
  families.** Detector-absent singles (`58`/`76` impl-filter, `69`/
  `117` type, `113`/`13` return, `10` impl, `19`/`112` trait, `102`
  use, `104` symbol, `86`/`97`/`09`/`75` macro/token-tree, `82`
  const-block, `22` attr-strip, `06` insertion, `08` qualified
  call, `04` two-singles). Probes this run: a one-line type rewrite,
  a one-site macro literal change, and a `HashMap`→`BTreeMap`
  rename each re-diff as bare ordinary hunks with no semantic part
  at all. These are the small precise edits agents make most; each
  needs its named uniqueness proof or a renamed by-design reason
  (WI-2, WI-3).
- **F3. A pure rename stops raising the moment the header carries
  stable qualifiers.** Multi-line probes: plain, `pub`, generic,
  and `pub(crate)` renames fire as one `# sweep` + `# follow=uses`
  intent; `async fn`, `pub async fn`, `unsafe fn`, `const fn`, and
  `extern "C" fn` renames with token-identical qualifiers fragment
  into an ordinary definition beside a semantic call hunk. The
  authored language already matches these headers slot-wise (SPEC
  14.1); the raiser's header classifier only admits the qualifier
  run when it *changes* (round-7 WI-5). Everyday async Rust pays
  this on every rename. Correction is WI-5.
- **F4. One-line definitions are invisible to the intents.**
  `fn foo(x: i32) -> i32 { x }` beside its renamed call re-diffs as
  ordinary definition + semantic call; the multi-line twin fires as
  one intent. The same split hits one-line method bodies (the
  discovered single-line trait-method misclassification,
  `865901c`, is this blind spot surfacing). One-line getters,
  wrappers, and trait stubs are idiomatic Rust, not an edge case.
  Correction is WI-6.
- **F5. Proved intents still lose to a 16-byte guard at small site
  counts.** `PRECISION_GUARD` is 16 bytes per hunk in
  [definition_evolution.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/definition_evolution.rs).
  Probes: a small inherent-method rename (one definition + one
  call) fragments to ordinary definition + `Receiver.old()` sweep
  while the long-signature twin fires as one follow hunk; a
  rename+arg-add with one call stays wholly ordinary while fixture
  `120` (two calls) fires; a rename+body joint with one call emits
  three separate rules while diffrs fixture `119` (two calls)
  fires as one. The scratch-apply probe already proved these site
  sets; bytes outvote proof exactly where the audit win matters
  most. Correction is WI-7 (mechanism), spent by WI-8/WI-9.
- **F6. Follow anchors still stop at wrappers Rust derefs for
  free.** Authored probes with the canonical `impl S` follow shape:
  `S::make().run()` verifies (one hop); `S::make().mid().run()`
  skips `chain-depth` and reports `partial`; `b.run()` with
  `b: Box<S>` skips `off-target` (`&S` on the next line rewrites);
  `c.clone().get()` skips `chain-unproven` because `Clone` is a
  trait return, not an inherent constructor. The skips are honest —
  and they strand the commonest receiver shapes in Rust. WI-11
  (anchors) landed its stated caps; the caps are the residue
  (WI-10, WI-11 in new numbering).
- **F7. Joints still stop at file boundaries.** Same-file joint
  `impl` fires as two shared-rule follow hunks; the cross-file twin
  (trait in `a.rs`, `struct`+`impl` in `b.rs`) emits a follow-free
  trait rename in `a.rs` beside a wholly ordinary `b.rs`. The
  rename+field-deletion joint fires same-file (fixture `142`); its
  cross-file twin (definition in `a.rs`, last use edited in `b.rs`)
  re-diffs wholly ordinary in both files. The workspace probe that
  powers assoc/trait-method/member intents (round-7 WI-12) was
  never offered to the joint detectors. Correction is WI-12.
- **F8. Moves and macros keep their round-7 residues.** Moving a
  `fn` while adding `#[inline]` re-diffs as delete + insert with no
  move intent; moving a trait (with a method rename) likewise; a
  single literal edit inside a `println!` invocation and inside a
  `macro_rules!` body each re-diff ordinary. Fixture `45` still
  pins the split macro-rules intent (definition single, uses
  ordinary). Corrections are WI-14 (moves) and WI-3/WI-16 (macro
  singles and the macro-rules use side).
- **F9. `# symbol` still stops at call shapes.** Re-probed
  verbatim: a `let x = Old::compute();` hunk with `# symbol` +
  `# fallback=syntax` fails `RSMPL_PATTERN_CONFLICT` (C14) with
  “only supported in call shapes in v0.1”. Vendored-path renames in
  `let`, type, field, and use positions still force concrete-path
  patterns that silently miss re-exported spellings. Correction is
  WI-4 (unchanged from round 7; it did not land).
- **F10. Declined provenance names a subset as if it were the
  whole trial.** A comment-only change under `diffrs --format
  json` lists 12 declined detectors; the chain in
  [chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs)
  attempts roughly 40. An agent cannot distinguish “`call_sweep`
  declined for reason R” from “`call_sweep` never tried this
  hunk”, so it cannot compute its next action from the report —
  the exact reconstruction WI-15 exists to prevent. The text
  surface has the same gap (bare ordinary patch, no provenance at
  all). Correction is WI-17.
- **F11. Partition residue is one shape wide.** Interleaved
  (A B A) sections stay ordinary by design — order is never
  manufactured, and that stays. What remains is fixture `12`
  (header+`let` in one block) and its single-pair kin: no detector
  owns a definition line fused with a statement line, and SPEC D.5
  still calls the fused case a permanent non-goal in the
  `single_block` paragraph. WI-13's scoped body pairs give this a
  stated shape; WI-15 spends it.
- **F12. The contract documents drifted again while the code
  improved.** SPEC D.5's exclusion prose still enumerates `01` and
  `21` although both have been in `CYCLE_OK` since round 5; the
  prose was hand-patched for WI-3/WI-4 (it now narrates fixtures
  `128`/`129`) without becoming generated, so the next flip drifts
  it again. SPEC.md grew 9213 → 9827 lines (512 KB). Joint and
  partition benches remain the weakest in the table (`45` 37%,
  `46` 21%, `47` 31%, `48` 26% saved) — the auditable form is
  shipped, the diet is not, and the table still presents them as
  token savings. Corrections are WI-23 (v5 + generated D.5 +
  diet) and WI-24 (single-source tables).
- **F13. The watch files are still at the gate, tests still
  exiled.** The five `#[path]` sibling test files hold 2699 lines
  (650 + 1076 + 124 + 528 + 321), up from 2683 — the exile grew
  during a round that added no WI-22 work. Eleven production files
  sit above 1600 lines, four above 1900
  ([item_intent.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/item_intent.rs) 1994,
  [fn_joint.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/fn_joint.rs) 1972,
  [methods.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/methods.rs) 1961,
  [matcher_fn.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_fn.rs) 1958).
  Every item in this plan that touches them must split first
  (WI-22, pulled forward per-item by the standing rule).
- **F14. Robustness is still reviewed, not measured — and the
  real-world signal is thin.** No property/fuzz harness exists.
  The history sweep converts this repo's own last 20 commits
  20/20 but compresses only 6 — real diffs (docs, fixtures, mixed
  edits) mostly stay ordinary, and nothing in CI would notice the
  rate falling. The fixture anecdote set (121 + 144 dirs) remains
  the only systematic evidence. Corrections are WI-19 (generator), WI-20
  (history gate with a floor), WI-21 (raising scale).

## Key decisions

- **D1. Losslessness is judged per origin corpus, in writing.**
  Native-origin diffs owe state-losslessness (post-tree
  byte-identical). Semantic-origin patches owe canonical
  losslessness for multi-site shapes and a *defined* fixed-point
  equality (`S″ == S′`) for single-site shapes. Raw native bytes
  never round-trip (0-context lowering, `index`/title stripping) —
  declared, as today. Binary/gitlink/non-UTF-8/CRLF semantic
  compression stays out of scope; passthrough cycles must remain
  tree-lossless.
- **D2. No success field is true by construction, and no proof is
  a substring search.** Every `checked` claim is computed by a
  check that can fail on real input *at the reported span*. Every
  `repair` is computed from the actual failure, at the actual
  line, and never encodes a truncated body as the intended body.
  Round 8 extends the same rule to equality: a canonical-equality
  claim (WI-1) is computed by instantiating both sides at their
  sites, never by comparing pattern strings.
- **D3. Inference stays precision-first, and proof outranks
  bytes.** Every detector keeps pre-image totality (count equals
  pair count, zero opaque hits). Where the scratch-apply probe
  proves the exact site set, the byte guard degrades to a bounded
  precision allowance (WI-7); where no proof exists, strict T375
  (`semantic < ordinary`) still vetoes. A detector that cannot
  prove totality stays ordinary — no guessing.
- **D4. One anti-unification core, thin detectors.** Raising work
  reuses the existing
  [antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
  core and the sub-line extractors instead of growing per-shape
  string surgery. Engine work is structural `syn` matching —
  concrete, fail-loud, no expansion, no guessing.
- **D5. Split production code before growing; never exile
  tests.** New code lands in new modules; no item pushes a file
  over 2000 lines, and any file over 1600 lines touched by an
  item leaves under 1600 or with its new code in a sibling module.
  Unit tests stay inline (`mod tests` at the bottom of the owning
  file); integration/e2e tests under `bin/*/tests/` and
  `crates/*/tests/` are unaffected. Each work item below keeps the
  smallest focused regression test with the project, inline in the
  owning file.
- **D6. Familiar syntax wins over clever syntax.** New surface
  reuses Git/SmPL/Rust spellings (`...`, `#` directives, concrete
  paths, `as` aliases, header pairs that look like the code they
  select). Diet remains canonical; the parser keeps accepting long
  forms so `.history/` archives stay readable. Any new spelling
  must be explainable in one AGENTS.md row. Round 8 adds no new
  directive keywords: widened intents reuse `# follow=uses` and
  `# sweep`; scoped body pairs (WI-13) reuse the WI-20 body-pair
  spelling; the only new CLI surface is none — every item works
  through flags the agent already knows.
- **D7. Breaking changes are taken where they buy trust.** JSON
  schema stays at its current version for additive keys; a key
  whose *meaning* changes bumps the version with the old key
  removed. SPEC text moves with code in the same item. This round
  bumps the unified report to `version: 5` exactly once (WI-23)
  after WI-17 has settled the provenance shapes it carries.
- **D8. Anchors before indirection.** Follow, symbol resolution,
  and member anchoring prove site ownership from text the change
  already contains — fn-signature param types, `let` annotations,
  struct-literal paths, `use` chains, the enclosing
  `impl`/`trait` header, stated return types — before consulting
  any wider graph. A site no anchor proves is evidence
  (`orphans`/`follow_skipped` with a reason code), never a rewrite
  and never a veto on the proven sites. Round 8 adds exactly two
  anchor facts Rust itself guarantees: the closed smart-pointer
  set (`Box`/`Rc`/`Arc`, plus `&`/`&mut`) derefs to its parameter,
  and a trait method's stated `Self` return is a return type
  (WI-10).
- **D9. A rename owns its secondary definitions.** For every
  rename kind, SPEC enumerates the *definition set* (trait decl +
  impl definitions for associated items and trait methods; the
  binding + its scoped uses for locals; the `use` re-export tail
  for cross-file intents; the composed `impl` header for joint
  renames) and the follow/orphan machinery accounts for every
  member: rewritten, skipped with a reason, or the run degrades.
- **D10. Auditability outranks byte-minimality.** When two
  emissions prove the same site set, `diffrs` emits the one whose
  pattern states its frame (holes + anchors), even at a small byte
  cost bounded by WI-7's allowance. Bare-fragment emissions
  survive only where the fragment is the whole matched unit. A
  `- I` cut is never emitted as a *change representation* (cuts
  exist only paired with `# bind=`/`# use=` in moves). Bench
  claims follow the auditable form: a bench that cannot beat
  native bytes after dieting retitles its claim (WI-23).
- **D11. Binder honesty is keyed by binding, not by spelling.**
  Any binder-adjacent work enters through the compiled pattern's
  binder identity, never a list of surface shapes. Body rewrites
  (the `as` splice, scoped body pairs) are keyed by the *bound*
  occurrences of the renamed ident, never by word-bounded text.
- **D12. `partial` must name its blockers.** A non-`verified` run
  states, per blocking rule, the counts that block it (orphans,
  follow-skips, ordinary hunks, unprobed shapes) in text and as
  JSON totals. “Somewhere in these 40 files” is not evidence; the
  agent's next action must be computable from the report alone.
  Round 8 completes the raising half of this rule: an ordinary
  section must name its blockers too (WI-17).
- **D13. Joints compose only under one probe.** A joint intent
  (rename+evolution, rename+addition/deletion, rename+body,
  move+delta, double-delta `impl`) is emitted only when a single
  scratch-apply of the *composed* emission against the pre-image
  reproduces the native plus lines at exactly the claimed sites
  plus every other line byte-identical. If any component delta
  fails the joint probe, the whole class stays ordinary.
- **D14. Frames are raising context, never pattern syntax.**
  Frame unwrapping changes only which subterm the detectors see;
  emitted patterns never carry `fn`/`||`/`match` frame text, and
  authored patterns gain no frame spelling. One-line definitions
  (WI-6) are a header-classification widening in the raiser and
  the item matchers, not a new pattern spelling.
- **D15. Error guidance is keyed by layer, never copied across
  layers.** Parse-time failures teach marker discipline;
  engine-time failures teach content discipline (“paste
  `actual`”); build-time failures teach repair discipline. Each
  hint string has exactly one owning layer and one source string;
  documents quote it, never retype it (WI-24 generates the
  quotations).
- **D16. A splice that rewrites a body must prove the body.**
  Body-rewriting pastes and scoped body pairs prove, per
  occurrence, that the occurrence is the renamed item, and leave
  every other occurrence byte-verbatim or veto. Unclassifiable
  occurrences veto; they never ride a text scan.
- **D17. Equality is site-set equality (new).** At N=1, two
  spellings are canonically equal iff instantiating each against
  the pre-image yields the same single site and the same post
  text at that site. The comparison is performed by the
  canonicalizer with the tree in scope (the WI-1a machinery),
  never by normalizing pattern text in isolation — a normalized
  template that would match a second site the literal does not
  is *not* equal, and the entries stay divergent with that reason
  named.

## Work plan

Order is dependency order: WI-1 → WI-2 → WI-3 → WI-4 → WI-5 →
WI-6 → WI-7 → WI-8 → WI-9 → WI-10 → WI-11 → WI-12 → WI-13 →
WI-14 → WI-15 → WI-16 → WI-17 → WI-18 → WI-19 → WI-20 → WI-21 →
WI-22 → WI-23 → WI-24 → WI-25. Equality first (WI-1) because
every later singles item is judged against it; the header
classifiers (WI-5, WI-6) precede the guard reform (WI-7) that
prices their emissions; anchors (WI-10) precede cross-file
joints (WI-12) whose probes consume them; evidence (WI-17)
precedes the schema bump (WI-23) that freezes its shapes;
WI-24/WI-25 last because they generate documents from the
settled contract. Sizes are relative (S ≤ 1 day, M ≈ 2–3 days,
L ≈ 4–5 days of focused work).

### WI-1 (P0, L): N=1 canonical equality — templated, literal, and holed singles unify [carries F1; enacts D2/D17]

**Problem.** F1: 13 `KNOWN_DIVERGENT` entries (`47`, `49`, `63`,
`64`, `65`, `66`, `67`, `83`, `94`, `95`, `96`, `118`, `119`)
close both tree legs but fail the canonical leg because the
authored templated spelling and the inferred literal/holed single
differ as text. The harness reason already says “canonicalization
territory, not a detector gap” for `94`–`96`; the other ten are
the same territory unsliced. Left alone, every new templated
single the raiser learns (WI-2/WI-3) re-creates the schism.

**Decision — the tree canonicalizer gains a single-site
equivalence, proved by instantiation (D17).** For a hunk pair
where both sides reduce to exactly one edit site under the
fixture pre-image, `canonicalize_with_tree` compares the
*instantiated* site sets: each side is scratch-applied to the
pre-image in memory; the spellings are equal iff both produce
the identical post-tree and each side's pattern, run as a matcher
over the pre-image, hits exactly that one site (the WI-7
matcher-run proof, reused as an equality premise). The emitted
bytes never change — `diffrs` keeps emitting the cheapest
trustworthy single (D10) and authored patches keep their
spelling; only the *comparison* widens. Multi-site spellings are
untouched: a `# sweep` template that would hit a second site the
literal misses is not equal, and its entry stays divergent with
the reason rewritten to “template proves wider than the single
site” instead of the current generic divergence. Entries `52`
(let-verbatim macro sweep vs sub-line call sweep) and `18`
(holed sweep vs bare single) are assessed by the same rule in
this item: `18` flips if the instantiation proof holds; `52`
flips only if both spellings instantiate to the same sites,
else its reason is rewritten to name the two different shapes.

**Changes.** SPEC D.5 (single-site equality paragraph rewritten:
defined equality now includes proved single-site spelling
equivalence; the exclusion enumeration regenerated by WI-23
thereafter). Code: equivalence in
[canonical_tree.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/canonical_tree.rs)
(the tree-aware canonicalizer; the text-only
[canonical.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/canonical.rs)
is unchanged — equality without a tree stays strict), matcher-run
helper shared with WI-2's detectors via
[antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
only if the canonicalizer needs the template form (prefer
calling the engine matcher the canonicalizer already wraps).
If `canonical_tree.rs` (1531 lines) plus the equivalence exceeds
1600, the equivalence lands in a sibling module (D5).

**Acceptance.** The 13 entries move to `CYCLE_OK` slice-by-slice
(verbatim family, then holed family), each move landing in the
same change as the proof arm that earns it; a synthetic
counter-case (authored template with a second latent match site,
inferred literal single) stays divergent with the widened reason
pinned; every multi-site `CYCLE_OK` entry is byte-unchanged;
both tree legs still run before the canonical leg (a tree
failure can never be laundered into an equality).

**Tests.** Inline in `canonical_tree.rs` `mod tests`
(equivalence positive/negative pairs per family: arg-wrap,
struct-pat, len-delta, control, return, infer) plus the
`cycle_e2e` table moves. No new test files.

**Non-goals / risks.** No emission change: making `diffrs` emit
templated singles to match authored text is rejected (it taxes
every native-origin raise for a semantic-origin comparison).
Risk: an over-wide equivalence hides a real over-match — the
exactly-one-hit premise on *both* sides is the guard, and the
negative control is pinned, not argued.

### WI-2 (P1, L): Precision singles II — type, impl-filter, and signature-return [carries old WI-8 / F2 family A]

**Problem.** `58`/`76` (impl-filter rename), `69`/`117`
(type-hole path rewrite), `113` and the return half of `13`
(signature-return evolution) still re-infer ordinary at N=1.
Probes: a one-line `Vec<u8>` → `Box<[u8]>` return change and a
single `-> i32` → `-> i64` header each re-diff as bare ordinary
hunks. The N≥2 detectors exist (type sweep, sig evolution); only
the single-site admission is missing.

**Decision — three admissions under WI-1's matcher-run proof,
one per sub-family, each with its anchor (D10).** (a)
Impl-filter: a rename pair whose plus/minus differ only inside
one `impl` block's method set fires when the enclosing `impl`
header plus the receiver-anchored call set prove the block is
the only candidate (the header is the filter; bare `Old` method
spellings without the header anchor stay ordinary). (b) Type:
hole-minimal anti-unification (fewest holes wins; ties veto)
plus exactly-one-hit over the file. (c) Signature-return: a
return-type-only header delta joins the existing sig-evolution
detector at N=1 under the definition-shape precision posture —
the concrete header pair is the emission, never a return hole,
because one proved evolution does not license a slot. Detector
names are classified in
[guard.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/guard.rs)
in the same change (`Precision` only where the scratch-apply
plus re-match proves the site; otherwise the entry keeps
`Strict` and this item does not claim it).

**Changes.** SPEC D.5 family list + D.4 (type/impl-filter/sig
cells gain the N=1 clause). Code: type arm in
[generalized.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/generalized.rs)
with hole-minimality in
[antiunify_types.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify_types.rs);
impl-filter arm in
[impl_single.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/impl_single.rs)
beside the existing impl singles; sig-return arm in
[sig_evolution.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/sig_evolution.rs);
guard entries per the table discipline (a detector that cannot
name its proof does not move).

**Acceptance.** `58`, `69`, `76`, `113`, `117` flip to
`CYCLE_OK` or carry reasons naming the failed proof (no bare
`t375` survives this item); the `13` reason is rewritten to
cover only its remaining residue; a hetero twin (two impls,
same method name, one renamed — the probe shape from this
review) fires only for the filtered block and leaves the other
block's calls untouched, real-apply byte-identical; every N≥2
emission byte-unchanged.

**Tests.** Inline per module (fire/veto pairs per sub-family,
including the two-impl filter control) plus cycle table moves.

**Non-goals / risks.** No generics/`where` invention: a type
pair needing a hole the anti-unifier cannot place stays
ordinary. Risk: hole-minimality ties are common in small types
(`Vec<T>` vs `Box<T>`) — ties veto, pinned, never guessed.

### WI-3 (P1, L): Precision singles III — macro, use, symbol, trait, impl, and insertion [carries old WI-9 / F2 family B]

**Problem.** The remaining `t375` mass: `09`/`75`/`86`/`97`
(macro/token-tree), `10` (impl method), `19`/`112` (trait),
`102` (use), `104` (symbol), `82` (const-block), `22`
(attr-strip), `06` (insertion anchor), `08` (qualified call),
`04` (two unrelated singles in one authored sweep fixture).
Each currently re-diffs ordinary at N=1 although its N≥2 or
definition-shape sibling fires.

**Decision — admission by anchor, family by family, in
detector order.** Macro/token-tree singles fire only on
definition-side or invocation-side totality *within the token
tree they edit* (the exactly-one-hit proof runs per macro body
or per invocation, never per file — expansion is never
inspected). Impl/trait singles fire as precision definition
singles with their enclosing header as anchor (the WI-2
impl-filter machinery, def-only arm). Use/symbol singles fire
when the pair is a whole path (the path is the anchor); a
single-segment use rename without its definition or a use site
in scope stays ordinary. Const-block and attr-strip singles
fire under the definition-shape guard (owner header is the
anchor). Insertion singles (`06`) fire when the inserted line's
neighbours anchor it uniquely (the `-N,0` anchor law from the
WI-19 pure-insertion fix is the model). Qualified-call singles
(`08`) join the existing qualified detector at N=1 under the
matcher-run proof. `04` closes only if WI-15's single-pair
partition proves both singles independently; otherwise its
reason is rewritten to name the unproven half. `30`
(trait-side-only) stays ordinary *by design* and its reason is
rewritten without a `t375` tag, so the table stops implying it
is backlog.

**Changes.** SPEC D.5 + D.4 (macro/use/trait cells). Code:
[macros.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macros.rs),
[macro_rule_body.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macro_rule_body.rs),
use arm beside [paths.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/paths.rs),
guard entries per WI-2's discipline. Any file over 1600 keeps
new code in a sibling module (D5).

**Acceptance.** Each family's entries flip to `CYCLE_OK`
slice-by-slice or carry rewritten proofs; a macro-body template
hitting two arms stays ordinary, pinned; a use rename whose
definition also changed raises under WI-12/WI-18's intents, not
as two singles (registration order pinned by test); history
sweep compressing set must grow or hold.

**Tests.** Inline per module + cycle moves. The macro arms
include the negative control that no expansion API is consulted
(the proof input is the token tree text only).

**Non-goals / risks.** No proc-macro awareness, no `macro` 2.0,
no bare single-use macro detector (the macro-rules *use* side
is WI-16, follow-claimed only). Risk: token-tree templates
over-generalise fastest of all families — the per-body scoping
of the proof is pinned by test, never per-file.

### WI-4 (P1, M): `# symbol` beyond call shapes [carries old WI-10 / F9]

**Problem.** F9, unchanged since round 6: `# symbol` verifies
only in call-callee, call-argument, and method-receiver
positions (C14). The re-probed `let` failure is verbatim at
HEAD. Renaming a vendored path in a `let`, type, field, or use
position forces concrete-path patterns that miss re-exported
spellings silently.

**Decision — symbol verification extends to path positions the
resolver can prove, with the same fallback law.** `# symbol
Name=path` becomes legal in `let`-init, type, field-type,
struct-literal-path, and use-prefix positions: the compiler
classifies the hole's position; positions backed by the
crate-local index verify (`symbol: resolved`), unresolvable
positions require `# fallback=syntax` exactly as calls do today
(SPEC 19), and positions the index cannot attribute (macro
interiors, `qself` tails) reject with the C14-shaped error
naming the position. Matching stays exact-full-path-text at the
hole (SPEC 19.3); only the set of legal positions grows. The
C14 error text is rewritten to enumerate the legal positions
instead of “call shapes”, and SPEC 18.6's C14 row + AGENTS.md's
`# fallback=syntax` row move in the same change. WI-18's repair
gate owns the new arms' `repair` payloads (they enter through
the gate, not around it).

**Changes.** SPEC 19 + 18.6 C14 + 9.3. Code: position
classifier in
[pattern.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/pattern.rs)
(the C14 gate), resolver arms in
[matcher_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general.rs)
reusing the call-shape verification path — no second resolver.

**Acceptance.** The F9 probe (`let x = Old::compute();` with
`# symbol` + `# fallback=syntax`) applies syntactically and
reports the syntax posture; a resolvable twin reports
`symbol: resolved` and rewrites only the proved spelling
(re-exported twin pinned); type/field/use twins fire per
position; a macro-interior symbol still rejects, with the
position named.

**Tests.** Inline in `pattern.rs`/`matcher_general.rs`
`mod tests` (one fire + one wrong-namespace negative control
per new position) +
[pattern_compilation.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/tests/pattern_compilation.rs)
C14 arms extended in place (integration file, exempt from the
inline rule).

**Non-goals / risks.** No inference of symbols the author did
not declare. Risk: position misclassification silently widens
matching — every new position ships its negative control in the
landing change or the position does not ship.

### WI-5 (P1, M): Qualified-header renames — stable `async`/`unsafe`/`const`/`extern` slots classify [carries F3]

**Problem.** F3: the raiser admits a qualifier run only when it
*changes* (round-7 WI-5 joints). A pure rename whose header
carries token-identical qualifiers (`async fn foo` → `async fn
bar`) has a name-only delta the classifier never sees, so the
commonest async-era rename fragments. The engine has matched
these headers since SPEC 14.1 (`V Q fn … G (...) W` with the
qualifier set compared); only raising is blind.

**Decision — the header-delta classifier gains a name-only arm
over the full slot vector.** The shared classifier behind
[item_intent.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/item_intent.rs)
and
[item_joint_visqual.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/item_joint_visqual.rs)
classifies a header pair by *which slots differ*: name-only
difference fires the rename intent regardless of which other
slots are present-but-equal (vis, qualifier run incl. `extern
"ABI"`, generics, params, return, `where`). Emission is the
concrete header pair under the shared follow hunk — the
qualifiers written exactly as Rust writes them (D6), never
abstracted to `Q`, because one proved rename does not license a
qualifier hole (the round-7 WI-1/WI-5 rule, restated for the
stable case). Method headers inside `impl`/`trait` blocks get
the same arm (an `async fn` trait method renames like its plain
twin); `const fn` methods and other Rust-illegal qualifier
positions are not modelled (the input would not parse).

**Changes.** SPEC D.4 (`fn` + trait-method cells: name-only
clause over the full slot vector) + SPEC 14.1 cross-reference.
Code: classifier widening in `item_intent.rs` /
`item_joint_visqual.rs` (one classifier, two consumers — no
second header parser); `item_intent.rs` is 1994 lines, so the
arm lands in the existing visqual sibling and `item_intent.rs`
only re-exports it (D5, WI-22 pulled forward for this seam).

**Acceptance.** The five F3 probes (async, pub async, unsafe,
const, extern) raise to one follow hunk each, real-apply
byte-identical, `verified` on clean corpora; plain/generic/pub
emissions byte-unchanged; a qualifier-*changing* twin still
raises under WI-5's joint (registration order: joint before
name-only is impossible — the delta sets are disjoint — and a
test pins that neither arm claims the other's input).

**Tests.** Inline in the visqual module (slot-vector matrix:
each qualifier × name-only/change) plus one durable arm in
[fixtures_roundtrip_part2.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/fixtures_roundtrip_part2.rs).

**Non-goals / risks.** No qualifier *invention*: a header pair
differing in name plus a qualifier is WI-8's two-slot domain,
not this arm. Risk: `extern` ABI strings compare token-wise —
a whitespace-only ABI respelling must classify as equal
(normalized comparison, as today), pinned.

### WI-6 (P1, M): One-line definitions classify — `fn foo() { x }` is a definition [carries F4]

**Problem.** F4: intent detectors key on headers terminated by
end-of-line `{`. A definition whose body shares its header line
(single-line `fn`, one-line `impl` method, one-line trait stub
with a default body) is invisible as a definition, so its
rename fragments and — worse — a one-line method body was
misclassified as a trait-method *declaration* in the discovered
`865901c` case. The authored engine matches these items fine;
the blind spot is shared raiser/matcher header extraction.

**Decision — header extraction learns the brace-balanced
one-line form, rename-only.** The block-header extractor in
[block_headers.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/block_headers.rs)
(and the engine's item-span code it mirrors) recognises a
header whose `{ … }` closes on the same line: the item span is
that line, the header is the text before the brace, and the
body is the balanced interior. Admission is rename-only in
this item: name-only header delta (WI-5's classifier) plus a
byte-identical body interior fires the intent; any body delta
declines to WI-13 (scoped body pairs) or ordinary — this item
never rewrites a one-line body. Trait *declarations* (`fn
old(&self);`) keep classifying as declarations; the
discriminator is `;` vs balanced `{ … }`, stated in SPEC so
the `865901c` class cannot regress silently.

**Changes.** SPEC D.4 (one-line clause) + SPEC 13.4 (item-span
sentence for the one-line form). Code: extractor in
`block_headers.rs`, consumers in `item_intent.rs` and the
trait-method classifier; engine span parity pinned where the
engine extracts the same span (one extraction law, two
readers — divergence between them is a test failure, not a
dialect).

**Acceptance.** The F4 probe (one-line `fn` rename + call)
raises to one follow hunk; one-line method and trait-stub twins
fire likewise; a one-line definition with a changed body stays
ordinary (WI-13 territory, pinned); the `865901c` regression
shape (single-line method with body) classifies as a definition
with a body, pinned in the trait-method tests; multi-line
emissions byte-unchanged.

**Tests.** Inline in `block_headers.rs` and the trait-method
module (classification matrix: decl vs one-line body vs
multi-line body) plus durable arms in the roundtrip file.

**Non-goals / risks.** No multi-item-per-line splitting
(`fn a() {} fn b() {}` on one physical line stays ordinary —
git's line model cannot cite it honestly). Risk: brace
counting inside strings/comments on the header line — the
extractor counts with the existing literal-aware scanner (the
WI-2 skipper), never a raw brace count, pinned by a
string-brace control.

### WI-7 (P1, M): Proved intents outrank bytes — one precision allowance for joints and methods [carries F5; enacts D3]

**Problem.** F5: the byte guard is the only thing standing
between proved small intents and their emission.
`PRECISION_GUARD` (16 bytes, in
[definition_evolution.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/definition_evolution.rs))
is applied per hunk in at least three dialects today (flat
`+16`, per-claimed-site `16 × n`, and filter-block-adjusted),
and detectors not classified `Precision` in
[guard.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/guard.rs)
(`inherent_method_rename`, `trait_method_rename`, the joint
compositions) fall back to strict T375, where the follow hunk's
mandatory anchor block (impl header + `...` frame) alone exceeds
the native diff it replaces. The result is perverse: adding a
second call site makes a rename *more* semantic.

**Decision — one allowance law, classified by proof, not by
detector vintage.** (a) Every detector whose emission is
verified by a scratch-apply totality probe over its exact site
set (follow intents, joint compositions, method intents) is
classified `Precision` in `guard_for`; strict T375 remains for
sweep/template shapes whose minus pattern could match wider
than the diff. (b) The allowance is unified to
`semantic ≤ ordinary + 16 × claimed_sites + filter_block_bytes`,
where `filter_block_bytes` is the byte count of anchor lines
the emission is *required* to carry (enclosing `impl`/`trait`
headers, context headers) measured from the emission itself —
the per-emission filter-block pricing discovered in round 7,
generalised from its per-hunk home into the guard. (c) The
guard decision and its three inputs (ordinary bytes, semantic
bytes, allowance) become provenance the report can name
(WI-17 consumes `guard` declines with these numbers; an agent
must be able to see “proved, declined on bytes by N” — after
this item that state exists only above the unified allowance,
never inside it).

**Changes.** SPEC D.4 guard paragraph (one allowance law; the
per-detector guard table in SPEC is generated from `guard_for`
by WI-24 thereafter). Code:
[guard.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/guard.rs)
(classification + allowance function; the three open-coded
dialects in
[chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs)
and [lib.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/lib.rs)
collapse onto it — one decision point, as the module header
already promises).

**Acceptance.** The three F5 probes re-run: small method rename
fires as one follow hunk (WI-9 spends this), rename+arg-add at
one call fires as one joint (WI-8), rename+body at one call
fires as one joint (WI-13's small twin); a deliberately bloated
counter-case (semantic form exceeding the unified allowance)
still declines with `guard` provenance naming the overflow;
every emission that fired before fires byte-identically (the
allowance only admits, never respells).

**Tests.** Inline in `guard.rs` (allowance arithmetic:
per-site scaling, filter-block measurement, boundary equality)
+ `chain.rs` (dialect-collapse regression pins: the three
former formulas agree with the unified one on their pinned
corpora).

**Non-goals / risks.** No unlimited precision: above the
allowance, ordinary still wins and says so. Risk: allowance
creep by precedent — any future widening must change the one
formula in `guard.rs` with its SPEC sentence, never a
per-detector constant (the landing test asserts no second
allowance constant exists in the crate).

### WI-8 (P1, L): Two-slot rename joints — vis+return, rename+arg-add, rename+param-type at any N [carries F5 joint half; enacts D13]

**Problem.** Round-7 WI-5 admitted name + exactly one evolving
slot. Probes: `fn foo() -> i32` → `pub fn bar() -> i64` (name +
vis + return) fragments to ordinary definition + semantic call;
rename+arg-add at one call stays wholly ordinary (fixture `120`
fires only at two calls). Two-slot deltas are the everyday
“rename it and fix its signature while you're there” edit; the
half-semantic split is the failure mode the Goal forbids.

**Decision — the joint detector composes name + one
sig-evolution delta under the D13 probe, priced by WI-7.**
The `fn_joint` family gains a two-slot arm: header delta =
name plus exactly one sig-evolution class (vis run, qualifier
run, param-list evolution incl. arg-add/remove/retype, return
evolution, or one attr line — the classes the existing
single-slot joints and sig-evolution detector already prove
separately), with every other slot token-equal. Call sites ride
the follow set; call sites whose text must change to match the
new signature (arg-add with a concrete inserted argument,
retyped argument at anchored sites) are claimed only when the
native diff itself contains those plus lines and the joint
scratch-apply reproduces them exactly — the raiser never
invents an argument the diff does not show. Emission: one
shared-rule follow hunk carrying the concrete evolved header
pair (WI-5's concrete-spelling rule: no `V`/`Q`/return holes
for proved single evolutions). Two sig-evolution classes
(vis+return+generics) veto to ordinary, pinned.

**Changes.** SPEC D.4 `fn` joint cell (two-slot clause) + SPEC
14.7 cross-reference (slots written explicitly evolve — raising
now honours it for two slots). Code: joint arm in
[fn_joint.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/fn_joint.rs)
(1972 lines — new code lands in the existing
[fn_joint_body.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/fn_joint_body.rs)
sibling pattern, a new `fn_joint_sig.rs`, D5), probe reuse from
[fn_joint_ws.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/fn_joint_ws.rs).

**Acceptance.** Both probes raise to one follow hunk,
real-apply byte-identical; fixture `120`'s emission
byte-unchanged (the N=2 path must not respell when the site
count grows); single-slot joints byte-unchanged; a vis+return+
generics triple stays ordinary, pinned; an arg-add whose call
sites the diff does *not* update stays ordinary (never invent
arguments), pinned.

**Tests.** Inline in the new joint module (slot-class matrix ×
call-count 1/2) + durable arms in the roundtrip file. Bench:
diffrs benches `45`/`46` re-pinned only if emissions change;
their token claims are WI-23's diet problem, not this item's.

**Non-goals / risks.** No body delta in this joint (WI-13 owns
bodies; name+sig+body in one diff stays ordinary until both
land and a probe demands the triple). Risk: arg-add call
claiming over-reaches to same-name other-struct methods — the
follow anchor law (D8) owns receiver proof, unchanged.

### WI-9 (P1, M): Method intents at small N — one follow hunk for inherent and trait methods [carries F5 method half]

**Problem.** Inherent/trait method renames are classified
`Strict` in `guard_for`, so the small case (one definition, one
anchored call — the commonest method rename there is) fragments
into an ordinary definition beside a `Receiver.old()` sweep,
while the long-signature case fires as one follow hunk. The
fragmented form is not wrong, but it splits one intent across
two unrelated hunks and forfeits the definition-set accounting
(D9): nothing in the report states the sweep and the definition
are the same rename.

**Decision — method intents join the precision class on
WI-7's allowance, with the follow hunk as the only small-N
form.** `inherent_method_rename` and `trait_method_rename`
move to `Precision` in `guard_for` (their totality probe is
the same scratch-apply the fn intents use — the classification
was vintage, not proof). Emission at any site count is the
single `# follow=uses` hunk with the `impl`/`trait` filter
block (the form the long-signature case already emits); the
`Receiver.old()` sweep remains the fall-back only when the
follow probe itself declines (unanchored receivers), never as
a byte-saving substitute for a proved follow. Trait-method
intents keep owning every definition in the set (decl + each
impl def, D9) at small N too.

**Changes.** SPEC D.4 (method cells: precision clause). Code:
[guard.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/guard.rs)
classification, detector arms in
[methods.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/methods.rs)
(1961 lines — new code in a sibling module, D5).

**Acceptance.** The F5 small-method probe raises to exactly
one follow hunk, `verified` clean, real-apply byte-identical;
the long-signature e2e
([wi6_inherent_method_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/wi6_inherent_method_e2e.rs))
emission byte-unchanged; the unproven-receiver twin
(`o.run()` on an unrelated struct) still declines the follow
wholly (no def-only partial follow), pinned; trait decl+impls
at N=1 fire as one intent or stay wholly ordinary.

**Tests.** Inline in the methods sibling (small/long ×
inherent/trait × proven/unproven) + the existing e2e files
extended in place.

**Non-goals / risks.** No change to authored method matching.
Risk: precision-class creep — the move is justified only
because the probe is identical to the fn intents'; the guard
test must assert the proof kind per precision detector (WI-7's
table), not just the class name.

### WI-10 (P1, L): Follow anchors through wrappers — Deref receivers and trait-return chains [carries F6; enacts D8]

**Problem.** F6: `b: Box<S>` receivers skip `off-target`;
`c.clone().get()` skips `chain-unproven`. Both are receiver
shapes Rust resolves by rules the change text plus the
workspace already state: `Box<S>` derefs to `S` by language
rule, and `Clone::clone` returns `Self` by its trait
signature, which is in the workspace or is `std`'s most
imitated shape. Today the anchor table knows inherent
constructors and same-file stated returns only.

**Decision — two anchor extensions, each proved, each with
its skip reason when unproven (D8).** (a) Deref receivers: a
receiver typed `Box<T>`, `Rc<T>`, `Arc<T>`, `&T`, or `&mut T`
(in a param annotation, `let` annotation, struct-literal-free
binding, or `self` receiver of an `impl` for such a type)
anchors through exactly one deref step to `T`; a workspace
`impl Deref for C` with a stated `Target` anchors likewise
(one step; `DerefMut` chains and double-deref stay
`off-target`). The wrapper set is closed and named in SPEC —
it is the `std` smart-pointer vocabulary an agent already
knows, not an open heuristic. (b) Trait-return chains: a
method call whose receiver is `recv.m()` where `m` resolves
to a trait method (same-workspace trait signature) whose
return type is `Self` anchors the chain to `recv`'s anchored
type; inherent stated returns already work and are unchanged.
Unstatable steps keep `chain-unproven`; ambiguous trait
candidates keep their existing skip.

**Changes.** SPEC 14.8 anchor enumeration (deref step +
trait-`Self` return; reason codes unchanged — `off-target`
narrows, no new code is minted). Code: anchor arms in
[use_graph_method.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_method.rs)
and [use_graph_follow.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_follow.rs)
(timeout/depth interplay with WI-11 pinned: deref consumes no
chain depth, it rewrites the receiver type before the chain
walk).

**Acceptance.** The F6 probes: `Box<S>` call rewrites and the
run verifies; `clone().get()` rewrites when `Clone` is the
workspace trait or the closed `std`-shape signature, and the
run verifies; an `Rc<Other>` namesake still skips `shadowed`;
a user type merely *named* `Box` in another module does not
anchor (path must resolve to the `std`-vocabulary spelling or
a workspace `Deref` impl), pinned.

**Tests.** Inline in the owning modules' `mod tests`;
for `use_graph_follow.rs` (473 lines + 321 exiled) WI-22 is
pulled forward for that one module first (move its sibling
tests home verbatim, then add the new arms inline — the
sibling files must not grow) + durable arms in
[inherent_follow_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/tests/inherent_follow_e2e.rs)
extended in place.

**Non-goals / risks.** No `Deref` chasing beyond one step, no
cross-crate trait signatures beyond the closed `Clone`-shape
(`Self`-returning, single-method proof from the call's own
trait resolution where the trait is in-workspace; the pure-`std`
case anchors only when the receiver type is independently
anchored). Risk: a same-named local `Box` type — resolution,
not spelling, decides, and the negative control pins it.

### WI-11 (P2, M): Chain depth two — constructor chains prove a second hop [carries F6 depth half]

**Problem.** `S::make().mid().run()` skips `chain-depth` at
the second hop by a cap set when only one hop was proved
(round-7 WI-11). Builder-style APIs (`S::builder().timeout(x)
.build()`, `Client::new().connect().send()`) live at depth
two; depth one covers the demo, depth two covers the idiom.

**Decision — the constructor-chain walk generalises from one
hop to a bounded two, with per-hop proof.** Each hop must
independently satisfy the existing constructor proof (same-
file or workspace `impl` on the current type, stated return
of `Self`/the next type, name used only to select candidates
— never as proof). The walk is iterative over the anchored
type, capped at two hops; a third hop keeps `chain-depth`.
Field chains (round-7 slice b) compose: a deref step (WI-10)
plus two hops is the maximum modelled chain, stated in SPEC
as one sentence so the bound is a contract, not a constant
someone tunes. Failure at hop *k* skips with the existing
reason at that hop (`chain-unproven` / `chain-depth`); proven
prefix sites still rewrite (a chain proves its target method
or it does not — there is no partial chain rewrite of the
intermediate hops themselves, which are never rename targets
unless separately renamed).

**Changes.** SPEC 14.8 (chain bound sentence). Code: chain
walk in
[use_graph_method.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_method.rs)
(the one-hop arms become the iteration step; no parallel
walker).

**Acceptance.** The F6 two-hop probe rewrites `…mid().run()`
→ `…mid().execute()` and verifies; a three-hop twin keeps
`chain-depth`, pinned; a hop whose return type is unstated
keeps `chain-unproven` at that hop, pinned; one-hop emissions
byte-unchanged.

**Tests.** Inline in `use_graph_method.rs` (depth 0/1/2/3 ×
proven/unproven matrix) + e2e arm in the inherent-follow file.

**Non-goals / risks.** No iterator-adapter chains
(`iter().map().collect()` as rename *targets* are method
renames whose receivers WI-10(b) anchors only when each
adapter's return is stated; general adapter typing is a type
inference project and stays out). Risk: combinatorial anchor
cost — bounded by the two-hop cap and the candidate-name
pre-filter, with the scale gate (WI-21) gaining a chained case.

### WI-12 (P1, L): Joints cross files — joint `impl` and deletion joints on the workspace probe [carries F7; enacts D8/D13]

**Problem.** F7: the two newest joint families (joint `impl`,
rename+deletion) were built on single-section probes while the
workspace probe (cross-file scratch-apply over the pre-image
map) already powers fn/item/assoc/trait-method/member
intents. The cross-file twins therefore fragment — worst case
a follow-free trait rename in one file beside ordinary text in
another, which reads as two unrelated edits.

**Decision — joint detectors consume the shared workspace
probe; file purity stops being a gate.** (a) Joint `impl`: the
trait pair, self-type pair, and composed header pair may sit
in different files; totality counts over the workspace
pre-image, emission is the shared-rule follow composition with
one section per definition file (the assoc pre-pass shape —
document-first hunk carries `# follow=uses`, siblings share
the rule). (b) Deletion joints: the def+deletion pair in one
file with its follow consequences (live uses of the deleted
member, of the renamed container) counted workspace-wide;
live uses keep reporting as `orphans` capping at `partial`
(round-7 WI-21 slice 3 law, now cross-file). (c) The member
pre-pass's “file/section-pure partial” posture (proven files
collapse, unproven stay ordinary, mixed sections veto) is
replaced for these joints by the joint law: the whole joint
fires or the whole joint stays ordinary (D13) — a cross-file
joint that half-fires is exactly the fragmentation this item
exists to remove. Single-file emissions byte-unchanged (the
workspace probe restricted to one file must reproduce them).

**Changes.** SPEC D.4 (joint + deletion cells: workspace
clause; the file-purity sentences deleted, not worked around).
Code: probe sharing via
[pre_pass.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/pre_pass.rs)
(the whole-patch pre-pass chain), joint detectors in
[item_joint_impl.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/item_joint_impl.rs)
and [item_joint_delete.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/item_joint_delete.rs)
consuming it; emission via
[item_joint_emit.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/item_joint_emit.rs).

**Acceptance.** Both F7 probes raise to the composed intent,
real-apply byte-identical; a third-file off-diff use of either
old name vetoes the whole joint, pinned; single-file fixtures
`128`/`131`/`142`/`143` emissions byte-unchanged; bench `39`
(assoc) gains its cross-file joint sibling measurement in
WI-23's re-pin, not here.

**Tests.** Inline per detector (cross-file fire/veto matrices)
+ cross-file durable arms in the roundtrip file.

**Non-goals / risks.** No cross-*crate* joints (workspace =
the `--repo` tree). Risk: probe cost grows with files touched
— bounded by the existing `ls-files` loader; WI-21's scale
gate gains a two-file joint case.

### WI-13 (P1, L): Body joints II — `let` and multi-expression bodies join the rename [carries F4 body half / old WI-20 residue; enacts D13/D16]

**Problem.** WI-20 slice 1 admits exactly one whole-line
`syn::Expr` body pair under a rename-only header. Probes: a
rename plus a `let y = x + 1` → `let y = x + 2` body delta, and
a rename plus two expression deltas, each fragment into an
ordinary header beside per-line general-expr rules under
separate rule names — related edits, unrelated report entries,
no single intent to audit. Multi-statement bodies are the norm;
single-expression bodies are the demo.

**Decision — scoped body pairs generalise from one expression
to a bounded statement set, still span-scoped, still
loud.** The fn hunk admits up to three additional body pairs
under the renamed header, each pair one whole body line that
is either a `syn::Expr` (slice-1 law) or a plain `let` statement
(`let` binder + init expression; binder renames inside the body
pair enter binder identity, D11, or veto — never text). The
engine applies body pairs only inside the renamed definition's
body span (the slice-1 `body`-labelled edit, generalised to a
labelled edit *set*); each body minus must occur exactly once
as a whole line inside that body, else `RSMPL_MATCH_CONFLICT`
per pair, never a sweep. Raising fires only under the D13
joint probe (header + body set + follow set in one
scratch-apply). Header slot deltas compose only with the
rename-only header in this item (name+sig+body stays ordinary;
WI-8 + this item landing is the prerequisite for any triple,
which needs its own probe).

**Changes.** SPEC 14.7 body paragraph (the exception widens:
one expression → up to three whole-line expr/`let` pairs; the
“bodies are preserved unless a scoped body pair is written”
law unchanged in shape), SPEC D.4. Code: engine span-scoped
edit set in
[matcher_fn_body.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_fn_body.rs)
+ dispatch in [plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs);
raiser arm in
[fn_joint_body.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/fn_joint_body.rs).

**Acceptance.** The two probes raise to one follow hunk each
and verify clean; authored multi-pair body hunks apply with
`span_edits` labelling every body edit; a body pair matching
two in-body lines fails loud; a `let` body pair whose binder
would capture a same-name outer use vetoes via binder identity
(pinned, orphans named); the existing authored corpus applies
byte-unchanged (regression pin is the corpus itself).

**Tests.** Inline in `matcher_fn_body.rs`/`fn_joint_body.rs`
(pair-count 1/2/3/4, expr/`let` mix, capture veto) + durable
e2e in the roundtrip file.

**Non-goals / risks.** No arbitrary statement rewrites
(`if`/`match`/loop lines as body pairs stay ordinary hunks),
no full-body replacement. Risk: this widens the one language
exception to body preservation — the AGENTS.md row must stay
one line (“extra pairs under the header rewrite inside that
body only, each exactly once”) or the widening does not land
(D6).

### WI-14 (P2, L): Move completion II — attribute, trait, and container-delta moves [carries F8 move half; enacts D13/D16]

**Problem.** Round-7 WI-19 closed multi-line, signature,
body-evolving, container, and impl-block moves. What still
re-diffs as bare delete+insert: a move that also adds or
changes one attribute line (probe: `#[inline]` added during
the move), a trait move whose methods ride along (probe: trait
moved between files with one method renamed stays ordinary on
both sides), and a container move with a vis or signature-side
delta (SPEC's container `as` law is name-only by statement).

**Decision — one delta axis per slice, each under the move
joint probe, each reusing an existing splice.** (a) Attr axis:
a move whose only extra delta is one attr line added, removed,
or whole-balanced-replaced on the moved item fires as the
existing move form plus that attr line written where Rust
writes it (above the paste header, SPEC 14.6 delta law); the
attr interior is opaque (round-7 attr-opacity law) — an attr
*value* expression mentioning the old name vetoes, never
rewrites. (b) Trait axis: a moved `trait` whose body delta is
exactly one method rename (decl side) fires as a trait `as`
paste generalising the container form (`+ trait Old as New {
... }` — one SPEC paragraph mirroring the `fn` occurrence law:
only the header ident and `Self::`/self-path occurrences
rewrite; method names, macro names, attr interiors veto loud).
Trait-*impl* moves stay with WI-12's joint machinery (never
the `as` form — SPEC already reserves that header). (c)
Container delta axis: a container move with exactly one vis
delta (no signature — containers have none) fires as the
concrete-vis paste (existing evolution-paste form) composed
with the `as` body law; vis+attr together veto. Each axis owns
its veto pins; an axis that cannot prove stays ordinary and
SPEC's move residue list shrinks by exactly that axis.

**Changes.** SPEC 20 (three residue sentences replaced by
their laws) + D.4 move cells. Code:
[move_evolve.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/move_evolve.rs),
[move_evolve_container.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/move_evolve_container.rs),
capture splice arms in
[capture.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/capture.rs)
— all through the WI-2 occurrence classifier, never a new
scanner.

**Acceptance.** Each probe fires byte-identically (move +
real-apply; `--verify build` where the fixture is a crate);
fixtures `33`/`39` and the WI-19 move suite byte-unchanged;
attr-value-mentioning-the-name vetoes pinned; two-axis
(attr+vis) container move stays ordinary, pinned.

**Tests.** Inline per module (axis × veto matrices, smallest
focused regression per axis) + roundtrip move arms extended
in place.

**Non-goals / risks.** No signature deltas on containers
(there are none to delta), no associated-item moves beyond
the trait axis, no move+body-arbitrary-edit (body deltas
beyond the rename remain WI-13's scoped-pair domain in place,
ordinary across files). Risk: template proliferation — each
new `as` kind reuses the one occurrence law verbatim in SPEC
(one paragraph, per-kind sentences) or it does not land.

### WI-15 (P2, M): Partition residue — the header+statement single block [carries F11]

**Problem.** Fixture `12` (`12-stmt-rewrite`) is the last
mixed shape with no owner: a definition header line fused with
a `let` line in one native hunk. The `single_block` detector
fuses pure `let` runs only and SPEC calls any mixed run a
permanent non-goal; WI-4's partition splits *runs of pairs*,
not a header fused with its first statement. Agents produce
this shape whenever they rename a fn and touch its first line
in the same edit — precisely WI-13's in-place shape, except
the raiser meets it as an unstructured hunk.

**Decision — a header+statement pair partitions as header
(intent) plus statement (scoped pair), under WI-13's law.**
A two-line minus/plus group whose first pair is a rename-only
or WI-8 joint header and whose remaining one-to-three lines
are `syn::Expr`/plain-`let` lines inside that header's body
raises as the WI-13 joint (header pair + scoped body pairs,
one follow hunk) whenever the body lines lie inside the
renamed definition's span in the pre-image. This is not a new
emission: it is WI-13's detector fed by WI-4's partitioner
(the partitioner proposes the split, WI-13's probe proves it).
Interleaved (A B A) order still never splits (order is
evidence, not an obstacle), multi-header groups stay
ordinary, and the SPEC “permanent non-goal” sentence is
narrowed to exactly those two exclusions — what was actually
being protected was one-pair-one-claim atomicity, which the
scoped-pair law preserves.

**Changes.** SPEC D.5 (exclusion prose for `12` rewritten;
`single_block` guard paragraph narrowed) + D.4 partition
cell. Code: partition proposal in
[partition.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/partition.rs)
feeding [fn_joint_body.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/fn_joint_body.rs);
provenance for the declined split via WI-17.

**Acceptance.** Fixture `12`'s native twin raises as one
joint hunk, real-apply byte-identical, and `12-stmt-rewrite`
leaves `KNOWN_DIVERGENT` (or its reason names the specific
unproven line — “still mixed” without a named line does not
close this item); fixture `99-partition-partial-mixed`
behaviour byte-unchanged; an interleaved control stays
ordinary, pinned.

**Tests.** Inline in `partition.rs` (header+statement fire /
interleave veto / multi-header veto) + cycle table move.

**Non-goals / risks.** No splitting of a single line across
two semantics, ever (D13's per-line single-claim law is the
surviving half of the old non-goal). Risk: partition
proposing splits WI-13 cannot prove wastes probe time —
bounded by trying this proposal only after whole-hunk
detectors decline (the round-7 ordering).

### WI-16 (P2, M): Macro-rules completion — the use side joins by follow [carries F8 macro half]

**Problem.** Fixture `45` still pins a split intent: the
`macro_rules!` definition re-infers as a precision item single
while every invocation stays ordinary, so a macro rename —
the change with the *least* visible call sites in Rust —
raises as its least auditable form. Round 7 deferred this as
“slice 2, follow-claimed only”; it never landed. Separately,
WI-3's macro singles must not grow into a use-side detector by
drift.

**Decision — macro invocations join only under the
definition's follow, claimed by path identity.** A
`macro_rules!` rename (def pair proved as the precision
single) plus invocation sites raises to the definition hunk
with `# follow=uses` when every invocation of the old name in
the workspace is claimed by the macro follow probe (same
machinery as `macro_rules_rename`'s existing follow: textual
`old!` / `old::` path tails, macro interiors of *other* macros
opaque and therefore vetoing for the follow but not for the
def single). If any invocation is unclaimable (inside another
macro's body, shadowed by a same-name `macro` 2.0 or local
macro), the intent degrades to def-single + `orphans` naming
those sites at `partial` — never silent, never a bare
single-use detector (WI-3's exclusion stands). The authored
side already supports this follow; this item is raising-only,
plus SPEC stating which degradation is honest.

**Changes.** SPEC D.4 macro cells (follow-claimed use law +
degradation). Code: follow claim in
[macro_rules.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macro_rules.rs),
probe reuse from the member/item pre-pass
([pre_pass.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/pre_pass.rs)).

**Acceptance.** Fixture `45`'s corpus form (def + clean
invocations) raises as one follow hunk and its
`KNOWN_DIVERGENT` reason is rewritten to cover only the
degraded residue or the entry moves; an invocation inside
another macro body degrades with that site named in
`orphans`, pinned; WI-3's invocation-single emissions
byte-unchanged (follow-claimed and bare singles are disjoint
by the presence of a def pair — test pins the boundary).

**Tests.** Inline in `macro_rules.rs` (clean/degraded/veto
matrix) + roundtrip arms.

**Non-goals / risks.** No expansion inspection, no
`#[macro_export]` path resolution beyond the workspace text
probe. Risk: macro name shadowing is lexical in ways text
probes approximate — unresolvable shadowing degrades (safe
direction), pinned, never guesses.

### WI-17 (P1, M): Provenance completeness — every ordinary section explains itself [carries F10; enacts D12]

**Problem.** F10: declined provenance covers roughly a third
of the attempted detectors and nothing in text mode. The
agent's question — “which intent did you try for this hunk,
and what proof failed?” — is answerable only by re-running
detectors mentally over the file, which is the reconstruction
the report exists to eliminate.

**Decision — provenance becomes a complete trial log with a
closed reason taxonomy.** (a) Every detector the chain or a
pre-pass attempts for a hunk records exactly one outcome:
`fire` (the hunk is not ordinary — no record needed), or a
decline with one reason from the closed set {`no-match`,
`guard` (with WI-7's three numbers), `totality` (site-count
mismatch, with both counts), `veto` (a named veto: off-diff
use, opaque hit, ambiguous anti-unification), `precondition`
(a stated shape premise failed, named)}. Detectors never
attempted for a hunk (wrong section kind, pre-pass-only,
feature-gated) are listed once per section as `not-tried`,
distinct from declines — the absence is data, not silence.
(b) Round-7's “explicitly silent” pre-pass declines and
inner-single silences are re-expressed in this taxonomy (the
silence was a workaround for an incomplete log; the log now
carries them, text mode included: one `declined:` line per
ordinary hunk, detector + reason, budgeted like other report
lines and lifted by the same flags). (c) JSON gains no new
top-level keys beyond the completed `declined` entries
(additive within v4; WI-23 freezes the set at v5).

**Changes.** SPEC D.2 (provenance paragraph + reason
taxonomy). Code: trial recording in
[prov.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/prov.rs)
+ [lib.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/lib.rs)
(`HunkProvenance` completed for the chain path, not only the
single/pair path), rendering in
[main.rs (diffrs)](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs)
(text + JSON parity pinned by test).

**Acceptance.** The F10 comment-only probe's JSON lists every
chain detector exactly once across `declined`/`not-tried`;
text mode prints the same trial for the same input
(parity test); a `guard`-declined hunk shows its overflow
number; no firing emission changes bytes; report-budget tests
pin the truncated form (totals still pre-truncation truths).

**Tests.** Inline in `prov.rs` (taxonomy completeness: a
synthetic detector registry must classify every chain
registration or CI fails — the registry is enumerated from
the chain source, house source-scan style) +
[report_budget_e2e.rs (diffrs)](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/report_budget_e2e.rs)
extended in place.

**Non-goals / risks.** No per-site decline detail beyond the
hunk level (sites belong to fired rules). Risk: provenance
volume on large ordinary diffs — bounded by one line per
detector per hunk and the existing budget machinery; the
totals, not the log, are the default truth (D12).

### WI-18 (P2, S): Use-split — regroup symmetry for the path template [carries F12 use half]

**Problem.** WI-14 (round 7) landed merge (`use a::X; use
a::Y;` → `use a::{X, Y};` as a path template). The reverse —
one grouped `use` split into separate lines, the rustfmt-
driven direction agents hit when adding a path that does not
fit the group — still re-diffs ordinary (probe this run).
Merge-without-split makes the template a one-way dialect:
patches raised from split-heavy codebases never compress.

**Decision — the regroup law gains its inverse, same
guardrails.** A minus grouped `use` line plus N plus lines
whose trees are exactly its expansion (same paths, same
tails, one per line) emits the path-template pair in the
split direction (`- use a::b::{X, Y};` / stated expansion
lines), under the same segment-wise tail law as merge
(SPEC 14.9: `as`/glob/`pub use` tails preserved; any tail the
law cannot state vetoes). Round-trip law: merge(split(x)) ==
x canonically and split(merge(x)) == x canonically, both
pinned — the two directions are one equivalence, judged by
WI-1's instantiation rule where counts differ.

**Changes.** SPEC 14.9 (split clause beside merge) + D.4.
Code: split arm beside the merge in the use machinery of
[paths.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/paths.rs)
(shared expansion/grouping core — no second tree walker).

**Acceptance.** The probe raises as a path template,
real-apply byte-identical; merge emissions byte-unchanged;
split of a group with a glob or `as` tail vetoes to ordinary,
pinned; canonical round-trip pair pinned in the use e2e.

**Tests.** Inline in `paths.rs` (merge/split/veto matrix) +
[use_group_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/tests/use_group_e2e.rs)
extended in place.

**Non-goals / risks.** No nested-group restructuring
(`a::{b::{X}, Y}` reshapes stay ordinary). Size S: the tree
machinery exists; the risk is tail-law asymmetry, owned by
the round-trip pins.

### WI-19 (P1, L): Property cycle harness — random patches, both directions, in CI [carries old WI-23 / F14]

**Problem.** F14, carried twice: correctness is argued from
144+121 pinned fixtures the detectors were built to pass.
Nothing generates the patch an agent *would* write (random
metavars, stale ranges, overlapping hunks, macro interiors)
or the diff an agent *would* produce (random intent
compositions), so over-matching, non-idempotence, and
canonical divergence outside the anecdote set are invisible
until a user hits them.

**Decision — a seeded generator + invariant suite running
in `cargo test`, no new dependencies.** Generator A (authored
side): random RSmPL patches from the SPEC 18.6 supported-shape
grammar (random shapes × random metavars × random ranges,
including deliberately stale ranges and `# optional`/`#
repeat` arms) applied to random small parseable Rust files
(template mutation). Generator B (native side): random native
mutations drawn from the *intent catalogue* (SPEC D.4 cells:
rename/sig/body/move/attr/joint compositions — the catalogue,
not detector internals, to keep the generator honest).
Invariants on every case: (1) tree-losslessness both ways;
(2) fixed point (`S″ == S′` byte-identical; second-raise
stability); (3) verdict honesty — `verified` implies zero
orphans/skips recomputed independently by the harness, never
by reading the report's own fields (D2); (4) WI-1 canonical
equality wherever the harness claims it; (5) no panic, ever
(a panic fails with the seed printed). Seeds fixed in CI;
a failing seed becomes a pinned fixture in the same change
that fixes it. A wide run is a documented `tools/`
invocation, not a CI gate.

**Changes.** SPEC D.5 (harness paragraph: invariants stated
as contract). Code: harness as *integration* tests under
[crates/rsmpl_diff_infer/tests/](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/tests/)
(exempt from the inline rule; the generators' own unit tests
stay inline in the harness crate). Seeded RNG implemented
in-harness (no `proptest` for ~200 lines of generation; if
generation outgrows it, that is a later decision with its
own pin).

**Acceptance.** Harness green in `cargo test --workspace`
at the pinned seed set (≤60 s added, enforced by case
counts, not wall-clock asserts); at least one historical bug
shape from the T-number corpus is re-derived by the generator
and pinned — a harness that has never caught anything is
decoration, and the landing change must name what it caught.

**Tests.** The harness is the test. **Non-goals / risks.**
No coverage-guided fuzzing, no `cargo-fuzz`. Risk: generator
bias (it generates what its author imagined) — mitigated by
drawing mutations from the D.4 catalogue and by WI-20's
history gate covering real commits the generator never
imagined.

### WI-20 (P2, M): History gate — the sweep becomes a CI floor, not a ritual [carries F14 rate half]

**Problem.** `tools/history_sweep.py` runs by hand. This run:
20/20 convert, 6 compress — and nothing would fail if next
month read 20/20 convert, 2 compress. The tool's own history
is the cheapest real-world corpus available (768 commits of
exactly the mixed doc/code/fixture diffs agents produce), and
its compression rate is the only raising metric that cannot
be gamed by adding fixtures.

**Decision — a pinned-range sweep gate with a rate floor and
named exemptions.** (a) The sweep runs in CI over a pinned
rev-range (the range is a constant in the gate script;
 advancing it is a deliberate act in a commit, like moving a
fixture). (b) The gate asserts: conversion failures == 0,
every converted patch `--check`s clean via the T361 loop
(WI-16 machinery, shared implementation), and the compressing
count is ≥ the pinned floor (set at landing from the measured
range; raising the floor is encouraged, lowering it requires
the commit to name the detector change that cost the rate).
(c) Non-compressing commits are listed with their WI-17
declined provenance top reason — the gate output doubles as
the detector backlog, ranked by real frequency instead of
fixture anecdote.

**Changes.** SPEC D.5 (gate paragraph). Code:
[history_sweep.py](/home/ai/git/tools/rsmpl-git/tools/history_sweep.py)
gains `--gate` mode (pinned range + floor from a checked-in
pin file); gate wiring documented beside
[run.sh](/home/ai/git/tools/rsmpl-git/example-benches/run.sh)
(scale-gate precedent: warn vs fail stated explicitly — this
one fails on conversion errors, warns on rate within 2 of the
floor, fails below it).

**Acceptance.** Gate green at HEAD with the floor pinned to
the measured rate; deliberately breaking a detector (mutation
check in review) drops the rate and fails the gate naming the
lost commits; the gate runs in the documented CI command
sequence without network or history rewriting.

**Tests.** The gate script's self-test (synthetic two-commit
repo, compressing + ordinary arms) beside the sweep tooling.

**Non-goals / risks.** No full-history rewrite, no gate on
wall-clock. Risk: the corpus is this repo's own style —
mitigated by WI-19's generator for breadth; the floor measures
*regression*, not universal raising quality, and SPEC says so.

### WI-21 (P2, M): Raising scale gates — diffrs gets the benches applyrs has [carries F14 scale half]

**Problem.** Scale evidence is one-sided: `example-benches`
pins two applyrs scale gates (follow-wide 3000 lines,
postcheck-deep) and zero diffrs raising gates, although
raising is the superlinear side (per-section detector chain ×
scratch-apply probes × workspace pre-passes over every file
the diff touches). WI-10/WI-11/WI-12 each add anchor or probe
work whose cost is currently unmeasured; the first symptom on
a real repo would be an agent timeout, not a test failure.

**Decision — two diffrs scale benches mirroring the applyrs
ones, warn-only like theirs.** (a) Raise-wide: a rename with
200 anchored call sites across 20 files must raise+`--check`
within the pinned budget (set at landing from measurement,
2 s class). (b) Raise-deep: a 300-item file with 50 mixed
hunks (the WI-4 partition worst case) within its budget
(5 s class). Both live in `example-benches/diffrs/` as
generated corpora (generator script checked in, corpora
regenerated by the bench runner — no megabyte fixtures in the
tree), measured by `run.sh` beside the existing scale table,
warn-never-fail on wall-clock as today.

**Changes.** SPEC: none (bench contract only). Code:
generator + two bench directories under
[example-benches/diffrs/](/home/ai/git/tools/rsmpl-git/example-benches/diffrs/),
scale table arms in `run.sh`.

**Acceptance.** Both benches run green at landing with budgets
printed; a deliberate probe-cost regression (mutation check:
remove a candidate pre-filter) visibly blows the raise-deep
budget in local runs; `run.sh` output format unchanged for
existing rows.

**Tests.** Bench generation is deterministic (seeded); the
runner's byte tables for existing benches unchanged.

**Non-goals / risks.** No CI wall-clock assertions (shared
runners lie); budgets are review artefacts, as the existing
scale gates state. Risk: generated corpora drift from real
Rust shapes — they reuse WI-19's intent catalogue, so the
two harnesses share one imagination and WI-20 covers the gap.

### WI-22 (P1, M): Inline tests restored, production splits finished [carries old WI-22 / F13; enacts D5]

**Problem.** F13: 2699 lines of unit tests still live in five
`#[path]` sibling files, against the project convention (and
this plan's standing instruction) that unit tests stay inline
in `mod tests` at the bottom of the owning file:
[plan_tests.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan_tests.rs) (650),
[postcheck_move_tests.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck_move_tests.rs) (1076),
[postcheck_move_sweep_tests.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck_move_sweep_tests.rs) (124),
[use_graph_tests.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_tests.rs) (528),
[use_graph_follow_tests.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_follow_tests.rs) (321).
The count *grew* by 16 lines during round 7, whose items
(WI-10/WI-11 in new numbering) correctly refused to grow them
further and paid with inline tests in awkward homes. Every
round defers this because the owners are large; the deferral
is why the owners keep growing without their tests travelling
with their code.

**Decision — move tests home module-by-module, splitting
production code first where the owner would exceed 2000.**
Order per module: (1) split production code on a semantic
seam until the owner is ≤1600 lines, tests untouched; (2)
move the sibling file's tests into the owner's `mod tests`
verbatim (no rewrites — a test that fails after a verbatim
move is a finding, not an edit); (3) delete the sibling file
and its `#[path]` attribute. Modules in size order:
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs),
[postcheck_move.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck_move.rs),
[use_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph.rs),
[use_graph_follow.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_follow.rs).
The same pass finishes the F13 watch list: any production
file still above 1900 lines at this item's start
(`item_intent.rs` 1994, `fn_joint.rs` 1972, `methods.rs`
1961, `matcher_fn.rs` 1958, `postcheck.rs` 1952 at HEAD)
leaves below 1900 via a named seam, whether or not it owns
exiled tests. Enforcement: a workspace test (source-scan
house style, shared scanner with WI-17's registry and
WI-24's generators) fails if any `*_tests.rs` sibling or
`#[path = "*_tests.rs"]` reappears under `crates/*/src/`;
e2e/integration files under `tests/` are explicitly exempt
and listed.

**Changes.** No SPEC change. Code moves only; any production
split names its seam in the commit/PR description (split
production, never tests, D5).

**Acceptance.** All five siblings deleted; `cargo test
--workspace` test count identical before/after (3964 at HEAD
— the count is the proof nothing was dropped); no `.rs` file
over 2000 lines, none of the five watch files over 1900; the
gate test present and mutation-checked (re-adding a stub
sibling fails it).

**Tests.** The gate test inline in the owning crate (one
scanner shared with WI-17/WI-24). **Non-goals / risks.** No
test *rewriting* in this item (consolidation is a separate,
later decision). Risk: giant diffs that hide behaviour
changes — mitigated by the per-module two-step (split, then
move) landing as separate changes, each independently green.

### WI-23 (P1, L): One contract — schema v5, generated D.5, diet for joints [carries old WI-24 / F12; bumps D7]

**Problem.** F12: the report version is still `4` while
WI-17's provenance completion is landing additively; SPEC
D.5 prose has drifted twice from the harness table and will
drift again; joint/partition benches (`45` 37%, `46` 21%,
`47` 31%, `48` 26%) still present barely-winning forms as
token savings; SPEC.md at 9827 lines costs more to read than
most patches save.

**Decision — four contract closings in one bump.** (a)
`REPORT_VERSION` 4→5 in
[lib.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/lib.rs)
(the single constant `applyrs`/`diffrs`/`gitrs` already
share), collecting WI-17's completed provenance, WI-1's
equality fields where reported, and WI-7's guard numbers;
any key whose *meaning* changed is removed, not aliased. The
v5 note documents the already-shipped `verified` tightening.
(b) SPEC D.5's exclusion prose is *generated*: a `tools/`
script reads `CYCLE_OK`/`KNOWN_DIVERGENT` from
[cycle_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/cycle_e2e.rs)
and emits the prose block; a fence-style lint fails when SPEC
and table disagree (the `01`/`21` drift becomes a red test,
not a review finding). The laws stay hand-written; only the
enumeration is generated (WI-24's boundary). (c) Joint diet:
the shared-rule composition states shared header facts once
per rule instead of once per hunk where the parser already
accepts the factored form (canonical-form change in
[canonical.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/canonical.rs),
parser accepts both, diet emits the short form); benches
`45`–`48` re-pin to the diet bytes, and any bench still under
40% saved after dieting retitles its claim from token-saving
to auditability (D10 — the bench table is a contract, not
marketing). (d) SPEC readability: the one-screen agent
contract at SPEC's head remains the only normative quick
reference, now assembled from WI-24's generated tables, and
each body section gains a “read this if…” first line; no
normative text is deleted, only re-fronted.

**Changes.** SPEC D.2/D.5 + head contract. Code: version
constant, canonical diet, generator script in
[tools/](/home/ai/git/tools/rsmpl-git/tools/), lint alongside
[doc_fences_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/tests/doc_fences_e2e.rs)
extended in place.

**Acceptance.** All three binaries report `version: 5`
(dispatcher skew handshake already enforces parity — pinned
by its test); SPEC/table disagreement fails CI; benches
`45`–`48` meet their re-pinned diet targets or carry the
retitled claim; `cargo test --workspace` + clippy + fmt +
benches green.

**Tests.** Version pins updated in place (one constant, three
envelopes); generation lint mutation-checked (edit the table
without regenerating → red). **Non-goals / risks.** No v4
compatibility shim (backwards compatibility is explicitly not
required). Risk: diet canonical changes invalidate
`.history/` byte comparisons — archives stay *readable*
(parser accepts long form, D6) but not byte-comparable across
the bump; stated in the v5 note.

### WI-24 (P2, M): Single-source the agent contract — tables generated, never retyped [carries old WI-25 / F12; enacts D15]

**Problem.** The error table and directive table live in
three documents (SPEC.md §25/§9.3, AGENTS.md,
[SKILL.md](/home/ai/git/tools/rsmpl-git/skills/gitrs/SKILL.md))
plus the code strings themselves. Round 7's WI-1 fix had to
edit the hint text in four places; WI-4/WI-5/WI-8 in this
plan each owe the same multi-document edit, by hand, forever
— the exact process that produced the original hint drift.

**Decision — code strings are the source; documents are
generated views, linted.** (a) Error table: a `tools/`
generator extracts the SPEC 25 table (code → fix) from the
`StructuredError` construction sites (already gate-audited by
round-7 WI-18) plus SPEC's normative annotations, and emits
the AGENTS.md/SKILL.md tables. Hand-edits inside generated
blocks fail the fence lint (blocks carry `<!-- generated: …
-->` markers). (b) Directive table: SPEC 9.3's table is
generated from
[directives.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/directives.rs)
(the parser's own registry — if a directive parses, it is in
the table; if it is in the table, it parses; the generator
proves both directions or fails). The guard classification
table (`guard_for`, WI-7) is generated the same way from its
source. (c) The one-screen agent contract (WI-23d) is
assembled from these generated blocks, so the cheapest
document to read is also the hardest to let drift.

**Changes.** SPEC 9.3/25 + D.4 guard table (generated-block
markers), AGENTS.md + SKILL.md (blocks regenerated). Code:
generator in [tools/](/home/ai/git/tools/rsmpl-git/tools/)
(Python, matching `history_sweep.py` precedent — no new Rust
tooling crate), lint arms in `doc_fences_e2e.rs` extended in
place.

**Acceptance.** Editing a suggestion string in code without
regenerating fails CI naming the stale documents; adding a
directive to the parser without SPEC text fails the
generator; adding a detector to `guard_for` without its SPEC
row fails likewise (all mutation-checked in review);
WI-4/WI-5/WI-17 wording lives in exactly one editable place
each by the time they land (those items land before this one
and update the *source*; this item deletes the copies).

**Tests.** Generator + lint are the tests. **Non-goals /
risks.** No generation of normative *prose* (laws, veto
lists) — only tables whose truth is already in code. Risk:
over-generation makes SPEC unreviewable — mitigated by
generating tables and enumerations only, with prose
hand-owned and cross-linked (the WI-23b boundary).

### WI-25 (P2, M): Learnability gate — every shape has a row, an example, and a fixture [carries D6/D10 surface half]

**Problem.** The surface has grown by accretion: `as` pastes
in four kinds plus an impl form, scoped body pairs, joint
compositions, and per-kind degradation laws each landed with
SPEC text, but AGENTS.md rows and skill examples were updated
“in the same change” by discipline, not by check — and the
discipline already slipped once (the `as` spelling shipped
with no AGENTS.md row in its landing round; fixture numbers
`100` and `101` each name two different diffrs fixtures
today, so “fixture 100” is ambiguous in review prose). An
agent learns RSmPL from AGENTS.md + SPEC + fixtures; any
shape missing from one surface is a shape the agent will
misspell and pay a `fix_pattern` round-trip to discover.

**Decision — a three-surface coverage gate plus corpus
hygiene.** (a) Coverage: a lint (same source-scan family as
WI-17/WI-22/WI-24) enumerates the *shape catalogue* — SPEC
D.4's detector/intent cells and SPEC 20/14.7's authored forms
— and requires for each: one AGENTS.md row (or an explicit
`internal:` exemption for detector-internal names), one
`skills/gitrs/SKILL.md` example or row, and at least one
fixture or bench naming it. Missing surfaces fail CI naming
the shape. (b) Hygiene: duplicate diffrs fixture numbers are
renumbered once (fixtures are directories; the rename is
mechanical, git-preserved, and the cycle/roundtrip harnesses
enumerate directories so nothing else moves), and a lint
pins number uniqueness thereafter. (c) Each authored form's
SPEC fence stays machine-linted as today (the round-6 fence
lint is kept and extended to the new fences WI-13/WI-14 add).

**Changes.** AGENTS.md + SKILL.md (missing rows/examples
added — expected to be few; the gate's first run *is* the
audit and its output list is pasted in the landing change),
fixture renames under
[test-fixtures/diffrs/](/home/ai/git/tools/rsmpl-git/test-fixtures/diffrs/),
lint in `doc_fences_e2e.rs` extended in place.

**Acceptance.** Gate green with zero unexempted shapes; the
four `as` forms + impl form + scoped body pairs each show
their three surfaces; “fixture NNN” is unambiguous in both
corpora; deleting any row makes CI fail naming the shape
(mutation-checked).

**Tests.** The lint is the test. **Non-goals / risks.** No
SPEC prose generation (WI-24's boundary stands), no fixture
*content* changes beyond renames. Risk: catalogue
enumeration rots — the catalogue source is SPEC D.4's own
table (WI-24 generates its guard half; this lint parses the
same table), so a new D.4 cell without surfaces fails
immediately, which is the point.

## Sequencing and gates

- **Per item:** `cargo test --workspace`, `cargo clippy
  --all-targets -- -D warnings`, `cargo fmt --check`, and
  `./example-benches/run.sh` stay green; the landing change
  re-runs its item's named probes as fixtures before claiming
  the acceptance line. No item lands on a red workspace to
  “fix forward”.
- **Per slice within an item:** tree legs before canonical
  legs (a cycle entry moves only when both pass); emission
  changes and their SPEC sentences in the same commit; cycle
  table moves in the same commit as the detector change that
  earns them — never a table-only “fix”.
- **Order rationale (one line):** equality (WI-1) defines
  what “closed” means for every singles item after it;
  headers (WI-5/WI-6) and the guard (WI-7) define what small
  intents may cost before joints (WI-8/WI-9/WI-12/WI-13)
  spend them; anchors (WI-10/WI-11) precede the cross-file
  probes that consume them; provenance (WI-17) precedes the
  v5 freeze (WI-23); generation (WI-24/WI-25) comes last
  because it can only generate a settled contract.
- **Stop rule honoured:** 25 work items. The next review
  audits this plan exactly as §“Carry-forward audit” above
  audited round 7: landed / landed narrowly / partial / not
  landed, with the probe or pin named for each verdict.




