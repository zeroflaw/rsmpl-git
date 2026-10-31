# IMPROVE — RSmPL as a contract that cannot say `verified` and break the build, round 4: rename completeness, frame-aware raising, auditable templates

## Goal

Make RSmPL the contract an agent can trust in both directions without
reconstructing and inspecting every edit by hand:

- **Lowering** (semantic → native, `applyrs` via `gitrs apply`) must never
  report `verified` for a rename whose own definition set it left
  half-renamed. A rename is complete only when every definition the
  language says belongs to it moved, or the report names each leftover
  as structured evidence and the semantics word says so.
- **Raising** (native → semantic, `diffrs` via `gitrs diff`) must compress
  the Rust agents actually write — one-line functions, guards and calls
  inside `fn … { … }` frames, method renames with their call sites —
  and must emit templates a human can audit (`E + 1` → `E + 2`), not
  bare fragments (`1` → `2`) whose scope only a totality proof knows.
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

This review supersedes the previous IMPROVE.md in full. That plan's
WI-1…WI-20 all landed (see the carry-forward audit); this round carries
their residues forward explicitly and adds the fresh gaps this review
probe-verified at HEAD. Work stops at 20 items.

## Success criteria

1. **No `verified`-but-broken rename (WI-1, WI-2, WI-3, WI-8).** A trait
   associated-item rename rewrites its `impl` definitions or fails
   loud; a `let`-binding rename rewrites its block-scoped uses or
   reports them and downgrades; a method rename can follow its call
   sites in one hunk. The probe corpora in each item apply for real
   and then pass `cargo check` without `--verify build` having been
   the only thing standing between the agent and broken code.
2. **Everyday Rust raises (WI-4, WI-5, WI-13, WI-19).** Calls, methods,
   fields, and operator/literal deltas inside one-line `fn` frames
   compress at the same precision as their own-line twins;
   `python3 tools/history_sweep.py` stays at 0 failures and the
   compressing set does not shrink without a named reason in the
   landing change. Emitted general templates carry their anchor frame;
   a bare-literal hunk is emitted only when the literal *is* the frame.
3. **Follow-through is verified, not asserted (WI-6, WI-7, WI-16).**
   Follow-only files earn `instantiated` by ident re-match at each
   rewritten span; moves and insertions earn a structural verdict
   (cut bytes equal paste bytes; inserted item re-parses at its
   anchor). A cross-file rename no longer reports global `partial`
   solely because its uses live in another file.
4. **Cycles (WI-9, WI-10, WI-11, WI-12, WI-14).** The `cycle_e2e`
   harness stays green; the SPEC D.5 exclusion list and the
   `KNOWN_DIVERGENT` table shrink by named families (frame-blind
   singles first, then spelling-only divergences, then the joint
   double-delta) — they never grow silently. Token-tree holes,
   joint intents, and mixed sections re-diff to the shape that was
   applied, or stay pinned with a sharpened owner.
5. **Self-correction (WI-8, WI-17, WI-18).** Every patch/IO/build
   failure carries a machine `repair` or joins a named allow-list in
   the conformance test. `diffrs` output can be verified in one
   `gitrs diff --check` round-trip (real apply + byte-compare, T361
   doctrine: `--check` alone never proves a sweep) instead of a
   hand-rolled two-binary dance.
6. **Contract hygiene (WI-20).** One report version across `applyrs`,
   `diffrs`, and `gitrs` errors; the workspace test gate is green at
   HEAD (it is red today, see F9); every RSmPL fence in SPEC.md,
   AGENTS.md, and `skills/gitrs/SKILL.md` is machine-linted; SPEC
   corpus counts match the fixtures on disk.
7. **Gates.** `cargo test --workspace`, `cargo clippy --all-targets --
   -D warnings`, `cargo fmt --check`, and `./example-benches/run.sh`
   stay green per item; no `.rs` file exceeds 2000 lines (split before
   growing, D5); bench byte targets in each item are met or the item
   does not land.

## Review findings (evidence)

Verified this run at HEAD `b4be17d` (573 commits): **2615 tests listed;
`cargo test --workspace` runs green except one pre-existing failure —
`no_rust_file_exceeds_2000_lines` flags the committed 10000-line scale
fixture (F9); clippy and fmt clean.** Probes below were run with the
debug binaries against throwaway `/tmp` git repos (authored
`applyrs --format json --check`, real applies followed by
`cargo check`, and pending-change `diffrs` conversions); each finding
names the probe so the landing change can re-run it as a fixture. The
architecture is sound and stays:
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
siblings. The gaps are rename completeness, frame blindness in
raising, and evidence that stops one file short — not correctness of
what fires on own-line, single-definition shapes.

### Carry-forward audit of the previous plan

| Old item | Verdict | Evidence / residue |
|---|---|---|
| WI-1 structural postcondition | Landed | String sweep `foo(E)` → `bar(E, "hi")` now reports `postcondition`/`verified` (re-probed); tampered-span veto holds — but follow-only files and moves never reach the probe (F5) and two rename families verify while broken (F1, F2) |
| WI-2 assertable evidence | Landed | Cut bindings carry `binding_meta` digests; `--bindings full` lifts the cut (re-probed, equal `sha256` across sites) — follow-only entries still carry no `sites` and orphans still cap at 8 lines (F8) |
| WI-3 repair completeness | Landed | Range-past-EOF and unknown-directive now carry `fix_range`/`fix_directive` (re-probed) — `--verify build` failures still print `repair: null` (F6) |
| WI-4 pattern-anchored drift | Landed | Stale terse rename 3 lines off applies with `drift: cited [1,1] → actual [4,4]` (re-probed) |
| WI-5 name holes `N` | Landed | `fn N` → `pub fn N` sweep fires (re-probed) |
| WI-6 general-expression raising | Landed with residue | Own-line `v > 0` → `v >= 0` fires; the same delta inside one-line `fn` frames stays ordinary and multi-site `x + 1` emits a bare `- 1` / `+ 2` (F3, F4) |
| WI-7 literal-frame raising | Landed with residue | `println!("a")` → `println!("b")` fires even inline — the literal detector is the one sub-line family that sees through frames, which is why F4's bare form wins the chain |
| WI-8 control + untyped-let templates | Landed | Own-line guards/lets fire; frame-wrapped twins stay ordinary (F3) |
| WI-9 header slot-only raising | Landed | `N`-holed headers fire whole-file |
| WI-10 attribute evolution | Landed | `#[derive(Debug)]` → `#[derive(Debug, Clone)]` raises as an `N`-hole sweep (re-probed) |
| WI-11/12 definition RHS + raising | Landed | `type A = Vec<u8>;` → `Vec<u16>;` authors and single-raises under the precision guard (re-probed) |
| WI-13/26/27/28 associated items | Landed with a correctness hole | Rename authors and `Self::`/`S::` follow fires — but `impl` *definitions* of the renamed item never move (F1) |
| WI-14 anchored member follow | Landed | Field rename with `follow=uses` rewrites the typed use and keeps the sibling as evidence (re-probed) |
| WI-15 cross-file intents | Landed with residue | `struct S` → `T` across two files fires — the foreign file's entry is `reparse-only` and the run is `partial` (F5); plain-`fn` intents still do not raise (F7) |
| WI-16 crate-local `# symbol` | Landed narrow | Call shapes verify `resolved`/`syntax`; general/`let`/type/macro and method-ident symbols stay C14 rejections (F8) |
| WI-17 macro raising depth | Landed narrow | Token-tree holes cycle multi-site; a literal edit inside a `macro_rules!` body still re-diffs ordinary (F8) |
| WI-18 collection moves | Landed | Moves apply — and stay `reparse-only` forever (F5) |
| WI-19 precision-first cycles | Landed partial | Divergent table at ~68 entries, 56 still tagged `t375` (counted in [cycle_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/cycle_e2e.rs)); spelling-only divergences (`01`, `90`, `51`) remain (F7) |
| WI-20 unified schema v4 | Landed | `REPORT_VERSION = 4` in [lib.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/lib.rs:36), shared by all three binaries (re-probed side by side) — the gate it added is red at HEAD (F9) |

Fixture counts on disk today: 121 applyrs fixtures, 116 diffrs
fixtures, 32 applyrs benches, 42 diffrs benches. SPEC D.5 still cites
“41 / 60” (F9).

