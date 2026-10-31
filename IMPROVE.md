# IMPROVE — RSmPL as a verifiable agent contract, round 3: structural proof, everyday-Rust completeness, lossless cycles

## Goal

Make RSmPL the contract an agent can trust in both directions without
reconstructing and inspecting every edit by hand:

- **Lowering** (semantic → native, `applyrs` via `gitrs apply`) must prove,
  per site, what it touched, what it deliberately left alone, and what it
  checked — by re-matching the authored pattern in the post-image, not by
  searching for substrings, and never in fields that are true by
  construction.
- **Raising** (native → semantic, `diffrs` via `gitrs diff`) must compress
  the refactor shapes real Rust code actually takes — body rewrites,
  literals, attributes, definition values, associated items — at
  pre-image-proven precision, so the semantic form is the cheapest
  *trustworthy* view of a change, not merely the cheapest bytes.
- **Cycles must close under stated equalities**: semantic → native →
  semantic and native → semantic → native, with every remaining divergence
  named, owned, and pinned — never silent. The single-site demotion class
  that now dominates the pinned-divergent list must shrink by decision,
  not by drift.

`gitrs` stays a thin dispatcher (`apply` → `applyrs`, `diff` → `diffrs`);
all semantics live in the crates. Backwards compatibility is not required:
breaking changes to [SPEC.md](/home/ai/git/tools/rsmpl-git/SPEC.md), the
JSON envelopes, and emitted patch text are allowed where this plan says so.
Token efficiency matters, but never at the cost of a guess: every new
capability fails loud or stays ordinary when it cannot prove itself
(SPEC 2.6). Syntax stays familiar — Git hunk shape, SmPL-style `#`
directives, Rust-shaped patterns — because agents and humans already know
those spellings from training data and daily use. Diet spelling
(`# sweep`, inferred metavars, terse renames) remains canonical.

This review supersedes the previous IMPROVE.md in full. That plan's
WI-1…WI-10 all landed (see the carry-forward audit); this round carries
their residues forward explicitly and adds the fresh gaps this review
probe-verified. Work stops at 20 items.

## Success criteria

1. **Trust (WI-1, WI-2, WI-3).** No success field is true by construction
   and no verification is a substring search. A patch whose plus side
   carries a string literal or `...` verifies structurally at its reported
   spans, or the report says `reparse-only` and the global word is
   `partial` — the per-rule `semantics` word itself is the
   machine-visible verdict, never a prose explanation. Every binding an agent
   might act on is either complete or accompanied by its length and
   SHA-256, so equality across sites is assertable without paying for the
   bytes twice. Every patch/IO failure carries a machine `repair`, or the
   conformance suite names the one exception class that cannot.
2. **Everyday Rust authors and raises (WI-5…WI-13).** Function-name holes,
   general expressions, literal frames, control templates, header
   slot-only evolution, attribute deltas, definition right-hand sides,
   and associated items have authored spellings an agent already
   recognises from Git/SmPL/Rust, and detectors that raise them only
   under pre-image totality with a real-apply byte-compare (T361
   doctrine: `--check` alone never proves a sweep).
   `python3 tools/history_sweep.py` stays at 0 failures and the
   compressing set does not shrink without a named reason in the landing
   change.
3. **Follow-through (WI-14, WI-15, WI-16).** A rename rewrites the sites
   the code itself proves belong to it — typed receivers, literal labels,
   multi-hop re-exports, cross-file members, `mod` + file renames — and
   lists every other same-name site as structured evidence with a reason
   code. No same-name sibling elsewhere in the workspace vetoes a rename
   whose proven sites are unambiguous. `# symbol` either verifies
   against a crate-local resolution graph or fails as syntax; it never
   silently degrades to the text match it claims to improve on.
4. **Cycles (WI-17, WI-18, WI-19).** The `cycle_e2e` harness stays green;
   the SPEC D.5 exclusion list and the `KNOWN_DIVERGENT` table shrink by
   named families (`nodet` first, then the `t375` single-site mass under
   the WI-19 precision guard, then partial partition for the rest) — they
   never grow silently. Token-tree holes, collection moves, and
   mixed sections re-diff to the shape that was applied, or stay pinned
   with a sharpened owner.
5. **Self-correction (WI-3, WI-4).** A stale ranged patch either applies
   with `drift` evidence naming cited vs actual spans, or fails with a
   `repair` an agent can apply verbatim — never with a bare `null` and a
   paragraph. Every new compile-time fence carries a concrete
   `write_instead` template; no `repair` echoes prose.
6. **Contract hygiene (WI-20).** One report version across `applyrs`,
   `diffrs`, and `gitrs` errors; every RSmPL fence in SPEC.md,
   AGENTS.md, and `skills/gitrs/SKILL.md` is machine-linted, so the
   documents an agent learns the language from cannot teach a spelling
   the engine rejects.
7. **Gates.** `cargo test --workspace`, `cargo clippy --all-targets --
   -D warnings`, `cargo fmt --check`, and `./example-benches/run.sh` stay
   green per item; no `.rs` file exceeds 2000 lines (split before growing,
   D5); bench byte targets in each item are met or the item does not land.

## Review findings (evidence)

Verified this run at HEAD `ae6d2f3` (531 commits): **2148 tests passing,
0 failed (summed across all `test result` lines); clippy and fmt clean;
`./example-benches/run.sh` re-measures byte-identical to the committed
tables (working tree clean after the run).** Probes below were run with
the release binaries against throwaway `/tmp` git repos (authored
`applyrs --format json --check` and pending-change `diffrs` conversions);
each finding names the probe so the landing change can re-run it as a
fixture. The architecture is sound and stays:
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
siblings. The gaps are proof strength, authoring completeness for
everyday Rust, and the raising/cycle tail — not correctness of what fires.

### Carry-forward audit of the previous plan

| Old item | Verdict | Evidence / residue |
|---|---|---|
| WI-1 honest `checked` | Landed | Plus-instantiation + per-rule `semantics`, schema v3, marked truncation — but instantiation is a substring search that abstains on strings/`...` (F1) |
| WI-2 everyday expressions | Landed | Five `match_expr` arms + C19 lint + head-token preview fire; the anti-unification side never learned the same shapes (F5, F6) |
| WI-3 structural patterns | Landed | Six `match_pat` composites + `gen_pat` fire; untyped let-init templates with destructuring still decline (F6) |
| WI-4 headers + trait raising | Landed | Header is a filter in every scope; `trait_method_rename` groups same-file — pure trait-side and header slot-only classes stay ordinary (F7) |
| WI-5 hetero arg templates | Landed | Wrap/permute specified in SPEC D.4; whole-arg permutation (`swap(E0, E1)` → `swap(E1, E0)`) still declines in code (F6) |
| WI-6 collection captures | Landed | `# bind` + `# allow=many` cuts/pastes as one; the authored collection move does not re-raise (F10), and collections still cannot evolve per item (SPEC 33.2) |
| WI-7 macro depth | Landed | `macro_rules!` rename + per-arg token-tree holes author; raising emits whole-body `...` instead of the holed form (F10) |
| WI-8 pair partition | Landed | All-or-nothing per section; one double-delta pair keeps 8 claimable renames ordinary (F11) |
| WI-9 field/variant renames | Landed | Authored + follow + raising fire single-file; cross-file is declared future work in SPEC D.4, and follow vetoes on any same-name sibling (F8) |
| WI-10 CLI contract | Landed | Drift (context-anchored), `--context`, handshake, `diffrs` v2, rev-base — terse hunks without context lines still cannot drift (F4), and the envelopes now split v3/v2 (F3) |

Fixture expansion after that plan (applyrs fixtures to 120, diffrs to
100) grew the `KNOWN_DIVERGENT` table in
[cycle_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/cycle_e2e.rs)
to roughly 70 pinned entries, most of them `t375` single-site demotions.
That table, not the bench suite, is now the honest map of where the
cycle contract is defined-but-lossy; WI-19 owns shrinking it.

### Findings

- **F1. Verification is a substring search, and it abstains exactly where
  Rust lives.** The plus-instantiation gate in
  [postcheck.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck.rs)
  (`plus_instantiated`) returns `None` — downgrading the rule to
  `reparse-only` and the run to `semantics: partial` — whenever the plus
  template contains a `"` or a code-level `...`. Probe-verified: a
  three-site sweep `- foo(E)` / `+ bar(E, "hi")` applies correctly yet
  reports `reparse-only`/`partial`; benches 17 (macro sweep), 25 (async
  blocks), and 28 (verbatim-macro `let`) print `semantics partial` for the
  same reason. Where the gate does fire it is weaker than its name:
  renames pass when the new ident occurs *anywhere* in the post-image
  (`post_text.contains(new)`), so a pre-existing same-name item satisfies
  a rename that never landed at its span. The engine already owns real
  matchers for every plus shape; the postcondition never calls them.
- **F2. Truncated evidence cannot be asserted on, and skip evidence is
  prose.** Binding values cut at 80 chars gain a `…` and a count
  (previous WI-1), but carry no length or digest: an agent seeing
  `some_function_with_long_name(argument_one, …` cannot tell whether two
  sites bound the same expression, nor fetch the full value without
  re-reading the file the report exists to avoid. `follow_skipped` is one
  `; `-joined string and `orphans` entries carry no reason code, so
  neither is assertable. Signature-evolution spans repeat the same
  `[1,1]` three times for one match (probe-verified on a lifetime-evolving
  rename) with nothing naming which slot each edit belongs to.
- **F3. Two report versions, and `repair: null` on common failures.**
  `applyrs` success/error envelopes are `version: 3` while `diffrs`
  prints `version: 2` for the same error object (probe-verified side by
  side); an agent must branch on which binary answered. Two everyday
  failures carry no machine repair at all (probe-verified):
  `RSMPL_RANGE_CONFLICT` (range past EOF) and unknown-directive
  `RSMPL_PATTERN_CONFLICT` both print `"repair": null`, although both
  have mechanical fixes (clamp to the file length; name the closest known
  directive). The seven repair producers from the previous plan cover the
  shapes they pinned, not the error surface.
- **F4. Drift still requires context lines the canonical spelling omits.**
  The SPEC 10.8 anchored retry needs leading/trailing ` ` body lines as
  literal anchors. The diet forms agents actually write — terse renames
  (`- fn foo` / `+ fn bar`), bare call pairs — carry no context lines, so
  a stale range fails with `RSMPL_MATCH_CONFLICT` and a
  `widen_scope` repair that costs a second authoring round (probe-verified:
  a rename authored 3 lines stale fails; the same hunk as `# sweep`
  would have applied). The engine already computed the decisive fact —
  exactly one match exists elsewhere in the file — and then declined to
  act on it for `allow=one` hunks.
- **F5. Raising has no general-expression detector, so body rewrites never
  compress.** The detector chain in
  [chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs)
  (`run_pair_chain`) covers calls, methods, `let`, types, and verbatim
  lines — but no bare-expression anti-unification. Probe-verified with
  multi-line fixtures: `1 + 1` → `1 + 2` across three fn bodies, a match
  guard `v > 0` → `v >= 0`, closure bodies `x + 1` → `x + 2`, and
  `foo().await + 1` → `+ 2` all stay ordinary, although the authored
  forms (`- E + 1` / `+ E + 2` under `# sweep`) apply and verify today.
  The engine is ready; only the detector is missing. This is the largest
  single raising gap: most real diffs edit expressions inside bodies.