### Findings

- **F1. Associated-item follow renames the trait and the uses, but not
  the implementations — and reports `verified`.** The follow walker in
  [use_graph_assoc.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_assoc.rs)
  (`AssocWalker::classify_path`) claims only `X::Old` path tails;
  `ImplItem::Type`/`Const` definition idents are consulted solely as a
  shadow census, never claimed. Probe-verified: renaming trait
  `type Item` → `Output` with `# follow=uses` rewrites the trait decl
  and `Self::Item` uses, leaves `impl Tr for S { type Item = u32; … }`
  untouched, reports `semantics: verified` with no orphans — then a
  real apply fails `cargo check` with E0437/E0046. The definition-only
  spelling is equally silent: `assoc_orphans` shares the walker, so
  the impl definitions appear in no evidence list. `--verify build`
  catches it (E0437 surfaces), which proves the default verdict is
  the one lying.
- **F2. `let`-binding renames verify while breaking their block.**
  Orphan collection in [plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
  dispatches only for `Fn`/`ItemRename`/`FieldRename`/`VariantRename`/
  `AssocRename`; a general `let` hunk gets no orphan scan at all.
  Probe-verified: `let total = 1;` → `let sum = 1;` with `total + total`
  below reports `instantiated`/`verified`, no orphans, and the applied
  tree fails `cargo check` (E0425). The pattern is doing exactly what
  it was authored to do — the contract failure is that nothing in the
  report says the binding's uses stayed behind. The same blind spot
  covers closure-parameter and match-arm binding renames spelled as
  general patterns.
- **F3. Raising is blind inside `fn … { … }` frames — the shape of
  compact Rust.** Whole-line detectors require a changed line to parse
  as exactly one expression; the sub-line fallback
  ([subline.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/subline.rs),
  `side_expr`) parses the line as an `Expr` or a `let` statement, so a
  line like `fn f0() { foo(0); }` parses as neither and every family
  declines. Probe-verified taxonomy (3 sites each): own-line
  `foo(E0)` sweeps fire; the identical calls inside one-line fns,
  inside `fn f() { let a = foo(1); … }`, in tail position
  (`fn f() -> u32 { foo(1) }`), method calls `a.old()` in one-line
  fns, field accesses, and operator deltas `v > 0` → `v >= 0` in
  one-line fns all stay ordinary. Only the literal detector (F4)
  sees through the frame. Real codebases — and this repo's own
  one-liner tests — are dense with the blind shape.
- **F4. The cheapest emission wins the chain, and it is unauditable.**
  Three sites of `x + 1` → `x + 2` (distinct receivers `x`/`y`/`z`)
  raise as `@@ -1,3 @@` + `# allow=many` + `- 1` / `+ 2`. The form is
  sound — pre-image totality plus the engine probe prove the site
  set, and re-applying it reproduces the post-tree — but neither a
  human nor an agent can see *which* `1`s it owns without re-running
  the proof. The anchored form the engine equally accepts
  (`- E + 1` / `+ E + 2` under `# sweep`, probe-verified) costs a few
  bytes and states its scope. SPEC 2.6's “never at the cost of a
  guess” is met; the round-3 addition — never at the cost of
  auditability — is not. Chain order, not precision, picks the winner
  today.
- **F5. Follow uses in other files, moves, and insertions can never
  be verified.** [plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
  hard-codes follow-only entries to `ReparseOnly` (“per-use plus
  shapes differ”) and [postcheck.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck.rs)
  excludes `Insertion`/`ItemCapture`/`Raw` from `is_rewrite_kind`.
  Probe-verified: a two-file `struct S` → `T` follow reports the
  definition file `instantiated`, the use file `reparse-only`, global
  `partial`; a textbook cut/paste move reports both rules
  `reparse-only`, global `partial`. The two operations an agent most
  needs to trust — “rename everywhere” and “move this item” — are
  permanently second-class. The per-use plus shape is not exotic: it
  is the new ident at a recorded span, checkable by the same syn
  ident equality WI-1.4 already uses for renames.
- **F6. `--verify build` is the only build truth, and it fails
  without a next action.** Under `--check` it verifies post-images in
  a temp copy; after a real apply it runs `cargo check` in place; on
  failure both paths emit `RSMPL_IO_CONFLICT` with `repair: null`
  (probe-verified on the F1 corpus), joining no allow-list the
  conformance test names. Worse, the flag is opt-in, so the F1/F2
  failures ship as plain `verified` unless the agent already suspects
  them. Trust that requires the caller to ask for it twice is not a
  contract; the semantics word must degrade on its own evidence
  (incomplete follow domains, unscoped binding renames) before any
  compiler runs.
- **F7. The single-site mass and the spelling mass are different
  problems sharing one table.** Of ~68 `KNOWN_DIVERGENT` entries, 56
  carry `t375` (single-site sweep shapes demoted to ordinary — sound,
  token-wasteful), while `01-rename-fn` (terse authored vs sweep
  re-inferred), `90-macro-rules-nofollow` (`allow=many` spelling vs
  directive-free), and `51-drift-stale-range` (cited vs actual range)
  diverge on *spelling over the same sites*. The first family needs
  precision decisions per shape; the second needs the canonical form
  in [canonical.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/canonical.rs)
  to compare site sets, not spellings. One WI cannot fix both, and
  shrinking either by loosening the other reintroduces guessing.
- **F8. Everyday intents still fragment.** (a) Methods: renaming an
  impl method authors only through the `impl S {` context form, and
  `# follow=uses` on that hunk is C17-rejected (probe-verified), so
  call sites need a second, unanchored `- E.run()` sweep that matches
  every receiver in the file; trait-method renames likewise stop at
  the trait/impl pair and re-diff as three ordinary hunks. (b) Plain
  fns: `fn foo` + two call sites re-diffs fully ordinary (probe-
  verified) — `item_intent` emits `# follow=uses` for items but “no
  follow emission for plain fn” (`46-fn-follow`). (c) Joint renames:
  `trait Printable` + `struct Old` + `impl Printable for Old` →
  `Labeled`/`New` stays one ordinary hunk (the `14-item-rename`
  double-delta). (d) Macros: `inner!(1)` → `inner!(2)` inside a
  `macro_rules!` body stays ordinary. (e) `# symbol` remains
  call-only (C14); a method-ident symbol is rejected by design.
  Each fragment forces the agent back to hand-inspecting edits —
  the exact cost RSmPL exists to remove.
- **F9. The hygiene gate is red and the map is stale.** At HEAD,
  `cargo test --workspace` fails `no_rust_file_exceeds_2000_lines`
  because the WI-20 scale fixture
  `example-benches/applyrs/44-scale-postcheck-deep/repo/src/lib.rs`
  is a committed 10000-line `.rs` file the gate counts. SPEC D.5
  cites 41/60 corpus fixtures against 121/116 on disk. Five engine
  files sit within 110 lines of the 2000-line cap
  (`matcher_fn.rs` 1946, `plan.rs` 1916, `postcheck.rs` 1889,
  `semantic/mod.rs` 1845, `use_graph.rs` 1827), so the next features
  in this plan cannot land without splits. A red gate trains agents
  to ignore gates.
- **F10. The raising loop has no single verify command.** `diffrs`
  accepts only git state (`[rev]`, `--staged`, `--repo`); an agent
  holding a native patch *text* (from `git diff`, a PR, another
  tool) cannot raise it without manufacturing a repo state, and
  checking a raised patch still takes the manual
  `diffrs | applyrs --check` pipe — which T361 already proved cannot
  detect sweep over-match. The trustworthy check (real apply +
  byte-compare) exists only inside test harnesses and
  `tools/history_sweep.py`. The contract's second direction has no
  equivalent of `gitrs apply --check`.

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
  the plus pattern, per-site bindings, orphan enumeration). If a shape
  cannot be re-matched, it is labelled `reparse-only` — never upgraded by
  finding its text somewhere in the file. Round 4 extends D2: a verdict
  computed over an *incomplete domain* (a follow that skipped secondary
  definitions, a binding rename that never looked at its uses) is also
  true-by-construction and must degrade the same way.
- **D3. Inference stays precision-first.** Every new detector keeps
  pre-image totality (count must equal pair count, zero opaque hits) and
  the T375 byte-win guard measured in diet bytes, except the
  precision guard for definition shapes (WI-19 of round 3, generalised
  in WI-9 here), which is itself bounded and pinned. A detector that
  cannot prove totality stays ordinary — no guessing (SPEC 2.6).
- **D4. One anti-unification core, thin detectors.** New raising work
  reuses and extends the existing
  [antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
  core and the sub-line extractors instead of growing per-shape string
  surgery. Engine work is structural `syn` matching — concrete,
  fail-loud, no expansion, no guessing.
- **D5. Split before growing.** New code lands in new modules; no item
  pushes a file over 2000 lines, and any file over 1600 lines touched by
  an item leaves under 1600 or with its new code in a sibling module.
  The watch list is F9's five files plus
  [chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs)
  (1557) and [main.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs) (1599).
- **D6. Familiar syntax wins over clever syntax.** New surface reuses
  Git/SmPL/Rust spellings (`...`, `#` directives, concrete paths, context
  headers that look like the block they select, attr lines written where
  Rust writes them). Diet remains canonical; the parser keeps accepting
  long forms so `.history/` archives stay readable. Any new spelling must
  be explainable in one AGENTS.md row. Round 4 adds no new directive
  keywords: method follow reuses `# follow=uses`; frame raising changes
  no spelling at all.
- **D7. Breaking changes are taken where they buy trust.** JSON schema
  stays at its current version for additive keys; a key whose *meaning*
  changes bumps the version with the old key removed, per SPEC D.2's own
  rule. SPEC text moves with code in the same item. This round bumps the
  unified report to `version: 5` exactly once (WI-20) after WI-6/WI-7/WI-8
  have settled the evidence shapes it carries.
- **D8. Anchors before indirection.** Follow, symbol resolution, and
  member anchoring prove site ownership from text the change already
  contains — fn-signature param types, `let` annotations, struct-literal
  paths, `use` chains, the enclosing `impl`/`trait` header — before
  consulting any wider graph. A site no anchor proves is evidence
  (`orphans`/`follow_skipped` with a reason code), never a rewrite and
  never a veto on the proven sites.
- **D9. A rename owns its secondary definitions.** For every rename kind,
  SPEC enumerates the *definition set* (trait decl + impl definitions
  for associated items; declaration + impl methods for trait methods;
  the binding + its block uses for locals) and the follow/orphan
  machinery must account for every member: rewritten, skipped with a
  reason, or the run degrades. “Use sites” never silently excludes
  definitions again.
- **D10. Auditability outranks byte-minimality.** When two emissions
  prove the same site set, `diffrs` emits the one whose pattern states
  its frame (holes + anchors), even at a small byte cost. Bare-fragment
  emissions survive only where the fragment is the whole matched unit.
  Token targets in benches are re-pinned to the auditable form, never
  met by stripping anchors.

## Work plan

Order is dependency order: WI-1 → WI-2 → WI-3 → WI-6 → WI-7 → WI-8 →
WI-4 → WI-5 → WI-9 → WI-10 → WI-11 → WI-12 → WI-13 → WI-14 → WI-15 →
WI-16 → WI-17 → WI-18 → WI-19 → WI-20. The rename-completeness fixes
(WI-1–WI-3) precede the verification lifts (WI-6–WI-8) that must
certify them; frame raising (WI-4) precedes the anchored-emission and
single-site work (WI-5, WI-9) that spend its site sets; WI-20 last
because it bumps the unified schema once. Sizes are relative
(S ≤ 1 day, M ≈ 2–3 days, L ≈ 4–5 days of focused work).

### WI-1 (P0, M): Associated-item follow renames its `impl` definitions [carries F1; completes D9 for assoc kinds]

**Problem.** F1/D2/D9: renaming trait `type Item`/`const LIMIT` with
`# follow=uses` leaves every `impl Trait for S { type Item = …; }`
definition behind, reports `verified`, and breaks the build. The
orphan path shares the blind spot, so definition-only renames warn
about `Self::` uses but never about the impl definitions themselves.

**Decision — definitions are follow sites.** The assoc domain gains a
definition set: for a trait-context rename, every `impl <Trait> for
<Self>` block whose trait tail names the renamed trait contributes
its same-kind, same-name `ImplItem` definition ident as a *definition
claim* (rewritten, labelled `rename` in `span_edits`, counted in the
definition entry's spans, never in `matches`). Inherent impls
(`impl S { type Item … }` with no trait) never claim — they are a
different item and become `shadowed` evidence when a same-name tail
site depends on the distinction. The census in
[use_graph_assoc.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_assoc.rs)
already collects these idents for shadowing; the walker additionally
records their spans. `collect_assoc_orphans` claims the same set, so
definition-only renames list leftover impl definitions as orphans
(`off-target`, text = the definition line). A workspace whose impl
header cannot be proven to name the trait (glob, second-hop) skips
with the existing reason codes — never claims, never vetoes the
trait-side rename. Postcondition: definition claims verify by the
WI-1.4 syn ident-equality probe like any rename span.

**Changes.** SPEC 14.8 assoc-follow paragraph (definition set,
inherent-impl exclusion), D.5 (no behaviour change). Code:
definition-claim recording in `AssocWalker`/`AssocCensus`
([use_graph_assoc.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_assoc.rs));
orphan parity in `assoc_orphans`
([plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)).
The smallest focused regression test ships with the fix: a trait +
two impls (one trait-proven, one inherent namesake) fixture under
`bin/applyrs/tests/` asserting the trait-proven impl definition
rewrites, the inherent one is untouched `shadowed` evidence, and a
real apply + `cargo check` passes — kept as the durable collateral
for this bug class.

**Acceptance.** The F1 probe (type and const variants, single- and
multi-line spellings) now rewrites impl definitions and passes
`cargo check` after a real apply with no `--verify` flag; the
inherent-namesake probe leaves that impl untouched and lists it;
definition-only probes list impl definitions in `orphans`.

**Tests.** Assoc follow matrix (trait-proven / inherent / glob-skip
× type / const); orphan parity pins; F1 fixture end-to-end with
build check; full workspace + cycle harness.

**Non-goals / risks.** No trait solving: “names the trait” stays the
syntactic last-segment + qself discipline of WI-27. Risk: an impl
through a renamed re-export now claims — bounded by the existing
one-hop rule (D8), pinned by a re-export fixture.

### WI-2 (P0, M): Binding-rename honesty — `let` uses are in scope or in evidence [carries F2; completes D9 for locals]

**Problem.** F2: a `let` (or closure-param / match-arm) binding rename
rewrites the binder and nothing else, with no orphan scan, so the
common “rename this local” edit verifies while breaking its block.

**Decision — block-scoped completion, else evidence + downgrade.**
When a general `let`-statement hunk (or a structural pattern hunk)
renames exactly one binder `old` → `new` and nothing else in the
pattern changes, the engine computes the binder's block scope from
the matched statement's enclosing block (the `syn` block containing
the `let`, bounded by shadowing: a nested `let`/param/pattern
rebinding `old` ends the scope). Same-scope, unshadowed value uses
of `old` rewrite in the same hunk (labelled `follow`, definition
entry spans, `matches` still counts binders); uses past a shadowing
boundary and uses in macro interiors are not rewritten. Every
renamed binder with at least one unrewritten same-name use in its
file — shadowed, macro-hidden, or outside the enclosing block —
reports those uses as `orphans` (`shadowed`/`macro-hidden`/
`off-target`) and the rule's semantics caps at `instantiated` only
when zero such uses exist; otherwise it reports `reparse-only` and
the run is `partial`, unless the hunk carries `# follow=uses`…
no — locals take no new directive (D6): block completion is part of
the `let`-rename shape itself, documented as such, and a caller who
wants binder-only renames writes the pair under an ordinary hunk.
Closure-param and match-arm binders get orphan evidence + the same
downgrade in this item; block completion for them is WI-19 scope.

**Changes.** SPEC 18.1/`let` rows (binder-rename completion +
shadowing bound), 24.6 (downgrade rule), D.2 (orphan reasons reused,
no schema change). Code: scope walker + orphan dispatch for
`PatternKind::General` let shapes in a new
`crates/rsmpl_engine/src/use_graph_local.rs` (D5: `plan.rs` is at
1916 lines); matcher reports binder identity to the planner.

**Acceptance.** The F2 probe rewrites both `total` uses (or, with a
shadowing inner `let total`, rewrites only the outer uses, lists the
inner ones, and reports `partial`); real apply + `cargo check`
passes on the completion probes; a binder with an escaped use
(closure capturing past the block — still same block, still claimed;
a use in a nested fn item — never claimed) pins evidence, not a
rewrite.

**Tests.** Scope matrix (plain / shadowed / nested-block / macro /
nested-fn); downgrade pins (`verified` only at zero leftovers);
durable fixture mirroring WI-1's (apply + `cargo check`).

**Non-goals / risks.** No dataflow or borrow analysis — scope is the
syntactic block + shadowing discipline only (D8). Risk: a `let`
rename inside a macro-expanded-looking block mis-scopes — macro
interiors stay opaque and force the downgrade, never a claim.

### WI-3 (P0, L): Method follow — `impl`-context and trait-method renames follow their call sites [carries F8(a); spends D9]

**Problem.** F8(a): the only authored method rename is the
`impl S {` context pair, and `# follow=uses` on it is C17-rejected;
trait-method renames have no follow at all. The fallback — a bare
`- E.run()` sweep — matches every `run` receiver in the file,
including other types', which is precisely the guess D8 forbids.

**Decision — two new follow domains, one spelling.** `# follow=uses`
becomes legal on (1) `ImplContext` fn renames and (2) trait-context
fn renames (the `trait Tr {` header + fn pair). Domain (1) claims
method-call sites whose receiver type is proven to be the impl's
self type by the WI-14 anchor order (fn-param `s: S` / `s: &S`,
`let x: S`, `let x = S { … }`, `self` inside `impl S`, `S::…`
associated-call tails); domain (2) additionally requires the impl
side to name the trait (WI-1's trait-tail discipline) and claims
both the impl method definitions (D9 definition set) and anchored
call sites. Unproven receivers become `follow_skipped`
(`off-target`); a second same-name method on another proven type in
scope becomes `shadowed` — neither vetoes. C17's legal list gains
exactly these two shapes; evolution and all other shapes keep the
rejection. Raising (`trait_method_rename`) emits the follow form
when its scratch-apply probe proves the call-site set, else today's
paired definition hunks.

**Changes.** SPEC 14.8 (method/trait-method follow paragraphs, C17
list), 18.6 C17 row, D.4 (emission rule). Code: method domain in a
new `crates/rsmpl_engine/src/use_graph_method.rs`; anchor reuse from
`use_graph_anchor.rs`; C17 gate widening in
[pattern_classify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/pattern_classify.rs);
detector emission in
[trait_methods.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/trait_methods.rs).

**Acceptance.** Probes: inherent method renamed across param-typed
and `self` receivers while a same-name method on another struct is
untouched evidence; trait method rename moves trait decl + impl
defs + anchored calls in one hunk and passes `cargo check` after
real apply; the F8 trait re-diff compresses to the follow form.

**Tests.** Anchor matrix per domain; shadowed/off-target pins;
C17 accept/reject pins (evolution still rejected); durable
apply+build fixture; cycle flips for the trait fixtures named in
the landing change.

**Non-goals / risks.** No dynamic-dispatch proof: `dyn Tr` receivers
prove through their declared type only. Risk: receiver anchors
through generic params (`s: impl Tr`) — claimed only when the bound
names the trait syntactically, else evidence.

### WI-4 (P0, L): Frame-aware raising — detectors see inside `fn` items [carries F3]

**Problem.** F3: any changed line that also carries `fn`, braces, or
a second statement is invisible to raising, so compact Rust — the
default style of tests, closures-heavy code, and this repo's own
fixtures — stays ordinary no matter how uniform the rewrite is.

**Decision — frame extraction before line classification.** A new
pre-pass in [chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs)
splits each balanced minus/plus line pair that fails whole-line
classification into (frame, sites): parse the line as a `syn::Item`
or statement sequence; when minus and plus frames are token-equal
outside a set of same-family subterm positions (the WI-4 sub-line
families plus binary/unary/field/index general-expression subterms),
the pair contributes those subterms as sites and the frame joins the
totality veto (any frame token delta, or a same-line site left
unchanged, vetoes the pair). Extraction reuses the existing family
collectors in [subline.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/subline.rs);
only the *line splitter* is new (in a sibling `frame.rs`, D5).
Emission is unchanged — the bare subterm hunk over the proven site
set — except as WI-5 reorders template preference. Totality counts
subterms in the pre-image exactly as WI-4 does today; frames add no
new trust, only new sight.

**Changes.** SPEC D.4 sub-line paragraph (frame rule), no authoring
change (engine matching already handles these sites — probe: the
emitted hunks re-apply today). Code: `frame.rs` splitter;
`side_expr` gains an item/statement-sequence entry; chain wiring
behind the existing “whole-line failed uniformly” gate so no
own-line class changes winner.

**Acceptance.** Every F3 probe pair compresses (call, let-inline,
tail-call, method, field, operator-delta) and each emitted patch
real-applies byte-identical to the true post-tree (T361 compare,
not `--check` alone); a frame with a header delta (`fn f0` →
`fn g0` alongside the call rewrite) stays ordinary; benches gain a
compact-style diffrs bench (43-series) at the byte target set in
the landing change.

**Tests.** Frame matrix (fn-brace / let-sequence / tail / guard /
distractor-twin veto / frame-delta veto); totality twins inside
frames veto; history sweep re-run with no shrink.

**Non-goals / risks.** Multi-line items whose changed lines are
proper own-line statements already work and keep their winners —
frames only claim lines nothing else parsed. Risk: over-splitting a
line holding two different families — mixed-family lines veto
(single-family rule, as today).

### WI-5 (P1, M): Anchored templates beat bare fragments [carries F4; enacts D10]

**Problem.** F4/D10: chain order lets the literal detector emit
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

**Acceptance.** The F4 corpus emits `- E0 + 1` / `+ E0 + 2`
(diet holes, SPEC 15.8) and real-applies byte-identical; whole-line
literal fixtures (`17-same-literal` family) still emit the bare
pair; no fixture's site set changes — only spellings, each listed.

**Tests.** Preference pins per overlapping class; site-set equality
assertions (old vs new emission match identical pre-image spans);
bench table re-pin with byte deltas named.

**Non-goals / risks.** No change to authored-pattern matching: a
hand-written `- 1` hunk still applies exactly as today. Risk:
anchored templates cost more on huge literal sweeps — bounded by
the ≥3-site bench (log-message bench 36) holding its target within
the re-pinned tolerance.

### WI-6 (P1, M): Follow uses verify at their spans [carries F5-follow; hardens WI-1/WI-15 residues]

**Problem.** F5: follow-only file entries are hard-coded
`reparse-only`, so every cross-file rename reports global `partial`
and the agent learns to ignore the one word that matters.

**Decision — ident-equality probe for use spans.** Follow edits are
single-ident replacements at recorded spans by construction (the
walker claims ident spans, never ranges). After materialisation,
each follow span in every file verifies iff the post-image tokens
at that span are exactly the new ident (syn token comparison via
the file's `LineIndex`, the WI-1.4 discipline — occurrence elsewhere
never counts). A follow-only entry whose spans all verify earns
`instantiated`; any failure fails the run loud with the existing
instantiation `RSMPL_MATCH_CONFLICT` (the edit did not land as
recorded — a planner bug, not a user error, and it must not ship
as a downgrade). Definition entries keep their existing probe;
their follow spans in the *same* file verify under the same rule,
so `span_edits: […, follow]` entries are all certified, ending the
“definition probe covers the rename” approximation. Struct-literal
shorthand expansions (`S { old }` → `S { new: old }`) verify by
their two-token post shape, enumerated in SPEC 24.6.

**Changes.** SPEC 24.6 (follow-span probe), D.2 (semantics
derivation unchanged — more entries earn `instantiated`). Code:
probe extension in [postcheck.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck.rs)
(new sibling `postcheck_follow.rs` if the file would cross 1600
effective growth, D5); the hard-coded `ReparseOnly` in
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
is replaced by probe-pending follow entries.

**Acceptance.** The F5 two-file probe reports both files
`instantiated`, global `verified`; a hand-corrupted post-image
(follow span reverted) fails loud in tests; same-file follow spans
verify identically.

**Tests.** Follow probe matrix (plain tail / shorthand expansion /
cross-file / tampered span); semantics-word pins for benches 40–42.

**Non-goals / risks.** Bindings for follow sites stay absent (they
bind nothing — WI-18 adds span-level provenance only). Risk: span
translation after line-changing definition edits — reuse
`translate_spans_to_post`, pinned by a line-growing rename fixture.

### WI-7 (P1, M): Moves and insertions earn a structural verdict [carries F5-moves]

**Problem.** F5: cut/paste moves and pure insertions are excluded
from `is_rewrite_kind`, so the move operation — half the product's
name — reports `reparse-only`/`partial` on every correct use.

**Decision — prove what a move promises.** (1) A cut+paste pair
verifies as `postcondition` iff the pasted post-image item text is
byte-identical (after the WI-18 name-evolution template, where
applicable) to the cut pre-image capture digest already recorded
in `binding_meta`, and the cut span in the post-image no longer
contains the item. Name-evolving pastes verify per item by
re-matching the paste template with the evolved name. (2) A pure
insertion verifies iff the inserted lines re-parse as the authored
item/expression at the reported anchor span in the post-image
`syn::File` (the insertion matcher run in reverse: parse the
inserted slice standalone, compare token streams). (3) Unpaired
cuts (deletions) verify iff the span is absent and the file
re-parses — labelled `postcondition`, honestly weaker than a move
and documented as such. Anything failing these checks fails loud;
shapes the checker cannot parse stay `reparse-only`.

**Changes.** SPEC 24.6 (move/insertion probes), D.2 (word meanings
unchanged). Code: `postcheck_move.rs` sibling beside
[postcheck.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck.rs);
capture digests threaded from
[capture.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/capture.rs).

**Acceptance.** The F5 move probe reports `postcondition` on both
rules and global `verified`; a paste with one byte altered in
transit (planner-bug injection test) fails loud; collection moves
(bench 31) and evolving pastes (WI-18 of round 3) verify per item.

**Tests.** Move matrix (single / collection / evolving-name /
cross-file / unpaired cut / insertion); tamper pins; bench 02/31
semantics words pinned.

**Non-goals / risks.** No semantic equivalence beyond byte/template
equality — a move that also edits the body is two hunks and verifies
as such. Risk: digest plumbing for large captures — digests already
exist for cut values (WI-2); no new bytes on the wire by default.

### WI-8 (P1, M): Build truth without begging for it — degraded verdicts and repairable build failures [carries F6, F1/F2 backstop]

**Problem.** F6: `verified` currently means “patterns re-matched”,
full stop. F1/F2 showed renames can satisfy that and still break
the build; the only net is an opt-in flag whose failure carries
`repair: null`.

**Decision — two independent fixes, no default build.** (1) The
semantics derivation learns *domain completeness*: a rule whose
follow/orphan domain was truncated (skip reasons present, orphan
list at cap, unsearchable workspace file, shadowed definitions
left behind) cannot report the run `verified` on its own — the
global word is `partial` and the rule carries its evidence as
today. This is computed from evidence the engine already collects;
no compiler runs. (2) `--verify build` failures gain a machine
repair: `{"action": "read_file", "file": <first diagnostic path>,
"range": "<line>..<line>"}` parsed from the first `cargo check`
diagnostic (fallback: the orphan hint's first entry), and the
error code for build failures becomes `RSMPL_BUILD_CONFLICT`
(new code, SPEC 25) instead of overloading `RSMPL_IO_CONFLICT` —
the one named exception being a missing `Cargo.toml`, which keeps
`RSMPL_IO_CONFLICT` + the environmental allow-list entry (WI-20's
conformance test names both). `--check` temp-copy and post-apply
paths emit the identical shape. The flag itself stays opt-in (D:
build cost discipline from round 3 stands); what changes is that
*not* passing it no longer manufactures confidence — incomplete
domains already say `partial`.

**Changes.** SPEC 24.6/D.2 (completeness rule), 25 (new code +
repair shape). Code: derivation in
[main.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
(`global_semantics` learns completeness inputs from the report);
diagnostic parser beside the existing verify helpers in the same
file (extract to `verify_report.rs` if it crosses the D5 watch
line).

**Acceptance.** F1/F2 corpora without `--verify` now report
`partial` with visible skip/orphan evidence (post-WI-1/WI-2 they
report `verified` because the domains complete — both directions
pinned); with `--verify build`, a deliberately broken rename
fails with `RSMPL_BUILD_CONFLICT` + a `read_file` repair naming
the diagnostic site; missing-`Cargo.toml` keeps its allow-listed
shape.

**Tests.** Completeness matrix (clean / skipped / capped /
unsearchable); build-repair pins for check-mode and apply-mode;
conformance enumeration updated (WI-20).

**Non-goals / risks.** No always-on `cargo check`, no test/clippy
verification modes. Risk: over-degrading honest partial work —
the rule fires only on *rename-domain* truncation, never on
ordinary-hunk presence (already `partial` by design).

### WI-9 (P1, L): Precision singles II — safe shapes fire at N=1 [carries F7-precision; generalises the WI-19 guard]

**Problem.** F7: 56 `t375` entries demote single-site sweeps whose
only sin is having one site. Round 3's precision guard proved the
pattern for definition shapes (a single pair whose template cannot
match wider than the diff because its anchors are the whole unit);
call/method/general singles were left under the byte guard because
a bare `foo(E)` *can* match siblings the diff never touched.

**Decision — uniqueness-proved singles.** A single-site class in
the call, method-call, general-expression, and let families fires
under a new `PrecisionUnique` guard iff (a) the emitted template,
run through the real engine probe against the pre-image, matches
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
anchorless vetoes); cycle harness re-pin; history sweep (new
compression reported, never asserted).

**Non-goals / risks.** No N=1 for verbatim/macro/token-tree shapes
(WI-14 owns macro singles) and none for hetero classes. Risk:
whole-file probes cost time on large files — bounded by the WI-20
scale gate; the probe parses once per file (existing discipline).

### WI-10 (P1, M): Site-set canonical equality — spelling stops diverging [carries F7-spelling]

**Problem.** F7: `01` (terse vs sweep), `90` (`allow=many` vs
directive-free), and `51` (stale cited range vs actual range)
diverge although pre/post trees and matched site sets are
identical. The canonicaliser compares spellings; the contract
cares about sites.

**Decision — canonicalise by proven site set.** `canonicalize_with_tree`
already re-emits `allow == many` hunks by matched spans; extend the
same treatment to (a) terse/directive-free rename hunks whose single
matched span set equals a sweep's (emit the diet sweep form — the
form `diffrs` emits — regardless of authored spelling), and (b)
ranged hunks whose proven spans differ from their cited range
(emit the actual-span range under the existing 64-line rule).
Two hunks compare equal iff their site sets, templates (post-diet),
and file sets are equal. Pure `canonicalize` (no tree) keeps
preserving authored locality — only the tree-aware comparator,
which already promises this normalisation, learns the two cases.
SPEC D.5 states the site-set definition explicitly and removes
`01`/`90`/`51` from the exclusion prose.

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
staleness); negative pin: different site sets never unify.

**Non-goals / risks.** No change to application semantics — ranges
still cite original coordinates at apply time (SPEC 10); only the
*comparison* normalises. Risk: unifying a hunk whose range was
load-bearing for disambiguation — the tree-aware path only fires
when the engine proves a unique site set, else authored form
stands (existing fallback).

### WI-11 (P2, M): Joint double-delta intent — `impl Trait for Type` renames as one [carries F8(c)]

**Problem.** F8(c): `impl Printable for Old` → `impl Labeled for
New` changes two names in one intent; every single-delta detector
declines and partition atomicity keeps the whole section ordinary
(`14-item-rename` residue: 8 semantic + 1 ordinary is the ceiling
today).

**Decision — one joint detector, two renames, one hunk pair.**
A new `impl_joint` detector claims a minus/plus line pair (or
impl-header context pair) iff it parses as `impl <Tr> for <Ty>`
on both sides with exactly those two segment deltas and nothing
else, *and* the section (or, cross-file, the workspace probe)
also contains the corresponding `trait Tr` → `trait Tr2` and/or
`struct/enum Ty` → `Ty2` definition pairs. Emission is the grouped
form agents already know: one definition hunk per renamed item
with `# follow=uses` (round-3 `item_intent` grouping), where the
impl header line itself is carried as a follow site of *both*
renames (its two segments labelled accordingly) instead of a
third hunk. If either definition pair is absent, the impl pair
stays ordinary (an impl-only retarget is a different, rarer edit
and keeps today's spelling). Authoring needs no new syntax: two
ordinary rename hunks with `# follow=uses` already produce this
lowering; WI-11 is raising + the SPEC statement that the impl
header is a shared follow site.

**Changes.** SPEC D.4 (`impl_joint`), 14.8 (shared follow site).
Code: detector in
[impl_single.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/impl_single.rs)
sibling `impl_joint.rs`; follow-site labelling reuses WI-6's
span probe (two labels, one span — `span_edits` gains the pair
form, additive).

**Acceptance.** The `14` corpus re-diffs to 9 semantic + 0 ordinary
(or the landing change names the surviving ordinary line and
why); the three-line probe (`trait`+`struct`+`impl`) compresses
and real-applies byte-identical; impl-only retarget stays
ordinary (pin).

**Tests.** Joint matrix (both defs / one def / neither / generic
impl veto); cycle flip for `14`; partition interaction pins.

**Non-goals / risks.** No general multi-delta anti-unification —
exactly two named segments, exactly the impl-header shape.
Risk: an impl pair whose trait/type pairs live in other files —
the cross-file probe (WI-15 machinery) decides; unproven stays
ordinary.

### WI-12 (P2, L): Partition degrades per-pair, mixed hunks split at detector boundaries [carries F8-hetero residue; revisits the WI-19c non-goal]

**Problem.** Round 3 declared “a single mixed native hunk never
splits” a permanent non-goal (`12` header+let). The cost is real:
one unclaimable pair in a native hunk keeps every claimable pair
beside it ordinary, because `git diff -U0` merges adjacent changed
lines into one hunk regardless of shape boundaries.

**Decision — split by lines, never by guessing.** `compress_section_per_hunk`
learns a line-level partition: within one balanced native hunk,
maximal runs of consecutive minus/plus line pairs sharing one
detector verdict (whole-line detectors only — no sub-line, no
frame extraction inside a split) emit as semantic hunks with
tight sub-ranges; runs with no verdict stay ordinary *in place*,
preserving original line order and SPEC 10.6 disjointness by
construction. The `12` class (fn-header pair + `let` pairs in one
hunk) therefore compresses its let run and keeps the header pair
ordinary until a header detector claims it. Atomicity is kept
where it matters: a single *pair* never splits (WI-19c's actual
invariant — one line pair, one verdict), and any run whose
totality depends on section-wide counting stays whole-section
or ordinary. SPEC D.4/D.5 rewrite the “permanent non-goal” prose
to state this boundary.

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
alternating verdicts emit alternating hunks in order); overlap
freedom property (emitted ranges pairwise disjoint) over the
fixture corpus.

**Non-goals / risks.** No splitting inside a line pair and no
joint header+body template (still future work, now honestly
scoped as such instead of “permanent”). Risk: hunk-count growth
hurting token targets — measured per bench; a split that does
not beat its ordinary bytes under T375 stays ordinary.

### WI-13 (P1, M): Plain-`fn` intents raise with follow [carries F8(b)]

**Problem.** F8(b): `struct` renames raise as one `# follow=uses`
hunk; the identical `fn` rename + call sites re-diffs as three
ordinary lines (or, post-WI-4, as a definition hunk plus an
unrelated call sweep that does not state they are one intent).

**Decision — `item_intent` learns the fn case.** When a section's
changed lines contain exactly one `fn` definition pair
(`fn old…` → `fn new…`, same slots) plus call/method sites of
`old` that the follow domain (WI-3 anchors + bare-call sites)
proves belong to it, and the scratch-apply probe (the existing
T361 real-apply byte-compare over the inferred follow patch)
reproduces the post-tree, emit one hunk: the terse fn rename
with `# follow=uses` (plus `# scope=file` when the site spread
exceeds the 64-line rule). Any unproven same-name call site in
the file vetoes to today's behaviour (definition sweep + call
sweep separately, or ordinary) — never a partial follow claim.
Cross-file fn intents use the WI-15 probe exactly as item
intents do.

**Changes.** SPEC D.4 (`item_intent` fn arm). Code:
[item_intent.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/item_intent.rs);
follow-domain reuse from the engine via the existing probe
facade in [rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs).

**Acceptance.** The F8(b) probe compresses to one follow hunk and
cycles (`46-fn-follow` leaves the hetero list or the landing
change names its remaining divergence); a same-name call on an
unrelated receiver vetoes to separate hunks (pin); bench 40-series
gains the fn counterpart at its byte target.

**Tests.** Intent matrix (bare calls / anchored methods / mixed
veto / cross-file); fixed-point pins; history-sweep spot pins.

**Non-goals / risks.** No follow emission for fn *evolution*
(signature changes rename nothing — C17 stands). Risk: call
sites that are actually macro-generated — opaque veto, as today.

### WI-14 (P2, M): Macro interiors at the depth the engine already proves [carries F8(d)]

**Problem.** F8(d): WI-17 raised token-tree *argument* holes and
rule-body interiors multi-site, but a literal edit inside a
`macro_rules!` body (`inner!(1)` → `inner!(2)`) and single-site
token-tree holes still re-diff ordinary.

**Decision — spend WI-9's guard inside macros.** (1) The
`macro_rule_body` detector gains the `PrecisionUnique` guard
(WI-9): a single rule-body pair whose holed template matches
exactly the diff sites under the existing one-level token-tree
matcher fires at N=1. (2) Token-tree argument holes
(`old_m!(E, 1)` family) gain the same guard. Depth stays at one
argument level and rule-body interiors stay non-recursive
(round-3 rejection stands); anything deeper or ambiguous stays
ordinary with the depth veto pinned. Emission reuses the WI-7a
holed spellings verbatim — no new syntax.

**Changes.** SPEC D.4 (macro singles), 21 (depth statement).
Code: guard wiring in
[macro_rule_body.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macro_rule_body.rs)
and [macros.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macros.rs).

**Acceptance.** The F8(d) probe compresses and cycles; fixture
`48`'s single-site siblings flip as named in the landing change;
depth-2 nested probe stays ordinary (pin); benches 17/18 hold
their (WI-5-re-pinned) bytes.

**Tests.** Macro single matrix (unique / twin veto / depth-2
veto); `S″ == S′` pins per flipped fixture.

**Non-goals / risks.** No macro expansion, no `macro_rules!`
*pattern*-side holes. Risk: token-tree totality on large macro
bodies — scale gate (WI-20) covers the worst bench.

### WI-15 (P2, L): `# symbol` beyond call shapes [carries F8(e)]

**Problem.** F8(e): C14 confines `# symbol` to call shapes, so the
directive's promise — “this name is that item in this crate” —
cannot be stated for the `let`/type/general positions where API
renames actually bite, and method idents are unstateable.

**Decision — same proof, more positions.** The crate graph
([crate_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/crate_graph.rs))
already resolves `crate::`/`self::`/`super::` seeds; extend the
C13 seed-verification gate to general-expression, `let`, and type
hunks whose symbol occupies a *path-head* position (the positions
the general matcher already compares as paths, R1/R5), and to
method-call *receiver-path* symbols (the receiver's head path —
the method ident itself stays a bare ident and the C14 method-ident
rejection stands, now with a `write_instead` naming the receiver
form). Matching stays exact-path-text once the seed verifies
(SPEC 19.3 discipline); only the gate widens. `# fallback=syntax`
semantics unchanged. Symbol report key (`resolved`/`syntax`)
covers the new shapes identically.

**Changes.** SPEC 19.3 (position list), 18.6 C13/C14 rows. Code:
gate widening in `pattern_classify` + `plan_symbol.rs`; matcher
path-head checks reuse R1/R5 comparisons (no new matcher).

**Acceptance.** Probes: `let x = Old::make()` , `Vec<Old>` type,
and `E: Old::Trait`-style general positions verify `resolved`
against a vendored namesake that text-matching would have hit
(veto pin from round 3 still passes); method-ident spelling
fails with the new `write_instead`.

**Tests.** Symbol position matrix (fires / vendored veto /
fallback-syntax parity with round-3 pins); C14 narrowed-row
verbatim pins updated in the same change.

**Non-goals / risks.** No trait solving, no glob-path resolution
beyond the existing one-hop rule. Risk: path-head detection
disagreeing with the general matcher on exotic paths — the gate
only *admits*; the matcher still decides matches.

### WI-16 (P2, L): Anchors grow up — chains, captures, and honest orphan budgets [carries F8-residue, F5-evidence]

**Problem.** WI-14 anchors prove receivers from params, annotations,
literals, and `self`. Everyday Rust also proves them from method
chains (`s.clone().old()` — receiver type flows through known
return positions only syntactically), closure captures, and generic
bounds (`fn f<T: Tr>(t: T)` for assoc/member claims); today all of
these fall to `off-target` evidence, and the orphan list itself
caps at 8 lines with 80-char texts, so a wide rename's evidence is
a sample, not a set.

**Decision — two bounded extensions.** (1) Anchor classes gain, in
order: chain-base propagation (a method chain claims iff its
*base* receiver is anchored and every intermediate method is
name-stable under the rename — i.e. not the renamed ident);
closure-capture (a use inside a closure in the same block claims
iff the captured binding is anchored at the closure's definition);
generic-bound (`T: Tr` / `T = S` bindings in the enclosing fn/impl
prove `T::Old` and `t.old()` sites for the corresponding domain).
Each class is syntactic, ordered after the existing classes, and
individually vetoable in tests. (2) Orphans/follow-skips gain a
counted-overflow contract: `orphans_truncated` already exists;
add `orphans_total` (full count, always present when orphans are)
and let `--bindings full` lift the 8-entry cap alongside the
80-char cut (one expensive-evidence flag, D.2 documents both
lifts). Default reports keep today's byte pin (WI-20 recomputes).

**Changes.** SPEC 14.8 (anchor order + overflow keys), D.2. Code:
`use_graph_anchor.rs` chain/capture/bound classes; report plumbing
in [applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs).

**Acceptance.** Chain/capture/bound probes rewrite; a chain through
the renamed method itself stays evidence (pin); a 20-orphan rename
reports `orphans_total: 20`, 8 entries by default, all 20 under
`--bindings full` with digest-equal texts.

**Tests.** Anchor matrix per new class (claim / shadow / veto);
overflow pins; budget pin recomputed, never loosened silently.

**Non-goals / risks.** No return-type inference through arbitrary
methods — chains propagate the *base* anchor only. Risk: capture
claims across `move` closures with shadowing — shadowing ends
claims (WI-2 discipline), pinned.

### WI-17 (P1, M): Raise from patch text, verify in one command [carries F10]

**Problem.** F10: raising requires git state, and verifying a raise
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
with the sibling `applyrs`, byte-compare against the post-tree
implied by the input (git state or `--patch` applied natively) —
and reports `{version, checked: {raising: "verified"|"partial",
tree: "identical"|"diverged"}, …}` in JSON, patch on stdout in
text mode only when identical. A diverged check exits 2 with the
first differing path in `error.reason` and the raised patch still
printed (review aid, marked diverged in JSON).

**Changes.** SPEC D.1/D.4 (input + check contracts), CLI help.
Code: source adapter in
[rsmpl_diff_source](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_source/src/lib.rs)
(patch-text sections → the existing section model); check loop
in [diffrs main](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs)
reusing `rsmpl_git::apply` primitives — no new apply logic.

**Acceptance.** A native patch saved from `git diff` raises
identically via git state and via `--patch`; `gitrs diff --check`
on the full fixture corpus reports identical trees; a hand-broken
raise (sibling site the patch would over-match) reports
`diverged` naming the file — the case `--check`-piping missed.

**Tests.** Input parity matrix (staged / rev / patch-text);
check-loop pins (identical / diverged / unappliable base);
dispatcher passthrough pins in `gitrs`.

**Non-goals / risks.** No PR/URL fetching — patch text only.
Risk: temp-tree cost on large repos — bounded to changed paths
(the source model already enumerates them); scale gate covers.

### WI-18 (P1, S): Evidence you can act on without re-reading files [carries F5-evidence, F8-reporting]

**Problem.** Follow-only entries carry spans but no `sites`;
diffrs JSON provenance (`hunks`) is empty for whole classes
(rename sections probe-verified: `"hunks": []` beside a correct
patch); text-mode follow/orphan lines go to stderr interleaved
with progress. An agent correlating “which detector claimed this,
at which sites, with what bindings” still re-reads files.

**Decision — close the three holes additively.** (1) Follow-only
entries gain `sites: [{span, bindings: {}}]`-shaped provenance
without bindings: emit `follow_sites: [spans]` is *not* added —
instead `spans` stays authoritative and each follow entry gains
`detector`-style origin: `origin: "follow"` (definition entries:
`"definition"`, ordinary: `"ordinary"`) so consumers stop inferring
provenance from `semantics`. (2) `diffrs --format json` populates
`hunks` for *every* emitted hunk (semantic, ordinary-passthrough,
rename) with `{rule| null, detector| "ordinary", file, spans}`;
today's semantic-only population is the bug. (3) Text mode keeps
stderr lines byte-identical (scripts may match them); JSON mode
guarantees stdout is exactly one JSON object (already true —
pinned by a new test so it cannot regress).

**Changes.** SPEC D.2 (both reports). Code: report assembly in
[applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
and [diffrs main](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs);
provenance threading from
[rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs)
(`HunkProvenance` already exists — emit it for all hunks).

**Acceptance.** Two-file follow JSON distinguishes definition vs
follow entries without reading `span_edits`; rename-section
diffrs JSON lists its hunks with detectors; budget pins recomputed
for the additive keys.

**Tests.** Provenance pins per hunk class; single-JSON-object
pin; budget test updated.

**Non-goals / risks.** No pagination protocol. Risk: additive keys
tempting consumers to depend on detector *names* — SPEC marks
detector names diagnostic, not contractual (rule names are).

### WI-19 (P2, M): Everyday completeness batch — the next ten ordinary diffs [carries F8-residue]

**Problem.** Beyond F3/F8, dogfood diffs still strand uniform edits
as ordinary. This item is deliberately a *batch with a fixed menu*,
each entry landing only with its pre-image-totality proof and a
fixture: (a) closure-param / match-arm binder completion (WI-2's
evidence-only binders learn block completion where the arm body /
closure body is the scope); (b) `?`/`await` chain deltas as
general-template anchors (operator-column hetero currently vetoes
— classify `?` and `.await` as anchor positions, not holes);
(c) single-site derive/attr deltas under `PrecisionUnique`
(today: multi-site only via the `N`-hole sweep); (d) use-group
member addition/removal (`use a::{x}` → `use a::{x, y}` as a
use-shape template, not a text hunk); (e) `mod` file-content moves
where the file rename already fires but its inner items also moved
(compose WI-15c with WI-7 moves instead of choosing one).

**Decision.** Each menu entry is an independent slice with its own
detector-or-matcher gate, fixture pair (`*-fires` /
`*-stays-ordinary`), and bench-or-history evidence; an entry that
cannot prove totality is cut from the batch and recorded in the
landing change — the batch never ships a guessed shape to meet
its count. Order inside the batch is (a)→(e) (trust before
tokens).

**Changes.** SPEC rows per slice (18.6/D.4 as applicable). Code:
slices land in the owning modules named per slice; no slice grows
a D5-watch file (new siblings where needed).

**Acceptance.** Each landed slice's fixture pair passes with
real-apply byte-compare; the batch's net effect on
`tools/history_sweep.py` is reported (compressing set grows or
the cut entries are named); `KNOWN_DIVERGENT` flips are listed
per slice.

**Tests.** Per-slice pairs + veto pins; sweep re-run.

**Non-goals / risks.** Anything needing type inference, macro
expansion, or formatting normalisation stays ordinary. Risk:
batch sprawl — capped at the five menu entries; a sixth waits
for the next round.

### WI-20 (P1, L): One contract, green gates — schema v5, honest counts, split files [carries F9; bumps D7]

**Problem.** F9: the workspace gate is red at HEAD, SPEC cites a
corpus a third its real size, five files crowd the size cap, and
WI-6/WI-7/WI-8/WI-18 change evidence meanings under schema v4.

**Decision — one hygiene landing after the evidence settles.**
(1) Fix the gate: the 10000-line scale fixture is generated by
its bench harness into `target/` (or excluded by path in the
gate with the exclusion named in the test) — committed `.rs`
sources obey 2000 lines without exception. (2) Bump
`REPORT_VERSION` to 5 with WI-6/WI-7/WI-8/WI-18's meaning changes
(follow entries can be `instantiated`; build failures carry the
new code; `origin`/`orphans_total` keys) and remove nothing else;
tri-way parity test (applyrs/diffrs/gitrs error envelopes) pins
v5. (3) SPEC/AGENTS/SKILL truth pass: corpus counts generated
from disk in the doc-lint test (counts in prose must equal
`test-fixtures/` listings), every RSmPL fence lint extended to
the new spellings, AGENTS.md stays within its line budget by
moving detail to SPEC (never by merging rows unreadably — the
WI-25 squeeze is reverted where it hurt). (4) Split the two
largest watch files the round touched (`plan.rs`,
`postcheck.rs` or `semantic/mod.rs`, per landing state) along
the seams this plan already created. (5) Scale gates (benches
43/44) become CI assertions with recorded budgets instead of
review artifacts.

**Changes.** SPEC D.2/D.5 (v5 + counts), AGENTS.md, SKILL.md.
Code: gate + lint tests in `bin/applyrs/tests/fixtures.rs` /
`spec_examples.rs` and `bin/diffrs/tests/`; splits are
behaviour-preserving moves only.

**Acceptance.** `cargo test --workspace` fully green at HEAD
(including the previously red gate); `REPORT_VERSION == 5`
everywhere with parity pins; doc-lint fails when a fixture is
added without updating counts (self-test); no `.rs` file over
2000 lines and touched watch files under 1600 or split.

**Tests.** The gates themselves; v5 envelope pins across all
three binaries; split-diff review (no semantic change).

**Non-goals / risks.** No new semantics in this item — anything
semantic found while splitting goes back to its owning WI.
Risk: v5 churn for consumers — accepted per D7 (prototype,
breaking changes allowed, one bump only).

## Sequencing and gates

| Item | Depends on | Size | Gate before next item |
|---|---|---|---|
| WI-1 assoc definitions | — | M | F1 probes rewrite + `cargo check` green; inherent namesake untouched |
| WI-2 binding honesty | — | M | F2 probe completes or downgrades with orphans; never silent `verified` |
| WI-3 method follow | WI-1 trait discipline | L | Anchored calls move; other-type namesake is evidence; C17 pins updated |
| WI-6 follow verification | WI-1, WI-3 domains | M | Cross-file follow reports `verified`; tampered span fails loud |
| WI-7 move verification | WI-6 probe discipline | M | Move probe `postcondition`; tampered paste fails loud |
| WI-8 build truth | WI-1–WI-3, WI-6 evidence | M | Incomplete domains report `partial`; build failure carries repair |
| WI-4 frame raising | — (parallel-safe with WI-1–WI-3) | L | F3 taxonomy compresses byte-identical; frame-delta veto pinned |
| WI-5 anchored templates | WI-4 site sets | M | F4 emits anchored form; site sets unchanged corpus-wide |
| WI-9 precision singles | WI-5 spellings | L | Named `t375` entries flip; sibling veto pins hold |
| WI-10 site-set canonical | WI-9 spellings | M | `01`/`90`/`51` leave the divergent table |
| WI-11 joint impl intent | WI-6, WI-13 machinery | M | `14` reaches 9+0 or survivor named |
| WI-12 line partition | WI-4 (frames excluded from splits) | L | `12` compresses its let run; whole-section winners unchanged |
| WI-13 fn follow raising | WI-3 domain, WI-4 frames | M | `46` flips or divergence named; veto pin holds |
| WI-14 macro singles | WI-9 guard | M | Rule-body probe cycles; depth-2 veto pinned |
| WI-15 symbol widening | WI-16 anchors (soft) | L | Vendored veto holds on new positions |
| WI-16 anchor growth | WI-3, WI-15 domains | L | Chain/capture/bound matrix green; overflow keys pinned |
| WI-17 patch-input raising | — (any time after WI-4) | M | Patch-text parity + diverged-check pin |
| WI-18 evidence closure | WI-6 entries | S | Rename-section JSON lists hunks; single-object pin green |
| WI-19 everyday batch | WI-2, WI-9 | M | Per-slice fixtures; cut entries named |
| WI-20 one contract v5 | WI-6, WI-7, WI-8, WI-18 | L | Full workspace green incl. fixed gate; v5 parity; counts linted |

Global validation per item: `cargo test --workspace`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`./example-benches/run.sh` (README tables regenerated only when
bytes change), and `python3 tools/history_sweep.py` (0 failures;
compressing set reported, never asserted to a fixed number beyond
“does not shrink without a named reason”). No item lands with a
red gate, a new `.rs` file over 2000 lines, or an existing file
pushed over 2000 lines (D5). Every raising item proves itself by
real apply + byte-compare (T361), never by `--check` alone; every
lowering trust item (WI-1, WI-2, WI-3, WI-6, WI-7) carries a
durable apply-then-`cargo check` fixture, not only JSON pins.

## Risks / open questions

Per-item risks are stated with each item. Open questions: none —
scope, canonical forms, detector order, guards (64-line range
rule, precision-uniqueness probe, one-level macro descent),
anchor proof order, schema versioning (unified v5 in WI-20), and
acceptance numbers are decided here. If a threshold proves wrong
against the probe corpora, the fix is a new pin plus a SPEC note,
not a silent constant edit. Three deliberate non-goals bear
repeating because they bound several items: no type inference or
trait solving anywhere (anchors are syntactic — WI-3/WI-15/WI-16
prove from text, never from `rustc`), no macro expansion (SPEC
21.5 stands; WI-14 deepens rendering-level matching by guard
only), and no minus-must-be-zero postcondition (widening rewrites
legitimately re-match their own output — WI-6/WI-7 verify the
plus/move at its spans instead). A fourth joins them this round:
no default `cargo check` (WI-8 makes the *evidence* honest
instead of making the compiler mandatory).

## Considered and rejected

- **Always-on build verification (default `--verify build`).**
  Rejected: couples every `--check` to a toolchain and a
  `Cargo.toml`, breaks the text-cost model, and teaches agents to
  route around the tool. WI-8 buys the honest signal (domain
  completeness → `partial`) at text cost; the compiler stays an
  explicit, repairable escalation.
- **Full type-directed resolution (rust-analyzer integration).**
  Rejected as in round 3: un-auditable in a prototype and
  unnecessary for the provable majority — WI-16's syntactic
  anchors cover chains/captures/bounds without it.
- **Dropping T375 entirely (always emit semantic).** Rejected:
  sweep-class templates whose minus matches wider than the diff
  are exactly where over-match risk lives; WI-9 narrows the guard
  to uniqueness-proved singles instead of abolishing it.
- **Emitting frames in templates (`fn f0() { foo(E) }` as the
  pattern) to fix F3 from the authoring side.** Rejected: the
  engine already matches the subterm; frames are a *raising*
  blind spot, not a language gap. Authoring new frame syntax
  would tax every agent to fix one detector.
- **A separate `verifyrs` binary.** Rejected: trust rides the
  existing `checked` envelopes (WI-6/WI-7/WI-8) and the one new
  check verb lives on `gitrs diff --check` (WI-17), keeping
  `gitrs` a two-subcommand dispatcher.
- **Recursive token-tree macro descent.** Rejected in WI-14 as
  before: one argument level is provable with the existing
  token-tree matcher; deeper descent invents a second macro
  engine inside the first.
- **CRLF/binary semantic compression.** Stays declared-lossy
  (D1): detectors strip `\r` by construction and patches are
  UTF-8; passthrough tree-losslessness is the correct contract.