- **F6. Template completeness lags the engine on four pinned shapes.**
  (a) Whole-arg permutation declines despite SPEC D.4 promising it:
  fixture `60-call-arg-swap` (`swap(E0, E1)` → `swap(E1, E0)`) is pinned
  `nodet`. (b) Untyped let-init templates decline: fixture
  `57-some-unwrap` / `68-diet-some` (`let Some(P) = E;` →
  `let Some(P) = E.unwrap_or(0);`, two sites) stay ordinary because the
  joint template detector requires an ascription. (c) Initializers
  holding `const { … }` or `_` decline (`53-const-block`,
  `55-infer-wildcard` pinned `nodet`) although WI-2 taught the matcher
  both arms. (d) Tuple-pattern swaps decline (`56-tuple-pat-swap`)
  although `gen_pat` matches the composites. Each is a detector-side
  veto over an engine-proven shape.
- **F7. Header evolution without a rename has neither spelling nor
  detector.** Function patterns bind visibility/qualifiers/generics/where
  as slots (`V Q G W`) but the fn *name* is always concrete — there is no
  name hole — so "add `pub` to these four functions", "give these two
  functions a `T: Clone` bound", and every other slot-only sweep across
  differently-named functions cannot be authored as one hunk, and
  `sig_evolution` (which requires one shared name delta) cannot raise it.
  Probe-verified: four `fn a/b/c/d` → `pub fn …` stay four ordinary
  hunks; two `fn foo<T>`/`fn bar<T>` gaining `: Clone` stay ordinary.
  A single-site slot evolution authored with explicit slots applies
  (probe-verified for lifetimes), so the missing piece is the name hole
  plus its sweep detector.
- **F8. Member follow is ident-global, not definition-anchored.**
  Probe-verified: renaming `A::timeout` with `# follow=uses` in a file
  that also defines `B::timeout` fails the whole run with
  `RSMPL_SYMBOL_CONFLICT` ("shadowed definition … cannot tell which
  definition each use names"), although the uses sit on receivers the
  fn signature itself types (`fn f(a: A, b: B)`) and in `A { … }`
  literals whose path names the struct. The definition rename — the one
  edit that is unambiguous — is blocked by sites a type anchor would
  have classified. With `# fallback=syntax` the ambiguous sites skip,
  but the skip set is the F2 prose string, and provable `A` sites reached
  only through typed locals still depend on the ident-global scan.
- **F9. Attributes and definition values cannot evolve semantically.**
  Probe-verified: adding `#[inline]` to a fn fails C11 ("replacement
  writes attributes the pattern did not match") — there is no authored
  spelling for add/remove/change of `#[derive(…)]`, `#[cfg(…)]`,
  `#[inline]`, or doc lines, and no detector raises the four identical
  `+#[inline]` insertions a real diff produces. Definition right-hand
  sides are likewise frozen: `- type A = Vec<T>;` fails with "concrete
  item shape after `<type> <name>` is not supported", field renames
  require the type token-equal on both sides, variant payloads and
  `const`/`static` values have no evolution shape at all, and associated
  `type`/`const` items have neither rename nor evolution (a trait-context
  `type Item;` pair fails as an unsupported concrete item shape).
  Probe-verified raising side: field-type `u32` → `u64`, variant-payload
  `A(i32)` → `A(String)`, struct-field addition with construction-site
  updates, and enum-variant addition all stay ordinary.
- **F10. Three cycle families are detector-shaped holes, not byte ties.**
  (a) Token-tree holes: fixture `48-token-tree-arg-hole`
  (`old_m!(E, 1)` → `new_m!(E, 1)`) re-diffs as a whole-body `...`
  verbatim sweep — raising never emits the per-arg hole form the engine
  has matched since WI-7a. (b) Collection moves: fixture
  `44-collection-move` (three fns, `old.rs` → `new.rs`) re-diffs as
  ordinary additions plus a paste-less `- I` cut — the collection
  emission that bench 31 proves at 91% saved declines on the re-diff
  shape (source file emptied to zero bytes). (c) Macro interiors stop at
  one level by design; `macro_rules!` *body* edits (an `inner!(1)` →
  `inner!(2)` inside a rule body) stay ordinary because token streams
  are compared whole. Each family has an engine-proven authored form
  waiting for its detector.
- **F11. Partition atomicity wastes claimable renames.** Fixture
  `14-item-rename` holds nine renames; eight are single-pair claimable,
  but the `impl Printable for Old` → `impl Labeled for New` double-delta
  pair vetoes the whole section under WI-8's all-or-nothing rule, so the
  re-diff is 1 semantic + 7 ordinary where 8 semantic + 1 ordinary is
  available and sound (SPEC 10.6 already permits semantic + ordinary
  mixing in one section when ranges are disjoint). The same all-or-nothing
  posture keeps mixed header+let sections (fixture `12`, declared a
  permanent non-goal under the old fusion design) fully ordinary even
  where a per-hunk split would compress the let run.
- **F12. `# symbol` still cannot resolve, and the honest shapes are
  taxed.** v0.1 has no semantic environment: `# symbol Old=crate::old_api`
  without `# fallback=syntax` fails C13, and with it matches exact text —
  the same result as writing the concrete path, plus two directive lines
  and a `New`/`Old` binding pair in every `sites` report (probe-verified).
  SPEC 19.3 already prefers concrete paths. What neither spelling offers
  is the thing the directive promises: proof that `Old` names the item at
  that path in *this* crate. A crate-local `mod`/`use` graph (the same
  syntactic discipline as `follow=uses`, no build, no rust-analyzer) can
  provide exactly that proof for `crate::`/`self::`/`super::` paths.
- **F13. Item-rename intents fragment across contexts.** Probe-verified:
  `struct S` → `struct T` with its `impl S` block and an `S`-typed fn
  parameter (three lines, one intent) stays three ordinary hunks. The
  single-pair rename detector claims at most one of the three lines, the
  partition pass claims none (the `impl S {` line is not a bare rename
  pair), and no grouping joins them — although authored
  `- struct S` + `# follow=uses` semantics already define which sites
  belong to the intent. The same fragmentation keeps `mod old;` →
  `mod new;` plus its file rename and `old::` path uses as unrelated
  sections, and member renames cross-file (SPEC D.4: "cross-file
  definition+uses intents stay ordinary (future work)").
- **F14. The documents that teach the language are only sample-tested.**
  [spec_examples.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/tests/spec_examples.rs)
  (292 lines) lints tagged fences and hand-pins six SPEC §29 examples;
  the RSmPL fences in AGENTS.md and `skills/gitrs/SKILL.md`, and the
  remaining SPEC examples, are checked by nothing. The recent history
  (diagnostic-hint and gate-headroom commits at HEAD) shows wording and
  behaviour drifting in small steps — each drift is an agent confidently
  authoring a rejected spelling. SPEC.md is 7583 lines with no agent
  quick-contract; the one-page summary lives in AGENTS.md and can diverge
  undetected.

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
  finding its text somewhere in the file.
- **D3. Inference stays precision-first.** Every new detector keeps
  pre-image totality (count must equal pair count, zero opaque hits) and
  the T375 byte-win guard measured in diet bytes, except the
  precision guard for definition shapes (introduced in WI-12,
  generalised in WI-19), which is itself bounded and pinned. A detector that cannot prove totality stays ordinary — no
  guessing (SPEC 2.6).
- **D4. One anti-unification core, thin detectors.** WI-6…WI-9 and WI-12
  reuse and extend the existing
  [antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
  core instead of growing per-shape string surgery. Engine work is
  structural `syn` matching — concrete, fail-loud, no expansion, no
  guessing.
- **D5. Split before growing.** New code lands in new modules; no item
  pushes a file over 2000 lines, and any file over 1600 lines touched by
  an item leaves under 1600 or with its new code in a sibling module.
  [plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
  (1756 lines), `matcher_fn.rs` (1732), `use_graph.rs` (1703),
  `matcher_expr.rs` (1582), and `capture.rs` (1495) are the watch list.
- **D6. Familiar syntax wins over clever syntax.** New surface reuses
  Git/SmPL/Rust spellings (`...`, `#` directives, concrete paths, context
  headers that look like the block they select, attr lines written where
  Rust writes them). Diet remains canonical; the parser keeps accepting
  long forms so `.history/` archives stay readable. Any new spelling must
  be explainable in one AGENTS.md row.
- **D7. Breaking changes are taken where they buy trust.** JSON schema
  stays at its current version for additive keys; a key whose *meaning*
  changes bumps the version with the old key removed, per SPEC D.2's own
  rule. SPEC text moves with code in the same item. This round bumps the
  unified report to `version: 4` exactly once (WI-20) after WI-1/WI-2
  have settled the evidence shapes it carries.
- **D8. Anchors before indirection.** Follow, symbol resolution, and
  member anchoring prove site ownership from text the change already
  contains — fn-signature param types, `let` annotations, struct-literal
  paths, `use` chains — before consulting any wider graph. A site no
  anchor proves is evidence (`orphans`/`follow_skipped` with a reason
  code), never a rewrite and never a veto on the proven sites.

## Work plan

Order is dependency order: WI-1 → WI-2 → WI-3 → WI-4 → WI-5 → WI-6 →
WI-7 → WI-8 → WI-9 → WI-10 → WI-11 → WI-12 → WI-13 → WI-14 → WI-15 →
WI-16 → WI-17 → WI-18 → WI-19 → WI-20. WI-1 first because every later
detector's `verified` claim rides on it; WI-5 (name hole) precedes WI-9
and WI-18, which spend it; WI-20 last because it bumps the unified
schema once, after WI-1/WI-2 settle the evidence shapes. Sizes are
relative (S ≤ 1 day, M ≈ 2–3 days, L ≈ 4–5 days of focused work).

### WI-1 (P0, M): Structural postcondition — re-match the plus, never search for it [carries F1; hardens WI-1/WI-2 of the previous plan]

**Problem.** F1/D2: `plus_instantiated` abstains on any plus containing
`"` or `...` (so string-bearing and body-preserving rewrites — the common
case — report `reparse-only`/`partial`), and where it fires it accepts a
rename whose new name merely occurs somewhere in the file. An agent
cannot distinguish "the engine re-verified this rewrite at its spans"
from "the text occurs in the file".

**Decision — span-local engine re-match.** After materialisation, each
rewrite hunk's plus pattern is re-matched *structurally at each reported
span* in the post-image, using the same matchers that matched the minus
(call/method/general/`let`/type/impl-context/fn-slot matchers, fed the
plus pattern and the recorded per-site bindings). A span verifies iff
the plus pattern matches the post-image node at that span with bindings
equal to the recorded ones (SPEC 15.7 equality). String literals compare
as literals inside the structural match (they were only ever opaque to
the substring search); `...` tails verify as "the absorbed tail is
present verbatim", checked by the block matcher, not by abstention.
Rename kinds (`Fn`, `ItemRename`, `FieldRename`, `VariantRename`,
`UseRename`) verify by matching the renamed header at the definition
span — occurrence elsewhere in the file never counts. Any span that
fails fails the run loud with the existing instantiation
`RSMPL_MATCH_CONFLICT`. Shapes with no structural matcher (insertions,
cuts, pastes, `Raw`) stay `reparse-only` — honest, never upgraded.
Per-rule `semantics` words and the global `verified`/`partial` rule are
unchanged; only the computation becomes structural. Schema stays
`version: 3` (additive: no key changes meaning — more rules earn
`instantiated`/`postcondition`, none lose an honest label they had).

**Changes.** SPEC 24.6 (span-local re-match; substring posture deleted),
18.6 (no row changes). Code: `plus_instantiated` in
[postcheck.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck.rs)
replaced by a `verify_plus_at_spans` that dispatches into the existing
matchers; the `contains` arms for renames are deleted. No new matcher is
written — dispatch reuses `matcher_expr`/`matcher_general`/
`matcher_fn` entry points over a post-image `syn::File` parsed once per
file (not per hunk).

**Acceptance.** The F1 probe sweep (`+ bar(E, "hi")`, 3 sites) now
reports `postcondition`/`verified`; benches 17, 25, 28 report
`verified` or the landing change names the remaining `reparse-only`
shape per bench. The tampered-post-image unit (plus reverted at its
span, same name present elsewhere) now fails — the old `contains`
posture passed it. A widening rewrite (`foo(E)` → `foo(foo(E))`) still
verifies (plus re-matches its own output; minus-zero remains rejected
by design).

**Tests.** Postcheck matrix per pattern kind (fire at span, fail at
span, name-elsewhere veto, string-bearing fire, `...`-tail fire);
bench semantics-word pins; full workspace + cycle harness.

**Non-goals / risks.** No type-level proof beyond opt-in
`--verify build`. Risk: a second matcher pass per rewrite hunk costs
time on large sweeps — bounded by reported spans (point re-matches over
one parsed post-image per file); the bench suite and the WI-20 scale
gate pin the cost.

### WI-2 (P0, M): Assertable evidence — binding digests, structured skips, labelled spans [carries F2]

**Problem.** F2: an agent cannot assert "site A and site B bound the
same `E`" from two 80-char prefixes, cannot fetch a full binding without
re-reading the target file, reads follow skips as a prose sentence, and
cannot tell three identical `[1,1]` spans apart.

**Decision — digest by default, bytes on request, structure everywhere.**
(1) Each binding value in `sites[].bindings` stays the 80-char display
string, and its site entry gains `binding_meta`: `{name: {len, sha256}}`
for every binding whose value was cut (absent when nothing was cut, so
uncut reports stay byte-identical to today). Length is in chars; the
digest is over the full untruncated value. Cross-site equality is now a
string compare on digests. (2) `applyrs --bindings full` (also via
`gitrs apply`) lifts the 80-char cut for that run only; the default
budget pin in SPEC D.2 is recomputed for the digest keys and the flag is
documented as the token-expensive path. (3) `follow_skipped` becomes a
structured array `[{file, line, reason}]` with `reason` one of
`shadowed`, `glob-reexport`, `second-hop`, `macro-hidden`,
`off-target` (the WI-14/WI-15 vocabulary); the `; `-joined string is
removed (D7: meaning-bearing shape change, carried by the WI-20 version
bump — until WI-20 lands, the array ships additive beside the string and
the string's removal is WI-20's). (4) `orphans` entries gain the same
`reason` codes. (5) Multi-edit spans (signature evolution) gain an
`edit` label per span (`generics`/`params`/`return`/`rename`/`follow`)
so identical `[s,e]` spans are distinguishable; single-edit rules omit
it. All keys additive under `version: 3` except (3)'s removal, deferred
to WI-20.

**Changes.** SPEC D.2 (digest keys, `--bindings full`, skip/orphan
reason codes, span `edit` labels; budget pin recomputed), 14.8 (skip
reason vocabulary). Code: digest + meta emission and the flag in
[applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
(`sites_json`/`orphans_json`); reason codes threaded from the collectors
in [use_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph.rs)
and `use_graph_member.rs` (new `follow_reasons.rs` sibling if a touched
file crosses 1600 lines, D5); span labels from
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
edit construction.

**Acceptance.** The long-binding probe reports equal `sha256` for both
sites and `--bindings full` returns the untruncated call; a
shadowed-field follow reports `follow_skipped: [{..., reason:
"shadowed"}]` in JSON (text mode prints the same triples, one per
line); the lifetime-evolution probe's three spans carry distinct
`edit` labels. Default-report overhead stays under the recomputed D.2
pin.

**Tests.** Digest fire/cut/absent pins; full-bindings round-trip;
skip/orphan reason matrix (one fixture per code); span-label pins for
each evolution slot; budget pin test updated, never loosened silently.

**Non-goals / risks.** No pagination protocol — `--bindings full` plus
digests covers the assert/fetch split at CLI scale. Risk: digest keys
inflate wide-sweep reports; bounded by emitting meta only for cut
values, pinned by the budget test.

### WI-3 (P1, S): Repair completeness — no patch failure without a next action [carries F3-repair]

**Problem.** F3: `RSMPL_RANGE_CONFLICT` and unknown-directive failures
print `repair: null`. The previous plan's rule — every new fence ships
a template — was never extended backwards over the existing error
surface, so the failures an agent hits first are the ones with no
machine exit.

**Decision — enumerate the surface, then close it.** (1) A conformance
test enumerates every `StructuredError` construction reachable from
patch input (all `ErrorCode` variants × the construction sites in
`rsmpl_patch`, `rsmpl_pattern`, `rsmpl_engine`, and the two CLI mains)
and asserts `repair != null` for every class except a named allow-list
(unreadable-file `RSMPL_IO_CONFLICT`, where the fix is environmental and
the suggestion already names the path — the allow-list lives in the
test, so a new null-repair error fails CI). (2) New producers:
range-past-EOF carries `fix_range` (`{file, file_lines, write_instead:
"@@ -<start>,<clamped-len> @@"}` — the largest in-file range starting
at the cited start, or the last line when the start itself is past
EOF); unknown directive/header-tag/`# root=` values carry
`fix_directive` (`{found, did_you_mean, write_instead}` — closest known
spelling by edit distance over the SPEC 9.3 table, `null` when nothing
is within distance 2, `write_instead` the full corrected directive
line); capture-name errors already carry `fix_name` and join the
enumeration. (3) Text mode prints each new payload as the trailing
`repair: {...}` line, same as existing producers.

**Changes.** SPEC D.2 (two new repair shapes; the "seven producers"
sentence replaced by the completeness rule), 25 (per-code repair
expectation column). Code: producers beside `fill_generic_repair` in
[validate.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/validate.rs);
range data threaded through `ensure_file_range`; directive table shared
from [directives.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/directives.rs)
(no second spelling list); conformance test in `bin/applyrs/tests/`.

**Acceptance.** Both F3 probes now carry applicable repairs (paste the
`write_instead` header/directive, retry, succeed — pinned end-to-end);
the enumeration test fails if any pinned producer is removed.

**Tests.** Per-producer JSON + text pins; edit-distance matrix
(`# alow=many` → `# allow=many`, `# scope=files` → `# scope=file`,
far miss → `did_you_mean: null`); enumeration conformance test.

**Non-goals / risks.** No prose-reason rewriting — reasons stay human
text; only `repair` must be machine-applicable. Risk: edit-distance
suggests a wrong directive for a deliberate new spelling — mitigated by
the distance-2 cap and `null` honesty.

### WI-4 (P1, M): Pattern-anchored drift — stale terse hunks apply once, with evidence [carries F4; extends SPEC 10.8]

**Problem.** F4: drift tolerance requires ` ` context anchors, so the
diet spellings (terse renames, bare pairs) fail on any line shift and
cost a re-author round, although the planner already proves "exactly
one match exists elsewhere" to build its `widen_scope` repair.

**Decision — the unique-match proof becomes the drift gate.** When a
ranged semantic hunk with `allow=one` (the default) finds zero matches
in its cited range, the engine retries once file-scoped with the same
pattern: exactly one match in the file → apply it and report
`"drift": {rule, cited: [s,e], actual: [a,b]}` exactly as SPEC 10.8
does for anchored hunks (text mode: the same `drift:` stderr line).
Zero or ≥2 file matches → the original `RSMPL_MATCH_CONFLICT` stands,
now always carrying the WI-3 repair. The anchored path keeps priority
(anchors, when present, must also agree with the unique candidate —
disagreement is the ambiguous case and fails). `# sweep`/`# scope=file`
hunks never drift-report (they made no locality claim); `allow=many`
ranged hunks keep today's behaviour (their range is a real claim about
a set). WI-1's postcondition gates drifted edits identically; overlaps
with other hunks are checked in original coordinates as today, so a
drifted edit colliding with a sibling hunk fails as overlap, never as
a silent merge.

**Changes.** SPEC 10.8 (pattern-anchored tier beside the context-anchor
tier; both emit identical `drift` evidence), D.2 (`drift` bullet covers
both tiers). Code: retry beside the T62 whole-file probe in
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs);
the existing whole-file count machinery is reused, not duplicated.

**Acceptance.** The F4 probe (terse rename 3 lines stale) applies with
correct `drift` evidence and a byte-identical post-tree; the ambiguous
fixture (two identical `fn foo` items — twin veto) fails with the
original conflict and applies nothing; an anchored hunk whose anchors
name the *other* candidate fails rather than drifting.

**Tests.** Drift matrix (terse/call/`let` shapes × unique/twin/zero;
anchor-agree/anchor-disagree; drift × overlap; drift × `# optional`);
`--check` vs apply parity; cycle fixture `51-drift-stale-range` keeps
its pinned shape (drift stays apply-side evidence, `S′` cites the
actual range).

**Non-goals / risks.** No fuzzy matching — the pattern still matches
semantically or not at all; only the search scope widens, once, under a
uniqueness proof. Risk: masking a genuinely wrong range — mitigated by
the exactly-one gate plus always-visible drift evidence (the agent sees
cited vs actual without asking).

### WI-5 (P0, M): Name holes — `N` binds the item name in fn/header patterns [enables WI-9, WI-18; carries F7-engine]

**Problem.** F7: slots abstract everything about a fn header except the
one part a slot-only sweep needs to vary — the name. Without a name
hole, "evolve the headers, keep the names" is inexpressible, so the
most mechanical header migrations (visibility, bounds, qualifiers) fall
back to per-function ordinary hunks.

**Decision — one inferred letter, one explicit kind, fn/header positions
only.** (1) Bare `N` in a fn pattern's name position (and in
impl/trait-context method pairs) infers as a *name hole*: it binds the
site's ident text per site (SPEC 15.7 equality governs repeats), and on
the plus side `N` replays the bound name while the written slots evolve
around it — `- V Q fn N(...) {` / `+ pub fn N(...) {`. Writing a
different literal name on the plus side beside an `N` minus is the
existing concrete-rename shape and stays distinct. (2) Explicit form
`# ident N` declares the kind for ambiguous positions (the `identifier`
rejection in SPEC 15.1 is narrowed to exactly this kind name — agents
meet `# ident` next to `# expr`/`# pat`, spelled as Rust names the
concept). (3) `N` never binds qualified paths, never appears in
call/method callee positions (those keep their backtick/literal rules),
and a plus-side `N` with no minus-side `N` is unbound (`fix_pattern`
repair, code `unbound`). (4) Inference: `N` joins the diet letters
(`E T P V Q G W I` → `E T P V Q G W I N`) only in name position — a bare
`N` elsewhere stays a literal ident, so no existing patch changes
meaning.

**Changes.** SPEC 15.1/15.8 (name-hole kind + inference position), 14.1/
14.2 (name position in the slot table), 18.6 (new C-row for misplaced
`N`). Code: kind + inference in
[metavar.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/metavar.rs)
and [fn_pattern.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/fn_pattern.rs);
binding + replay in
[matcher_fn.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_fn.rs)
(sibling module if it crosses 1600 after the change, D5).

**Acceptance.** The authored sweep `- V Q fn N(...) {` /
`+ pub fn N(...) {` under `# sweep` adds `pub` to four differently-named
fns; a fifth, already-`pub` fn also matches (`V` binds `pub`, the
plus replays it) and is reported in `matches`/`spans` with a
byte-identical edit that contributes no diff lines — and if *every*
site is already in the plus state, the existing sweep no-op gate
fails loud as today; a bound-adding sweep (`fn N<T>(...)` →
`fn N<T: Clone>(...)`) fires; `N` in a call callee fails the new C-row
at compile time with a template.

**Tests.** Name-hole matrix (bind/replay, repeat equality across two
`N` occurrences, unbound-plus veto, misplaced-`N` C-pin, literal-`N`
elsewhere unchanged); impl-context method pair with `N`; diet-vs-declared
canonicalisation pin (`# ident N` drops like other declarations).

**Non-goals / risks.** No item-name holes for `struct`/`enum` bare pairs
in this item (WI-15's grouped raising reuses `N` if it proves out
there; the bare-pair rename keeps concrete names so canonical forms
stay stable). Risk: single-letter inference creep — contained by the
name-position-only rule and the C-row fence.

### WI-6 (P0, L): General-expression raising — body rewrites compress [carries F5]

**Problem.** F5: the commonest diff in real code — the same expression
edit inside several bodies — has an authored form and no detector. Every
such change pays ordinary bytes and, worse, the agent reviewing a
`gitrs diff` output cannot see that three sites share one intent.

**Decision — anti-unify the changed subterm, frame stays concrete.** New
detector `general_expr`: N ≥ 2 balanced pairs whose changed lines each
hold exactly one changed expression subterm at the same structural
position, where the per-site subterm pairs anti-unify to one template
pair via the shared core in
[antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
— hole positions are subterms token-equal within each pair but hetero
across sites (`E0…`), literal anchors are tokens equal across all sites
on each side (the operator, the changed literal), and at least one
anchor carries the delta. Whole-arg permutation is the same core's
call-position case: `swap(a, b)` → `swap(b, a)` anti-unifies as two
whole-arg holes exchanged (SPEC D.4 already promises this; the
implementation's per-position same-hole assumption is the bug F6(a)
pins, fixed here, not re-specified). Emission is one `# allow=many`
general hunk (`- E0 + 1` / `+ E0 + 2`, diet holes, tight range or
`# sweep` per the D.5 64-line rule). Totality is the real-pair engine
probe (`template_pair_holds`): the emitted pair must match exactly N
subterms in the pre-image — an untouched twin (`4 + 1` the diff did not
touch) vetoes to ordinary. Macro-hiding subterms veto (the
WI-9b2-delim precedent: TokenStream-normalised text is unmatchable).
Chain placement: after `call_arg_template`/`method_arg_template`,
before verbatim — tighter call templates win; verbatim keeps
all-identical classes.

**Changes.** SPEC D.4 (`general_expr` detector bullet; the call-arg
bullet's permutation sentence now true in code). Code: generalisation
helpers in `antiunify.rs` (expression-position anti-unification reusing
`antiunify_plus_with_hole`); detector in new
`crates/rsmpl_diff_infer/src/semantic/general_expr.rs`; chain
registration in
[chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs);
provenance name in
[prov.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/prov.rs).

**Acceptance.** The F5 binary-op probe compresses and real-applies
byte-identical (the guard/closure/await probes are WI-8's acceptance,
not this item's); fixture `60-call-arg-swap` moves to `CYCLE_OK`;
a hetero-distractor fixture (third site `4 + 9` untouched by the diff
but matching the minus template) vetoes to ordinary. New diffrs bench
35 (binary-op migration, 5 sites) ≥ 50% saved; history compressing set
grows or the landing change names why a probed class declined.

**Tests.** Anti-unify matrix (binary/unary/field/index/cast, hole-left/
hole-right, permute fire, duplicate-hole veto, macro-subterm veto);
totality twin veto; fixtures round-trip real-apply; cycle harness.

**Non-goals / risks.** No statement-level generalisation here (WI-8 owns
control/statement positions), no multi-subterm-per-line classes (the
sub-line driver owns repeated same-family subterms). Risk: over-broad
templates (`E0 + E1` matching everything) — the ≥1-changed-anchor +
real-pair totality gates are acceptance criteria, not aspirations.

### WI-7 (P1, M): Literal-frame raising — string/number/bool edits keep their frame [carries F5-literal]

**Problem.** Identical literal edits (`"old"` → `"new"`, `10` → `20`,
`false` → `true`) inside hetero frames stay ordinary: the verbatim sweep
deliberately vetoes bare literals ("would match every mention"), and no
other detector owns a lone literal delta. Config strings, log messages,
and feature names — a daily edit class — never compress, although the
authored general form (`- "old"` / `+ "new"` under `# sweep`) applies
today (F1's probe) and the frame around each site is byte-identical
evidence the literal is the changed one.

**Decision — the frame is the anchor, occurrence-counted like sub-line
sweeps.** New detector `literal_frame`: N ≥ 2 pairs where each changed
line differs from its partner in exactly one literal token (string,
char, number, bool — same literal kind on both sides), the surrounding
line frame (text before + after the literal, per site) may differ
across sites, and the (old-literal, new-literal) pair is uniform across
sites. Emission is one `# allow=many` general hunk with the literal
pair (`- "old"` / `+ "new"`). Totality reuses the sub-line
occurrence-counting discipline (SPEC D.4): count occurrences of the old literal as an expression
subterm in the pre-image (engine-count probe over `syn` literals, macro/
string interiors opaque and vetoing only when they cover a changed
line, per `totality.rs`); the count must equal N. A same-literal
occurrence the diff left untouched vetoes — frame heterogeneity is why
the *count*, not frame equality, carries the proof. T375 guard as
usual; single-site literals stay ordinary.

**Changes.** SPEC D.4 (`literal_frame` bullet; verbatim bullet gains a
"lone-literal deltas are `literal_frame`'s" cross-reference). Code:
detector in new `crates/rsmpl_diff_infer/src/semantic/literal_frame.rs`
(the existing `literal.rs` owns trailing-arg literals and is not grown,
D5); literal-occurrence counting beside the sub-line counters in
`totality.rs`.

**Acceptance.** The F5 string probe (three `"old"` returns) compresses
and real-applies byte-identical; a numeric probe (`MAX`-style `10` →
`20` in three hetero lines) fires; an untouched-twin fixture (fourth
`"old"` the diff did not change) vetoes to ordinary; a string *inside*
a string (literal text mentioning `"old"`) never counts. New diffrs
bench 36 (log-message rename, 4 sites) ≥ 45% saved.

**Tests.** Literal-kind matrix (string/char/int/float/bool, kind-mismatch
veto); occurrence-count fire/veto; macro-interior cover veto; fixtures
+ cycle harness.

**Non-goals / risks.** No literal holes (a varying literal is data, not
a template), no raw-string/`concat!` interiors (opaque, as today).
Risk: tiny literals (`0` → `1`) occur everywhere — the count-must-equal-N
gate plus T375 keeps the detector silent unless the diff truly owns
every occurrence; pinned by a dense-`0` veto fixture.

### WI-8 (P1, L): Control and untyped-let templates — guards, closures, await chains, destructuring inits [carries F6(b–d)]

**Problem.** F6(b–d): the engine matches match guards, closure bodies,
`if`/`for` scrutinees, `await`/`?` chains, and WI-3 destructuring
patterns structurally, but raising templates stop at typed `let`s and
bare calls. Diffs that change a guard comparison, bump a closure body,
or wrap an untyped destructuring init stay ordinary, and three fixtures
sit pinned `nodet` for exactly these shapes.

**Decision — one template family over the control positions the engine
already owns.** (1) *Untyped let-template*: the joint let detector loses
its ascription requirement — `let PAT = INIT;` pairs where PAT
anti-unifies via `gen_pat` (including `_` and WI-3 composites) and INIT
anti-unifies via the WI-6 core (including `Expr::Const` and
`Expr::Infer` subterms, which `gen_init` currently vetoes) emit
`- let P0 = E0;`-family hunks with the same real-pair totality. This is
fixture `57`/`68`'s shape; typed classes keep firing in the existing
detector first (chain order unchanged, disjoint by ascription).
(2) *Control templates*: new detector `control_template` for N ≥ 2 pairs
whose changed subterm sits in one uniform control position — match-arm
guard, arm body, closure body, `if`/`while` condition, `for` iterable,
`let`-else diverge expression, or an `await`/`?` base — anti-unified by
the WI-6 core inside that position, emitted as the smallest general
hunk that names the position (`- E0 + 1` suffices where the subterm
template already matches only there; where the bare subterm would match
wider, the emission carries its control frame, e.g. the arm pattern,
exactly as the single-block detector frames its windows). Totality is
the real-pair probe scoped to the position (guard twins outside guards
never count). (3) Tuple-pattern swaps ride (1): `gen_pat` gains the
tuple-position case the swap fixture needs (positions exchange, holes
correspond per SPEC 15.7).

**Changes.** SPEC D.4 (`control_template` bullet; let-template bullet
splits typed/untyped), 18.6 (no engine rows move — detectors only).
Code: `gen_pat`/`gen_init` extensions in `antiunify.rs` (tuple exchange,
const/infer subterms); untyped arm in
[generalized.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/generalized.rs)
(`try_let_template_sweep` keeps its name; the untyped sibling lives in
new `let_untyped.rs` if the file crosses 1600, D5); control detector in
new `crates/rsmpl_diff_infer/src/semantic/control_template.rs`.

**Acceptance.** Fixtures `53-const-block`, `55-infer-wildcard`,
`56-tuple-pat-swap`, `57-some-unwrap`, `68-diet-some` each move to
`CYCLE_OK` or stay pinned with the sharpened reason the landing change
names; the F5 guard and closure probes compress and real-apply
byte-identical; a guard-template fixture whose same subterm appears in
a plain body expression vetoes rather than rewriting the body site.

**Tests.** Position matrix (guard/arm-body/closure/condition/iterable/
diverge/await-base × fire + out-of-position twin veto); untyped-let
matrix (destructuring, wildcard, const-block init, macro-init veto);
`gen_pat` tuple-exchange units; fixtures + cycle harness.

**Non-goals / risks.** No `Or`-pattern holes (C8 stands), no multi-arm
fusion (arms stay per-position templates), no closure *signature*
templates (params keep the existing pattern discipline). Risk: control
frames over-specify and lose byte wins — T375 decides per class, and a
frame that cannot beat ordinary declines.

### WI-9 (P1, M): Header slot-only evolution raising — one uniform slot delta, any names [carries F7-raising; spends WI-5]

**Problem.** F7: with WI-5's name hole the authored form exists, but
`diffrs` still shows four `pub` additions as four ordinary hunks. The
slot sweep demands a shared name delta; evolution demands the same.
Nobody owns "same slot change, different names".

**Decision — `header_evolution` detector over name-holed headers.** N ≥ 2
single-line fn-header pairs (free fns and, per enclosing block,
impl/trait methods) where names are hetero and preserved (minus name ==
plus name per site), and exactly one non-name slot carries a uniform
delta: visibility (uniform plus text or uniform removal), qualifiers,
generic params (one uniform template with per-site preserved holes
riding `T`-family metavars where the core can prove correspondence),
params (one uniform template over `E`-/`T`-holes), return type, or
where clause. Emission is one hunk per uniform class (per impl block,
verbatim headers as the slot sweep emits) in the WI-5 spelling:
`- V Q fn N(...) {` / `+ pub fn N(...) {` (hetero preserved slots ride
their existing wildcards under the same `V`/`Q`-pairing and
return/where spelling laws the sig-evolution detector already enforces
— shared emission helpers, not a second copy). Two slots evolving at
once, or a name delta anywhere in the class, stays with
`sig_evolution`/ordinary. Totality is the generic engine-count check:
the emitted minus header (with `N`) must match exactly N headers in the
pre-image — a same-shape header the diff left alone vetoes.

**Changes.** SPEC D.4 (`header_evolution` bullet; sig-evolution bullet
gains the disjointness note). Code: detector in new
`crates/rsmpl_diff_infer/src/semantic/header_evolution.rs`; shared
header-emission helpers extracted from `generalized.rs`'s sig-evolution
section into `block_headers.rs` (existing sibling, D5) rather than
duplicated.

**Acceptance.** The F7 probes compress (four `pub` adds → one hunk; two
`: Clone` bound adds → one hunk) and real-apply byte-identical; new
applyrs bench 29 (visibility sweep) and diffrs bench 37 (bound sweep)
each ≥ 50% saved; a mixed fixture (three `pub` adds + one `async` add)
splits into two uniform classes under two rules (a class left at
N=1 by the split stays ordinary), never half-claimed under one rule.

**Tests.** Slot matrix (vis/qual/generics/params/return/where ×
add/change/remove); name-delta exclusion; impl-block per-header
emission; twin veto; cycle coverage for the authored WI-5 forms.

**Non-goals / risks.** No attr-slot evolution here (WI-10 owns attrs),
no body evolution (bodies stay `...`-preserved), no cross-block method
classes (per-block hunks, as today). Risk: `N` matches headers the diff
did not intend (e.g. adding `pub` inside a trait, where `pub` is
illegal) — trait-context classes reject vis deltas at emission, pinned.

### WI-10 (P1, M): Attribute evolution — write the attr delta where Rust writes it [carries F9-attrs]

**Problem.** F9: `#[derive]`, `#[cfg]`, `#[inline]`, `#[allow]`, and doc
lines are the highest-frequency item-adjacent edit in Rust, and RSmPL
can only preserve or strip them. Agents hand-write ordinary hunks for
attribute migrations and `diffrs` hands the same bytes back.

**Decision — written attr lines are a delta; unwritten attrs stay
preserved.** (1) *Authoring.* On fn/item/`let` shapes, attr/doc lines
written in the pattern are matched exactly as today; additionally, an
attr line present only on the plus side *adds* it, only on the minus
side *removes* it, and present on both sides with different text
*replaces* it — per attached line, in written order, above the item
line they attach to. C11/C12 narrow to the ambiguous remainder (an attr
delta beside `# attrs=explicit`, or a plus attr set that partially
overlaps the minus set without a one-to-one line correspondence) with
updated write-instead text. `# attrs=preserve` (default) now means:
unwritten attached attrs preserved, written lines are the delta.
`# attrs=absent`/`strip` keep their meanings; a delta under `strip`
fails C11 as today (contradictory instructions fail loud). (2) *Raising.*
New detector `attr_sweep`: N ≥ 2 item-header pairs (or pure-insertion
groups) sharing one uniform attr-line delta (add/remove/replace, same
attr text) over hetero items emits one hunk in the (1) spelling with
the item headers as concrete context (name-holed `N` form when the
class is header-uniform, concrete headers otherwise). Totality counts
items carrying the minus-side attr state in the pre-image; an untouched
item already in the plus state never vetoes an *add* (it cannot match
the minus), and an untouched item in the minus state always vetoes.

**Changes.** SPEC 14.6 (written-line delta semantics; C11/C12 narrowed),
18.6 (C11/C12 rows updated — text moves with code), D.4 (`attr_sweep`
bullet), AGENTS.md row for `# attrs=`. Code: attr-delta matching in
[matcher_item.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_item.rs)
and the fn matcher (attr handling already centralised there — extended,
not forked); detector in new
`crates/rsmpl_diff_infer/src/semantic/attr_sweep.rs`.

**Acceptance.** Authored `#[inline]` addition sweeps four fns; authored
`#[derive(Debug)]` → `#[derive(Debug, Clone)]` replacement fires on
three structs; removal fires; the four-insertion F9 diff compresses
under `attr_sweep` and real-applies byte-identical. New diffrs bench 38
(derive migration, 4 items) ≥ 45% saved.

**Tests.** Delta matrix (add/remove/replace × fn/struct/`let` ×
doc-line/outer-attr); ambiguity vetoes (partial overlap, `strip`
conflict); raising fire/twin-veto; SPEC 14.6 example pins updated in
`spec_examples`.

**Non-goals / risks.** No inner attrs (`#![…]`), no attr *reordering*
as a delta (order is written order; a pure reorder stays ordinary), no
`cfg`-evaluation (attrs compare as text, as today). Risk: doc-comment
vs `#[doc]` spelling parity — matching normalises through the existing
attr rendering, pinned by a `///`/`#[doc]` twin unit.

### WI-11 (P1, L): Definition right-hand sides — type/const/static/field/payload evolution [carries F9-values-authored]

**Problem.** F9: a definition's *value* side — the `= …` of a type
alias, const, or static; a field's type; a variant's payload — has no
semantic spelling. Renames freeze the RHS token-equal, so evolving it
means ordinary hunks even when the change is a clean template
(`Vec<T>` → `Box<[T]>` over the alias and nothing else).

**Decision — context-pair evolution shapes in the member-rename family
(D6).** Five shapes reuse the header+pair spelling agents already know
from field/variant renames (SPEC 14.8): (1) *type-alias evolution*:
`- type A = Vec<T>;` / `+ type A = Box<[T]>;` — name concrete and
preserved, RHS a type template (`T` infers as today), trailing `;`
required. (2) *const/static evolution*: `- const MAX: i32 = E;` /
`+ const MAX: i32 = E + 1;` — name and ascribed type token-equal
preserved, value a general-expr template. (3) *field-type evolution*:
`struct S {` context with `- old: Vec<T>` / `+ old: Box<[T]>` — field
name preserved, type template evolves (the field-rename gate "same type
both sides" becomes "same field name both sides" for this kind; the
classifier distinguishes by which side of the `:` is identical).
(4) *variant-payload evolution*: `enum E {` context with
`- Old(Vec<T>)` / `+ Old(Box<[T]>)` (tuple payloads as type templates)
and `- Old { f: Vec<T> }` / `+ Old { f: Box<[T]> }` (struct payloads,
field names preserved). (5) Discriminants (`Old = 1` → `Old = 2`) are
an expr template on the discriminant, same context. Each classifies as
its own `PatternKind` (`TypeAliasEvolution`, `ConstEvolution`,
`FieldTypeEvolution`, `VariantPayloadEvolution`) so report spans,
WI-1 postconditions (structural re-match of the evolved RHS at the
definition span), and T375 accounting come along without new directives.
`# follow=uses` is rejected on evolution kinds (C17 extension): evolving
a type does not rename anything, and silently following would guess at
conversion edits the patch never stated.

**Changes.** SPEC 14.8 (evolution sibling table beside the rename
table), 13.4 (five parse contexts), 18.6 (C17 extension row). Code:
classification in
[pattern_classify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/pattern_classify.rs);
matching in the existing
[matcher_member.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_member.rs)
extended with RHS template arms (new `matcher_value.rs` sibling if it
crosses 1600, D5); RHS template compilation reuses the type/general
pattern compilers — no new template language.

**Acceptance.** All five authored shapes apply with bodies/suffixes
preserved byte-exact beside the evolved RHS; `- type A = Vec<T>;`
fires (today: pattern conflict); a name-changing "evolution"
(`type A` → `type B` with RHS delta) fails fast naming the rename
spelling instead; WI-1 reports each as `instantiated`.

**Tests.** Per-shape matrix (fire, suffix/discriminant preservation,
name-change veto, follow-rejection C-pin); type-template depth (nested
`Vec<Vec<T>>`); const-expr template with `E` repeated (SPEC 15.7
equality veto).

**Non-goals / risks.** No generic-param evolution on aliases/fields in
this item (headers keep today's preserve posture), no `where`-clause
evolution on items, no value conversion at use sites (ordinary hunks or
WI-6 templates own those). Risk: classifier ambiguity between
field-rename and field-type-evolution when *both* sides differ — both
differ fails fast naming both spellings, never guesses.

### WI-12 (P1, M): Definition-value raising — field types, payloads, alias/const values compress [carries F9-values-raising]

**Problem.** F9 (raising side): field-type changes, payload changes,
field/variant additions, and alias/const value changes stay ordinary
even at N sites, so the diff view hides that one definition changed and
its constructions followed.

**Decision — definition-first detectors with construction follow-through
only by probe.** (1) *Single-definition evolution*: one definition pair
matching a WI-11 shape (alias RHS, const/static value, field type,
variant payload, discriminant) compresses to that shape under the
*precision guard* this item introduces for definition shapes: fire
whenever the semantic form costs at most `ordinary_bytes + 16` (the
SPEC D.4 constant WI-19 later generalises to all definition-shape
detectors, with its pin test written here first).
(2) *Field/variant addition*: one definition-add pair (`S` gains
`b: bool`; `E` gains `C`) plus construction-site pairs whose only delta
is the added label/arm (`S { a: 1 }` → `S { a: 1, b: true }`,
`E::A => 1` gaining `E::C => 3`) emits the definition evolution hunk
plus one verbatim-class hunk per construction shape — but only when a
scratch-apply probe proves the pair reproduces exactly the changed
lines (the `member_rename` probe discipline). Heterogeneous added
values (`b: true` at one site, `b: false` at another) are per-site
concrete and still probe-proven; a construction the probe cannot
reproduce stays ordinary and the definition hunk still fires alone.
(3) Chain placement: after `member_rename`, before the method sweeps
(disjoint by shape: `:`/payload lines vs call lines).

**Changes.** SPEC D.4 (`definition_evolution` bullet). Code: detector
in new `crates/rsmpl_diff_infer/src/semantic/definition_evolution.rs`;
construction probe reuses the scratch-apply helper from `member.rs`
(extracted to `move_parts.rs`-style shared sibling if needed, D5).

**Acceptance.** The F9 raising probes compress: field-type `u32`→`u64`
(definition hunk), payload `A(i32)`→`A(String)`, field-add with two
constructions (definition + probed constructions), variant-add with
match-arm addition. A field-add fixture whose third construction uses
`..Default::default()` (no explicit label to add) keeps that site
ordinary while the rest compress — pinned, never half-rewritten.

**Tests.** Per-shape fire/probe-veto matrix; heterogeneous added-value
fire; `..`-rest construction partial pin; T375/guard accounting pins;
fixtures + cycle harness.

**Non-goals / risks.** No field *removal* with construction cleanup in
this item (removal orphans uses it cannot prove — ordinary + orphan
evidence), no tuple-struct positional payload addition (labels are the
anchor; positional stays ordinary). Risk: addition probes over-fire on
coincidental label matches — the scratch-apply exact-line proof is the
gate, stated here so a probe-less emission cannot land.

### WI-13 (P2, L): Associated items — trait/impl `type`/`const` rename, evolution, and `Self::` follow [carries F9-assoc]

**Problem.** F9: associated types and consts are half of every trait's
surface, and neither direction can name them. Renaming `Item` → `Output`
across a trait, its impls, and `Self::Item` uses is per-line ordinary
work today, although the trait-context spelling family already exists
for methods.

**Decision — assoc shapes in the trait/impl context family.** (1)
*Rename*: trait or impl context header plus a bare assoc pair —
`type Item;` → `type Output;` (trait decl, suffix/generics preserved),
`type Item = i32;` → `type Output = i32;` (impl, RHS preserved
token-equal), `const MAX: i32;`/`= 10;` likewise (name-span edit only).
(2) *Evolution*: RHS/type templates in the WI-11 discipline under the
same contexts (`type Item = Vec<T>;` → `type Item = Box<[T]>;`).
(3) *Follow*: `# follow=uses` on an assoc rename rewrites `Self::Old`,
`S::Old`, `<S as Tr>::Old`, and `obj::Old`-free path tails in expr/type/
pattern positions that name the trait's assoc item — proven by the
context header (the trait is named; impls of *that* trait are the
follow domain, enumerated like WI-4's per-block hunks). A second trait
defining the same assoc name vetoes only its own blocks (WI-2 reason
`shadowed` per skipped block, not a whole-run failure — the D8 rule).
(4) *Raising*: detector `assoc_rename` joins a trait-decl pair, its
impl pairs, and `Self::` use lines sharing one ident delta into the
(1)+(3) emission, totality by scratch-apply probe as in WI-12.

**Changes.** SPEC 14.8 (assoc rename/evolution rows + follow domain),
13.4 (assoc parse contexts), D.4 (`assoc_rename` bullet). Code: kinds
in `pattern_classify.rs`; matching beside trait-method matching in
[matcher_impl.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_impl.rs)
(new `matcher_assoc.rs` sibling — `matcher_impl.rs` is not grown past
its current size without the split, D5); follow collector arm in
`use_graph.rs` sibling `use_graph_member.rs` pattern; detector in new
`crates/rsmpl_diff_infer/src/semantic/assoc.rs`.

**Acceptance.** Authored assoc rename + follow rewrites trait decl, two
impls, and `Self::Item` return types in one patch; without follow,
`orphans` lists every `Self::` site (named evidence, never hidden and
never filtered away by a reason code). Raising probe (trait + 2 impls + uses)
compresses and real-applies byte-identical. New diffrs bench 39
(assoc-type rename, 5 sites) ≥ 55% saved.

**Tests.** Decl/impl/use matrix (generics on assoc type preserved,
GAT-style `type Item<'a>;` suffix preservation, const value preserved);
same-name-other-trait partial-skip pin; raising probe veto (an impl of
another trait in the class vetoes to ordinary); cycle coverage.

**Non-goals / risks.** No assoc *fn* work (methods own it), no
cross-trait assoc unification, no default-RHS evolution in trait decls
beyond template parity with (2). Risk: `Self::` is also valid for
inherent impls — inherent `type`/`const` in `impl S` without a trait
stays an ordinary definition (no trait header, no follow domain),
pinned.

### WI-14 (P1, L): Type-anchored member follow — proven receivers rewrite, siblings become evidence [carries F8]

**Problem.** F8/D8: member follow scans by ident globally, so an
unrelated `B::timeout` vetoes `A::timeout`'s rename outright. The code
that would prove ownership — param types, annotations, literal paths —
is already in the file and ignored.

**Decision — anchor classes in proof order, vetoes demoted to evidence.**
Member follow (field + variant) classifies each candidate site by the
strongest anchor that proves it, rewriting only proven sites:
(1) *literal-path*: `A { timeout: … }`, `A { timeout, … }` (shorthand
expands as today), `A::Variant` constructions/patterns — the path names
the container. (2) *signature*: receivers/values whose type the
enclosing fn signature states (`fn f(a: A)` → `a.timeout`;
`-> A` construction-free returns do not prove field receivers and are
not claimed). (3) *annotation*: `let x: A = …;` / `let A { … }` bindings
whose later `x.timeout` uses fall in the same fn body after the binding
(syntactic, flow-insensitive beyond order — stated in SPEC, pinned by a
rebinding-shadow veto: a later `let x: B` re-anchors subsequent uses).
(4) Everything else same-name is `follow_skipped` with reason
`shadowed` (another container defines the name) or `off-target`, never
a run failure. The shadowed-definition `RSMPL_SYMBOL_CONFLICT` is
deleted for member renames: the definition hunk always applies;
`# fallback=syntax` keeps its meaning from fn follows (skip, don't
fail) and is no longer needed to survive a sibling definition. Variant
follow gains the same anchoring (`E::Old` is already literal-path;
bare imported `Old` requires a `use E::*` anchor line naming `E`,
else evidence). Raising (`member_rename`) keeps its scratch-apply
probe, which now validates against the anchored follow.

**Changes.** SPEC 14.8 (anchor classes + proof order; shadow-veto
paragraph replaced), D.2 (skip reasons already structured by WI-2).
Code: anchored collectors replacing the ident-global scan in
[use_graph_member.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph_member.rs);
signature/annotation anchor pass in new
`crates/rsmpl_engine/src/use_graph_anchor.rs`; orphan collection in
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
consumes the same classification (one classifier, two consumers).

**Acceptance.** The F8 probe renames `A::timeout` (definition, literal,
and `a.timeout` rewritten; `b.timeout` reported skipped/`shadowed`,
run green without `# fallback=syntax`); a same-file `B`-typed local
with an interleaved rebinding fixture proves the shadow veto; new
applyrs bench 30 (anchored field rename with sibling struct) passes
with the sibling byte-untouched.

**Tests.** Anchor matrix per class (fire, rebind veto, return-type
non-claim, shorthand, pattern labels); variant `use E::*` anchor
fire/unanchored-evidence; `member_rename` raising parity fixtures;
cycle harness (`42`/`43` stay `CYCLE_OK`).

**Non-goals / risks.** No cross-fn dataflow, no method-call return-type
inference (`make_a().timeout` without annotation is evidence, not a
rewrite — stated, pinned), no field follow through `Deref`/macros
(opaque, as today). Risk: annotation anchoring over-claims after
shadowing — the order+rebind rules above are the whole discipline, and
each is pinned before the collector ships.

### WI-15 (P1, L): Cross-file intents — item groups, member follow across files, `mod` + file renames [carries F13]

**Problem.** F13: real renames cross files and contexts at once — a
struct and its impl in another file, a field whose uses live in a
second module, a `mod` whose file moves with it. Each fragment raises
alone (or not at all), so the agent gets ordinary hunks for the joints
of the change, exactly where intent matters most.

**Decision — group what one ident delta joins, probe every joint.**
(1) *Item-intent raising*: detector extension joining a definition pair
(`struct S`/`enum`/`trait`/`type`/`fn`), its `impl S`/`impl Tr for S`
header pairs, and same-delta type-position lines (`s: S`, `-> S`,
`Vec<S>`) across files into one grouped intent: definition hunk with
`# follow=uses` when a scratch-apply of the authored follow reproduces
exactly the group's changed use lines (the scratch-apply probe discipline from member renaming, now
cross-file), else definition-only hunks per file with uses ordinary —
never a follow the probe did not prove. Group naming reuses
`rename_Old_to_New` (WI-7 grouping, per-group T375 re-basis unchanged).
(2) *Member cross-file*: `member_rename`'s probe extends across files
(anchor classes per WI-14, evaluated per file); SPEC D.4's "cross-file
stays ordinary (future work)" sentence is deleted. (3) *`mod` + file
rename*: a `mod old;` → `mod new;` pair joined with a git rename
section (`old.rs` → `new.rs` / `old/mod.rs` → `new/mod.rs`) and
`old::` path-use lines emits the `mod` rename hunk, keeps the rename
section's file move as the native rename prelude (round-trips as
today), and adds a use-prefix sweep for the path uses when the probe
proves them — one intent, three coordinated hunks, each independently
totality-checked. Authored side needs no new syntax (`mod` rename +
use-prefix + file rename already compose); this item is raising +
grouping only.

**Changes.** SPEC D.4 (item-intent grouping paragraph; member bullet
loses its single-file limit; `mod_rename` bullet), 14.8 (follow probe
wording cross-file). Code: grouping extension in
[group.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/group.rs);
item-intent join in `chain.rs`'s partition neighbourhood (new
`item_intent.rs` sibling, D5); member probe widening in
[member.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/member.rs).

**Acceptance.** The F13 probe (struct + impl + typed param, one file)
and its two-file variant compress grouped and real-apply
byte-identical; a `mod`+file+uses fixture round-trips (apply the raised
patch to the pre-tree, trees equal including the renamed file); a
cross-file field rename with an unprovable receiver in the second file
emits definition+proven-uses and lists the rest as orphans. New diffrs
bench 40 (cross-file struct rename, 6 sites, 2 files) ≥ 55% saved.

**Tests.** Intent-join matrix (def-only, def+impl, def+uses-probed,
def+uses-unprobed); grouping T375 fallback pins; `mod` rename with
`mod.rs`-layout and flat-layout files; cycle coverage incl. cross-mode
canonical comparator (D.5).

**Non-goals / risks.** No crate-boundary following (workspace = the
repo, as today), no `pub use` chain joining beyond WI-16's hop proof,
no rename of inline `mod { … }` bodies' files (there is no file).
Risk: grouped hunks reorder sections definition-first (canonical form
already does); a group whose probe fails mid-way must degrade per-hunk,
never emit a follow that rewrites an unlisted file — pinned.

### WI-16 (P2, L): Crate-local `# symbol` resolution — prove the path, or stay syntax [carries F12]

**Problem.** F12: `# symbol` charges two directive lines and a binding
pair in every report for a text match the concrete path spelling already
provides. The promise — "the engine verifies `Old` resolves to this
path" — is unkept, so agents learn to avoid the directive that exists
for the highest-stakes renames (public API paths).

**Decision — a syntactic crate graph verifies crate-relative paths; nothing
else changes meaning.** (1) A new `rsmpl_engine` module builds a
crate-local resolution index from `syn` alone (no build, no cargo):
`mod` declarations (file and inline), item definitions per module, and
`use` re-exports (the existing one-hop follow rule is the starting
point; deeper hops are followed only while each hop's target is itself
proven in the index, with cycles cut). Paths are normalised `crate::`/`self::`/`super::`/
relative to module paths. (2) A `# symbol Old=crate::a::b` hunk *without*
`# fallback=syntax` now verifies: `Old`'s seeded path must resolve to a
definition in the indexed crate, and each matched call/qualified site's
spelled path must resolve to that same definition — sites that spell a
*different* definition with the same text (a vendored submodule copy)
do not match and are counted as opaque for totality. Unresolvable paths
(outside the crate, `std::`, dependency paths, glob-imported names)
fail `RSMPL_SYMBOL_CONFLICT` with a WI-3 `repair` naming the concrete-path
spelling as the `write_instead` — the honest fallback becomes a choice
the agent makes by writing `# fallback=syntax`, never a silent downgrade.
(3) With `# fallback=syntax`, behaviour is byte-identical to today
(exact text), and the report's per-rule `semantics` stays
`postcondition`/`instantiated` by WI-1 — the *resolution* fact is
reported as a new additive rule key `symbol: "resolved" | "syntax"`
(absent on non-symbol hunks), so an agent can require `"resolved"` for
API renames. (4) Raising never emits `# symbol` (concrete paths stay
canonical, SPEC 19.3 unchanged); canonicalisation keeps normalising
single-pair symbol hunks to concrete form (D.5 paragraph unchanged).

**Changes.** SPEC 19 (resolution index + verified posture; 19.3's
"prefers concrete paths" gains "unless resolution proof is the point"),
25.5 (failure modes), D.2 (`symbol` rule key). Code: index in new
`crates/rsmpl_engine/src/crate_graph.rs`; verification hook in the
symbol gate beside the existing exact-text matcher; hop-walk shared with
[use_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph.rs)
helpers (extracted, not duplicated, D5).

**Acceptance.** A same-crate symbol rename verifies `resolved` and
rewrites only the proven definition's sites (a same-text vendored copy
untouched, pinned); a `std::` symbol without fallback fails with the
concrete-path repair; with fallback it applies as today with
`symbol: "syntax"` — never `"resolved"`. History dogfood: at least one
real `# symbol` bench (16-symbol-sweep) reports `resolved` in its
authored form or the landing change names why its path is out of index
scope.

**Tests.** Graph units (file/inline mods, `self`/`super`, one-hop and
chained re-exports, cycle cut, glob opacity); verification fire/veto
(same-text-different-def, unresolvable, dependency path); report-key
pins; cycle harness.

**Non-goals / risks.** No dependency/`std` resolution, no rust-analyzer,
no expansion-derived names (macro-defined items are opaque to the
index — a symbol whose definition hides in a macro stays
`SYMBOL_CONFLICT` with the concrete-path repair, stated in SPEC).
Risk: index build cost per apply — built once per workspace search,
only when a symbol hunk is present; the WI-20 scale gate pins it.

### WI-17 (P2, M): Macro raising depth — token-tree holes and `macro_rules!` bodies [carries F10(a,c)]

**Problem.** F10(a,c): the authored token-tree hole form
(`old_m!(E, 1)` → `new_m!(E, 1)`) is what an agent wants from a macro
migration, but re-diffing it produces a whole-body `...` sweep that
hides which argument changed; and edits *inside* a `macro_rules!` body
(one token inside a rule arm) compare whole and stay ordinary.

**Decision — emit the hole the diff proves; descend exactly one token
level inside rule bodies.** (1) *Hole raising*: the macro-interior
detector gains a holed emission beside its concrete/verbatim emissions:
when same-name (or one-delta rename) macro pairs share arity and differ
in exactly one top-level comma-separated argument position while the
remaining arguments are token-equal across sites (stable positions stay
concrete; the differing position's per-site values ride one diet `E{i}`
hole), emit the `# macro=token_tree` holed hunk instead of the concrete
one. Totality is the real-pair probe over the holed pair (the concrete
fallback keeps its own probe; the tighter of the two that proves wins,
concrete first on ties — canonical stability). This is fixture `48`'s
shape. (2) *Rule-body interiors*: pairs whose enclosing line is a
`macro_rules!` rule arm and whose delta is one token-tree argument of
an invocation *inside* the arm body reuse the existing one-level
interior emission with the arm line as frame (the enclosing-macro
discipline applied to rule bodies). Depth stays one: tokens nested
inside that argument's own delimiters are compared whole, as today.
Engine authoring for both forms already exists (WI-7a holes; token-tree
matching) — this item is detectors plus cycle pins, no new matcher.

**Changes.** SPEC D.4 (macro-interior bullet gains the holed emission +
rule-body frame), 21.3 (raising posture note; engine text unchanged),
D.5 (`48` re-pinned — see Acceptance). Code: holed emission in
[macros.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macros.rs)
(arg-splitting helpers in existing `macro_parts.rs`, D5); rule-body
frame arm in `macro_enclosing.rs`.

**Acceptance.** Fixture `48-token-tree-arg-hole` moves to `CYCLE_OK`
(raised form canonicalises to the authored holed form); the
`inner!(1)` → `inner!(2)` rule-body probe compresses and real-applies
byte-identical; a depth-2 body edit (changed token inside a nested
delimiter) stays ordinary (veto pin); string-hidden macro text never
counts. Benches 17/18 hold or gain; no bench loses without a named
reason.

**Tests.** Hole-emission matrix (one differing arg, two differing args
veto to concrete/ordinary, rename+hole joint); rule-body fire/depth-2
veto; real-pair totality twins; fixtures + cycle harness.

**Non-goals / risks.** No recursive token descent beyond the one
argument level (previous plan's rejection stands — depth two invents a
second macro engine), no `macro` 2.0 bodies, no hygiene reasoning.
Risk: arg-splitting on unbalanced bodies — the existing string-aware
splitter is the only splitter; split failure declines to ordinary.

### WI-18 (P1, M): Collection cycle + evolving collections — many items move as one, names may follow [carries F10(b); completes SPEC 33.2's collection-evolution note]

**Problem.** F10(b): an authored collection move (three fns,
`old.rs` → `new.rs`, source emptied) does not re-raise in that form —
the re-diff fragments into ordinary additions and a paste-less cut, so
the cheapest trustworthy view of a bulk move is unavailable exactly
when bulk moves happen. Separately, SPEC 33.2 still lists per-item
evolution of collections as future work: moving five helpers *and*
renaming them costs the scalar fan-out the collection exists to avoid.

**Decision — fix the decline at its gate; evolve by name template only.**
(1) *Cycle fix, diagnosed first*: the landing change's first step is a
pinned reproduction of fixture `44`'s re-diff through the move
pre-pass, naming the gate that declines (empty-source section handling,
cell pairing across the emptied file, or the ≥3-item threshold counting
the wrong side). The fix targets that gate — candidate disciplines, in
preference order, are: treat a source file emptied to zero bytes as a
first-class move source (its section is a deletion of all items, not a
file deletion); count the collection cell on the *paste* side when the
source side is empty. Whichever gate it is, the fix is stated in SPEC
D.4's move bullet; no threshold is silently loosened (the ≥3 rule and
T375 stay). (2) *Evolving collections*: a collection cut
(`# bind=H` + `# allow=many`) may be pasted with a name-evolution
template — `+ fn N(...) { ... }` in the WI-5 name-hole spelling beside
`# use=H` generalises to: each cut item's header replays through the
template with `N` bound per item (names may be preserved, prefixed by
concrete text around `N` such as `new_N`, or suffixed — the template
is one header line; bodies, attrs, and remaining slots replay per item
exactly as scalar evolution pastes do). Kind-changing templates (fn →
non-fn) and per-item divergent templates fail
`RSMPL_CAPTURE_CONFLICT` with a WI-3 repair. Scalar evolution pastes
are untouched.

**Changes.** SPEC 20.3/20.4 (evolving collection pastes), 33.2 (future
entry removed), D.4 (move bullet's empty-source sentence), D.5 (`44`
re-pinned). Code: decline fix in
[moves.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/moves.rs)
(cell/scan helpers in `move_collection.rs`/`move_scan.rs`); paste-side
template replay in
[capture.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/capture.rs)
(collection store from the previous WI-6; new `capture_evolve.rs`
sibling if the file crosses 1600, D5).

**Acceptance.** Fixture `44-collection-move` moves to `CYCLE_OK`,
and whichever gate had declined it is named in the SPEC D.4 move
bullet either way (a silent re-pin is a failed landing); an authored five-item move +
`new_N` rename applies byte-identical in one cut + one paste and
re-diffs grouped; a kind-changing collection template fails with the
capture repair. Diffrs bench 31 (collection move) holds at ≥ 85% saved.

**Tests.** Empty-source move matrix (source emptied, source partially
emptied, file deleted outright stays file-deletion); evolve matrix
(preserve/prefix/suffix, kind-change veto, attrs preserved per item);
scalar move fixtures byte-unchanged; cycle harness.

**Non-goals / risks.** No statement collections, no body evolution
(bodies replay verbatim), no ordering syntax (source order, as today).
Risk: empty-file sections collide with file-deletion inference — the
preference order above keeps a true file deletion (section with
`deleted file mode`) distinct from an emptied-but-present file, pinned.

### WI-19 (P0, L): Precision-first cycles — safety shapes fire at N=1, partition degrades per-hunk [carries the `t375` mass + F11]

**Problem.** Roughly 70 `KNOWN_DIVERGENT` entries pin the same story: a
single-site rename/move/member/macro change re-diffs as ordinary
because the semantic spelling ties or loses by a few diet bytes, so the
agent loses the intent it authored. T375 prices bytes; it does not
price the difference between "a rename the engine can re-verify" and
"two lines that happen to differ". The strict guard made sense when
semantic forms were verbose; with diet spelling the margin is noise.
Independently, F11: one unclaimable pair vetoes a whole partition.

**Decision — two guards, priced differently; partition falls back, never
vetoes.** (1) *Precision guard*: definition-shape detectors — item/fn
rename (incl. slot/sig-evolution singles), member rename, assoc rename
(WI-13), `macro_rules_rename`, moves/deletes, and WI-11/12 definition
evolutions — fire at N=1 whenever the semantic form costs at most
`ordinary_bytes + 16` (a bounded, pinned constant: the price of the
`diff --git` + `@@ … rule=` + pattern lines the ordinary form does not
carry). Template/sweep detectors (calls, methods, verbatim,
`general_expr`, `control_template`, macro interiors — shapes whose
minus pattern could match wider than the diff) keep strict T375 at
every N: their risk is over-match, and bytes remain the honest proxy
there. The 16-byte bound is a SPEC D.4 constant with a pin test, not a
tunable discovered in code. (2) *Partial partition*: WI-8's partition
pass, when ≥1 pair is claimable and ≥1 is not, emits the claimable
pairs as semantic hunks and leaves the rest ordinary *in the same
section* (SPEC 10.6 disjointness already guarantees soundness), keeping
all-or-nothing only within a single native hunk (a hunk never splits).
Fixture `14` becomes 8 semantic + 1 ordinary; fixture `12`'s let run
compresses per-hunk while its header line stays ordinary. (3) Every
`KNOWN_DIVERGENT` entry the guard reclassifies moves to `CYCLE_OK` in
the same change; entries that remain (empty-diff `05`, hetero shapes
with genuinely no template) stay pinned with sharpened reasons. The
landing change reports the before/after counts in its description —
shrinking the table is the deliverable, not a side effect.

**Changes.** SPEC D.4 (two-guard discipline + 16-byte constant;
partition paragraph rewritten), D.5 (equalities unchanged; exclusion
list shrinks per (3)). Code: guard selection per detector in
[rsmpl_diff_infer lib](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/lib.rs)
(one `guard_for(detector)` decision point — detectors stop open-coding
T375 comparisons); partial-partition emission in
[chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs).

**Acceptance.** `KNOWN_DIVERGENT` shrinks by the full single-site
definition-shape family (the landing change lists the count; any
definition-shape single still pinned names why in its reason); fixtures
`14` (8+1) and `12` (let run semantic) re-pin as stated; history sweep
stays 0 failures and its compressing set grows; no sweep-class detector
fires at a byte loss (guard pins per class).

**Tests.** Guard matrix per detector class (fire-at-loss for definition
shapes, decline-at-loss for sweeps, 16-byte boundary pins); partial
partition (mixed claimable/unclaimable, single-hunk atomicity kept);
cycle harness with the moved entries; T375 pins for sweeps unchanged.

**Non-goals / risks.** No guard for insertions/ordinary shapes, no
per-detector private thresholds (one constant, one decision point).
Risk: single-site semantic forms cost a few more bytes per tiny change —
accepted by decision (D3 amended here): precision is the product;
token efficiency is served by the sweeps where fan-out lives. The risk
is bounded by the 16-byte constant and reported by the bench suite
(single-site benches may go a few points negative; the landing change
must show them and this plan pre-approves exactly that trade).

### WI-20 (P1, L): One contract — unified schema v4, linted documents, scale gates [carries F3-version + F14]

**Problem.** F3/F14: an agent must learn two envelope versions and trust
three documents nothing checks. Each is small; together they mean the
*interface* drifts independently of the engine it describes.

**Decision — one version constant, every fence linted, two scale pins.**
(1) *Unified schema*: `diffrs` success and error envelopes move to the
`applyrs` version; both binaries and the `gitrs` dispatcher take the
number from one `rsmpl_patch::REPORT_VERSION` constant (today each main
hard-codes its own). The unified number is `4` (D7: WI-2's
`follow_skipped` string removal rides this bump, its one breaking
change; everything else in this round was additive under v3). Text
modes are unchanged. `gitrs` handshake (WI-10 of the previous plan)
already rejects version-skewed siblings — it now also covers the schema
constant transitively, pinned by the dispatcher test. (2) *Document
lint*: a test extracts every `diff rsmpl`-tagged fence *and* every
untagged patch-shaped fence (heuristic: starts with `diff --git` or
`@@`, stated in the test) from SPEC.md, AGENTS.md, and
`skills/gitrs/SKILL.md`, canonicalises each (`rsmpl_patch::canonicalize`
must succeed and be a fixed point) and lints complete patches against
an empty repo (`no match found` = well-formed, the SPEC D.2.4 posture).
§29 runnable examples keep their behavioural pins in
`spec_examples.rs`; the lint is the wide net under them. A fence that
is deliberately invalid (error illustrations) carries an
`<!-- rsmpl:invalid RSMPL_PATTERN_CONFLICT -->` marker naming the
expected code — unmarked invalid fences fail CI. (3) *Scale gates*: two
pinned performance fixtures join the bench suite (not wall-clock
assertions in unit tests — recorded budgets in `example-benches/run.sh`
output, reviewed like byte tables): a `# sweep` + follow over a
500-file synthetic workspace, and WI-1 re-verification over a 10k-line
file with a 100-site sweep. Budgets are set from the landing machine's
measurement and only ever tightened deliberately. (4) SPEC gains a
one-screen "agent contract" preamble (the five invariants: original
coordinates, diet-canonical form, fail-loud, evidence fields, cycle
equalities) linking into the normative sections — no normative text
moves.

**Changes.** SPEC D.2 (version 4, single-constant rule), D.1 (handshake
note), front matter (contract preamble). Code: constant in
[rsmpl_patch lib](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/lib.rs);
version literals deleted from `bin/applyrs/src/main.rs`,
`bin/diffrs/src/main.rs`, `bin/gitrs/src/main.rs`; lint test in
`bin/applyrs/tests/spec_examples.rs` (grown into `doc_fences.rs` sibling
if it crosses comfortable size); scale fixtures under
`example-benches/`.

**Acceptance.** `applyrs`/`diffrs`/`gitrs` error JSON all report
`version: 4` for the same failure (key-parity pin extended tri-way);
every fence in the three documents lints clean or carries its
`rsmpl:invalid` marker (the landing change reports how many fences were
wrong and fixes or marks each); both scale fixtures complete within
their recorded budgets in `run.sh`.

**Tests.** Tri-way envelope parity; fence extraction + canonical
fixed-point + empty-repo lint; invalid-marker pins; dispatcher
handshake with a schema-skewed double; bench suite incl. new fixtures.

**Non-goals / risks.** No SPEC rewrite (preamble + cross-references
only), no wall-clock CI assertions (budgets are review artifacts, since
CI hardware varies — stated so nobody "fixes" a flake by deleting the
pin), no new CLI subcommands. Risk: fence heuristics catch prose that
looks patch-shaped — the marker mechanism is the escape hatch, and a
false positive costs one marker line, never a semantics change.

## Sequencing and gates

| Item | Depends on | Size | Gate before next item |
|---|---|---|---|
| WI-1 structural postcondition | — | M | String-bearing probe verifies; tampered-span test fails loud; benches 17/25/28 words pinned |
| WI-2 assertable evidence | WI-1 words | M | Digest equality probe; structured skips pinned; budget pin recomputed green |
| WI-3 repair completeness | — | S | Both null-repair probes carry templates; enumeration test green |
| WI-4 pattern-anchored drift | WI-3 repairs | M | Stale terse probe applies with `drift`; twin fixture fails clean |
| WI-5 name holes | — | M | `N` sweep authored probe fires; misplaced-`N` C-pin green |
| WI-6 general-expression raising | WI-1 | L | F5 probes compress byte-identical; fixture `60` cycles; bench 35 ≥ target |
| WI-7 literal-frame raising | WI-6 core | M | String probe compresses; dense-literal veto pinned; bench 36 ≥ target |
| WI-8 control + untyped-let templates | WI-6 core (existing `gen_pat`) | L | Fixtures `53/55/56/57/68` re-pinned; position twin vetoes green |
| WI-9 header slot-only raising | WI-5 | M | Vis/bound probes compress; benches 29/37 ≥ targets |
| WI-10 attribute evolution | WI-1 | M | Add/replace/remove authored probes fire; bench 38 ≥ target |
| WI-11 definition RHS authoring | WI-1 | L | Five shapes apply; both-sides-differ fails fast |
| WI-12 definition-value raising | WI-11 | M | F9 raising probes compress; `..`-rest partial pinned |
| WI-13 associated items | WI-11 contexts | L | Assoc follow probe fires; bench 39 ≥ target |
| WI-14 anchored member follow | WI-2 reasons | L | F8 probe green without fallback; rebind veto pinned |
| WI-15 cross-file intents | WI-14 anchors | L | Two-file + `mod`-rename fixtures round-trip; bench 40 ≥ target |
| WI-16 crate-local symbol resolution | WI-14 graph discipline | L | Vendored-copy veto pinned; `resolved`/`syntax` report pins green |
| WI-17 macro raising depth | — | M | Fixture `48` cycles; depth-2 veto pinned; benches 17/18 hold |
| WI-18 collection cycle + evolution | WI-5 templates | M | Fixture `44` re-pinned with its gate named; evolve probe byte-identical |
| WI-19 precision-first cycles | WI-12/13 detectors exist | L | Divergent table shrinks by the named family; sweep guards unchanged |
| WI-20 one contract | WI-1, WI-2 shapes settled | L | Tri-way v4 parity; all fences linted; scale budgets recorded |

Global validation per item: `cargo test --workspace`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`./example-benches/run.sh` (README tables regenerated only when bytes
change), and `python3 tools/history_sweep.py` (0 failures; compressing
set reported, never asserted to a fixed number beyond "does not shrink
without a named reason"). No item lands with a red gate, a new `.rs`
file over 2000 lines, or an existing file pushed over 2000 lines (D5).

## Risks / open questions

Per-item risks are stated with each item. Open questions: none —
scope, canonical forms, detector order, guards (64-line range rule,
16-byte precision bound, ≥3 collection threshold, one-level macro
descent, distance-2 directive suggestions), anchor proof order, schema
versioning (unified v4 in WI-20), and acceptance numbers are decided
here. If a threshold proves wrong against the probe corpora, the fix is
a new pin plus a SPEC note, not a silent constant edit. Three deliberate
non-goals bear repeating because they bound several items: no
type inference or trait solving anywhere (anchors are syntactic —
WI-14/WI-16 prove from text, never from `rustc`), no macro expansion
(SPEC 21.5 stands; WI-17 deepens rendering-level matching by one
argument level only), and no minus-must-be-zero postcondition (widening
rewrites legitimately re-match their own output — WI-1 verifies the
plus at its spans instead).

## Considered and rejected

- **Full type-directed resolution (rust-analyzer integration).** Would
  make follow and `# symbol` "really" correct, but couples every
  `--check` to a build environment, breaks the no-`cargo`-by-default
  cost model, and is un-auditable in a hackathon prototype. Syntactic
  anchors (WI-14) plus a crate-local graph (WI-16) buy the provable
  majority of the trust at text cost.
- **Dropping T375 entirely for token maximalism in reverse (always
  emit semantic).** Rejected: sweep-class templates whose minus matches
  wider than the diff are exactly where over-match risk lives; WI-19
  narrows the byte guard to the classes whose precision does not depend
  on it, with a bounded constant, instead of abolishing it.
- **General `...` in expressions/types/statements.** Rejected beyond
  the existing trailing-block and composite positions: unrestricted
  ellipsis turns precise patterns into guesses and breaks totality
  counting. WI-6/WI-8 generalise by anti-unification with anchors, not
  by ellipsis.
- **Recursive token-tree macro descent.** Rejected in WI-17 as in the
  previous plan: one argument level is provable with the existing
  token-tree matcher; deeper descent invents a second macro engine
  inside the first.
- **A separate `verifyrs` binary or always-on `cargo check`.** Rejected:
  trust must ride the existing `checked` envelope (WI-1) with build
  verification opt-in, or agents pay build costs on every rename and
  learn to skip the tool. `gitrs` stays a two-subcommand dispatcher.
- **CRLF/binary semantic compression.** Stays declared-lossy (D1):
  detectors strip `\r` by construction and patches are UTF-8;
  passthrough tree-losslessness is the correct contract there.
- **Prose-echo repairs for new fences.** Rejected (previous plan's rule,
  extended by WI-3 to the whole surface): every failure ships a concrete
  template or joins the named allow-list; the generic filler keys
  templates by error class, never copies the message.





