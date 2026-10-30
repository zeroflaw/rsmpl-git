# IMPROVE — RSmPL as a verifiable agent contract: true lossless cycles, per-site trust, and raising depth

## Goal

Make RSmPL the contract an agent can trust in both directions without
reconstructing and inspecting every edit by hand:

- **Lowering** (semantic → native, `applyrs` via `gitrs apply`) must prove,
  per site, what it touched, what it deliberately left alone, and what it
  checked — in machine fields an agent can assert on, not prose.
- **Raising** (native → semantic, `diffrs` via `gitrs diff`) must compress
  the refactor shapes real Rust code actually takes, at pre-image-proven
  precision, so the semantic form is the cheapest *trustworthy* view of a
  change, not merely the cheapest bytes.
- **Cycles must close under stated equalities**: semantic → native →
  semantic and native → semantic → native, with every remaining divergence
  named, owned, and pinned — never silent.

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
WI-1…WI-10 largely landed (see the carry-forward audit in Findings); this
plan is a fresh codebase review, not a re-issue of the old audit. Gaps that
survived, landed partially, or were exposed by the landed work are carried
forward explicitly per item. Work stops at 10 items.

## Success criteria

1. **Cycles (WI-1).** The `cycle_e2e` harness is green with the D.5
   exclusion list burned down to only the equalities this plan redefines
   (single-site demotion as a *defined* fixed-point equality, not a silent
   loss). Every remaining exclusion names its owning item below.
2. **Trust (WI-2, WI-7).** An agent can accept or reject a patch from the
   JSON reports alone — per-site bindings, postconditions, orphan
   evidence, and an honest `checked` breakdown — on every fixture,
   including one deliberately over-broad patch whose extra site is visible
   *before* apply. No success field is true by construction.
3. **Self-correction (WI-3).** The five highest-frequency failures carry
   machine `repair` payloads an agent can apply without parsing English,
   and every RSmPL example in SPEC §28/§29, Appendix A, AGENTS.md, and
   `skills/gitrs/SKILL.md` executes as written in CI.
4. **Raising breadth without precision loss (WI-4, WI-5, WI-8, WI-9).**
   `python3 tools/history_sweep.py` stays at 0 failures (today
   446/446 convert, 49 compressing) and the compressing set does not
   shrink without a named reason in the landing change. New detectors
   keep pre-image totality and real-apply byte-compare (T361 doctrine:
   `--check` alone never proves a sweep).
5. **Engine breadth (WI-5, WI-6).** The SPEC 18.6 bounded set grows only
   with structural, fail-loud arms; everything still rejected is rejected
   at compile time with an exact diagnostic, never as a silent
   `no match found` the agent must debug by reading the file.
6. **Gates.** `cargo test --workspace`, `cargo clippy --all-targets --
   -D warnings`, `cargo fmt --check`, and `./example-benches/run.sh` stay
   green per item; no `.rs` file exceeds 2000 lines (split before growing,
   D5); bench byte targets in each item are met or the item does not land.

## Review findings (evidence)

Verified this run at HEAD `21db81a` (446 commits): **1574 tests passing,
0 failed (125 suites); clippy and fmt clean; history sweep 446/446
convert, 0 failures, 49 compressing; benches 23 applyrs + 27 diffrs at
the values in [example-benches/README.md](/home/ai/git/tools/rsmpl-git/example-benches/README.md)**
(diffrs 17 at 46%, 18 at 41%, 21/22/23 at 81–87%, 27 at 81%). The
architecture is sound and stays: [rsmpl_patch](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/model.rs)
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
siblings. The gaps are cycle identity, evidence depth, and the remaining
veto surface — not correctness of what fires.

### Carry-forward audit of the previous plan

| Old item | Verdict | Evidence / residue |
|---|---|---|
| WI-1 canonical cycles | Landed, exclusions stale | `canonicalize` + `cycle_e2e` + SPEC D.5 exist; D.5 still blames WI-3/WI-4 for cases whose detectors have since landed (see F1) |
| WI-2 trust envelope | Landed, evidence shallow | `bindings`/`post_sha`/`verified`/provenance/`--verify build` present; bindings are first-match only and `verified` is true by construction (F2) |
| WI-3 paths/turbofish/macros | Landed | Benches 21–23 fire at 81–87%; `qself` and leading-`::` still vetoed in engine + inference (F6) |
| WI-4 anti-unification | Landed as 3 narrow slices | Let type-add, type, sig-evolution fire (benches 24–26); joint let templates and sequences still veto (F4) |
| WI-5 control flow | Landed | `If`/`Match`/loops/`Closure`/`Let`/`P @` match; async/try/unsafe/yield/macro-types and statement interiors still reject (F5) |
| WI-6 `# follow=uses` | Landed | Same-file + cross-file unique-definition following works; glob/macro/shadow veto, filesystem-walk workspace load (F7) |
| WI-7 grouping + renames | Landed | Shared `rename_Old_to_New` rules + rename-section content compression; single-delta only, headers-only renames passthrough (F8) |
| WI-8 move + evolve | Landed for `fn` only | Header-template paste splices `fn` bodies; every other item kind still needs byte identity (F8) |
| WI-9 multi-line / macro interiors | Macro half landed | Benches 17/18 at 46%/41% (target met); statement sequences never shipped — C18 still rejects them (F4/F5) |
| WI-10 executable contract | Partial | Splits + 2 `repair` producers landed; no test executes SPEC examples, §28 still teaches long spellings beside diet (F9) |

### Findings

- **F1. “Lossless” still has three silent dialects.** SPEC D.5 pins 19
  divergent corpus cases in
  [cycle_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/cycle_e2e.rs):
  12 `t375` single-site demotions, 5 `hetero`, 3 `nodet`. Two `nodet`
  reasons are now stale — the qualified-path and macro detectors landed
  (WI-3), yet `25-symbol-sweep` and `26-macro-sweep` still diverge because
  the *representation* differs: authored `# symbol Old=crate::old_api`
  vs inferred concrete `crate::old_api(E0)`, and authored `# sweep` dummy
  vs inferred tight range. `canonicalize`
  ([canonical.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/canonical.rs))
  is a pure parse → re-emit with no pre-image, so it cannot normalize
  either gap. `16-repeat-fold` has no raising detector at all (no
  `# repeat` is ever inferred). An agent cannot tell which divergences
  are “defined” and which are bugs without reading the harness source.
- **F2. The trust envelope proves less than it says.** In
  [plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
  pass 3, `check_untouched`
  ([edits.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/edits.rs))
  re-derives the post-image with the same `apply_edits` that produced it
  — it can only fail if materialization is non-deterministic, so
  `verified: {reparse: true, untouched_outside_spans: true}` is true by
  construction on every success. Bindings report only the *first* match
  (plan.rs WI-2 comment; capped at 8 entries × 80 chars in
  [applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)),
  so sites 2…N of a sweep — exactly where over-match hides — show spans
  but no content. `check: true` means only “the 0-context lowering
  passed `git apply --check`”, which is blind to semantic over-match
  (T361). On the `diffrs` side, multi-hunk `saved_bytes` is an equal
  split of the section delta (“first hunk takes the remainder”,
  [prov.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/prov.rs)),
  `sites` is a covering superset, and `totality: "section-visible"` —
  the no-pre-image fallback — is presented in the same shape as
  `"pre-image"` proof. `--verify build` runs only after a real apply and
  leaves a broken patch applied.
- **F3. Failures are still prose for the failures that matter.**
  `repair` has exactly three producers: ordinary-hunk `fix_header`
  ([parser.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/parser.rs))
  and `fix_context` ×2
  ([ordinary.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/ordinary.rs)).
  Zero-match, too-many, pattern-shape, capture, and symbol conflicts —
  the failures an authoring agent actually iterates on — carry only an
  English `suggestion`. The zero-match path already computes the two
  facts a machine repair needs (whole-file probe count + nearest-candidate
  preview in [suggest.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/suggest.rs))
  and then flattens them into a sentence.
- **F4. Raising is line-shaped in a token-shaped language.** Every pair
  detector consumes whole `-U0` lines and every sweep requires N ≥ 2
  (grep `len() < 2` across `semantic/*.rs`). Consequences, all pinned:
  two calls on one line never compress (`27-call-sweep`: 20 sites,
  10 lines, stays ordinary); a line holding one macro delta plus other
  edits needs the narrow nested-extraction path; single-site call /
  method / use rewrites always stay ordinary even when totality is
  pre-image-proven. Anti-unification
  ([antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs))
  generalizes one root at depth ≤ 2: joint `let` templates (pattern +
  type + initializer varying together, fixture `24-pat-sweep`) and
  statement sequences (C18) have no template at all. Verbatim is
  single-line by construction.
- **F5. The general matcher still stops at everyday Rust.**
  [matcher_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general.rs)
  `match_expr` falls to `_ => false` for `async` blocks, `try` blocks,
  `unsafe` blocks, `yield`, and macro expressions as pattern roots;
  [matcher_general_ctrl.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general_ctrl.rs)
  `match_stmt` compares `Stmt::Item` strictly token-equal and rejects
  `Stmt::Macro` outright, so a block containing either can never match
  structurally; `match_type` has no `Type::Macro` arm and path matching
  has no parenthesized `Fn(A) -> B` argument arm; blocks require equal
  statement counts with no ellipsis anywhere (C1). SPEC 18.6 R1/R5
  correctly call the remainder permanent v0.1 scope — the dogfood case
  for reopening is F4's ordinary residue, which is now measured.
- **F6. Renames are precise but not complete, and the engine cannot say
  the paths users write.** `# follow=uses`
  ([use_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph.rs))
  is syntactic, single-unique-definition only: glob imports, macro /
  attribute interiors, and shadowed names veto (or skip under
  `# fallback=syntax`); there is no re-export chasing beyond what the
  collector happens to see, and a definition-only rename that orphans
  every caller still passes `--check` clean with no signal. Call
  matching rejects leading-`::` callees outright
  ([matcher_expr.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_expr.rs):
  `pat_path.leading_colon.is_some() → false`) and inference declines
  them for exactly that reason
  ([paths.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/paths.rs)
  “no engine spelling”); `qself` paths (`<T as Trait>::m`) veto in both
  directions. `# symbol` remains exact full-path text with no resolver
  (SPEC 19.3). Cross-file follow loads the workspace by walking the
  filesystem (`workspace_rs_files` in applyrs main: every `.rs` under
  `--repo` except `.git`/`target`/`.history`) — ignored, generated, and
  vendored files are searched and rewritten as readily as tracked code.
- **F7. Moves still demand byte identity for everything except `fn`
  headers.** Move pairing
  ([moves.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/moves.rs))
  is byte-identical text matching; the evolve path
  ([move_evolve.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/move_evolve.rs))
  covers only `fn` vis/qualifier/name with generics, params, return,
  `where`, attrs, and body token-equal. A `struct` that gains `pub`
  while moving files, or an `fn` whose signature evolves while moving,
  stays a full delete+insert — the largest native patches there are.
  Captures are scalar by spec (SPEC 20.3): moving five items is five
  bind/use pairs with five range discoveries, and collection captures
  are explicitly deferred (SPEC 32/33.2).
- **F8. Cross-file intent and file-level events still fragment.**
  Grouping ([group.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/group.rs))
  joins only single-ident-delta rename-like hunks across ≥ 2 files;
  divergent deltas, template sweeps, and single-file repeats never
  group. Headers-only renames and mode-only sections stay passthrough
  (T377/T401). Two renames of different idents in one refactor surface
  as unrelated `rN` hunks with no shared provenance.
- **F9. The written contract can still drift from the language.** No
  test executes the RSmPL blocks in SPEC §28/§29 or Appendix A;
  [agents_md.rs](/home/ai/git/tools/rsmpl-git/bin/applyrs/tests/agents_md.rs)
  checks that AGENTS.md *citations* resolve, not that its recipes apply.
  §28.2 still teaches `# expr E` declarations beside the diet form the
  same section calls canonical. File-size pressure is returning:
  `matcher_general.rs` 1825, `matcher_expr.rs` 1787, `matcher_fn.rs`
  1722, `moves.rs` 1672, `parser.rs` 1633, `semantic/mod.rs` 1613 lines.
- **F10. Addressing is brittle and the native side is not quite native.**
  Semantic ranges cite original line numbers (SPEC 10); when the file
  moved since the agent read it, the patch fails with a range/match
  conflict and the agent pays a full re-read — Git's own answer to the
  same problem (context lines as anchors) is available in ordinary
  hunks but semantic hunks ignore body context lines for anchoring.
  The lowering is always 0-context (SPEC D.2.2): it applies only via
  `git apply --unidiff-zero`, so the “native patch” an agent hands to
  stock tooling is not appliable by vanilla `git apply`. `gitrs` does
  not check that its `applyrs`/`diffrs` siblings are the same version,
  and every `applyrs` run — including `--check` failures — archives
  into `<repo>/.history/` (128 kept), dirtying the worktree it measures.

## Key decisions

- **D1. Losslessness is judged per origin corpus, in writing.** Native-origin
  diffs owe state-losslessness (post-tree byte-identical). Semantic-origin
  patches owe canonical-losslessness for multi-site shapes and a *defined*
  fixed-point equality (`S″ == S′`) for single-site shapes the byte guard
  demotes. WI-1 rewrites SPEC D.5 to say exactly this and deletes every
  exclusion that no longer needs to exist. Raw native bytes never
  round-trip (0-context lowering, `index`/title stripping) — declared, as
  today. Binary/gitlink/non-UTF-8/CRLF semantic compression stays out of
  scope; passthrough cycles must remain tree-lossless.
- **D2. No success field may be true by construction.** Every `verified` /
  `checked` claim must be computed by a check that can fail on real input
  (postconditions re-matched against the post-image, per-site bindings,
  orphan enumeration). If a check cannot fail, it is removed, not renamed.
- **D3. Inference stays precision-first.** Every new detector keeps
  pre-image totality (count must equal pair count, zero opaque hits) and
  the T375 byte-win guard measured in diet bytes, extended only where
  WI-4 states an explicit, pinned slack rule. A detector that cannot
  prove totality stays ordinary — no guessing (SPEC 2.6).
- **D4. One anti-unification core, thin detectors.** WI-5 extends the
  existing `antiunify.rs` core; WI-4 and WI-8 reuse it instead of growing
  per-shape string surgery. Engine work (WI-5/WI-6) is structural `syn`
  matching in the WI-5 style — concrete, fail-loud, no expansion, no
  guessing.
- **D5. Split before growing.** New code lands in new modules; no item
  pushes a file over 2000 lines, and any file over 1600 lines touched by
  an item leaves under 1600 or with its new code in a sibling module.
- **D6. Familiar syntax wins over clever syntax.** New surface reuses
  Git/SmPL/Rust spellings (`...` where SmPL and Rust already use it for
  “the rest”, `#` directives, concrete paths). Diet remains canonical;
  the parser keeps accepting long forms so `.history/` archives stay
  readable. Any new spelling must be explainable in one AGENTS.md row.
- **D7. Breaking changes are taken where they buy trust.** JSON schema
  stays `version: 1` for additive keys; a key whose *meaning* changes
  (WI-2 `checked`) bumps to `version: 2` with the old key removed, per
  SPEC D.2's own rule. SPEC text moves with code in the same item.

## Work plan

Order is dependency order: WI-1 → WI-2 → WI-3 → WI-4 → WI-5 → WI-6 →
WI-7 → WI-8 → WI-9 → WI-10. WI-1 first because every later emitter and
harness compares canonical forms; WI-2 second because breadth without
honest evidence repeats F2 at larger scale. Sizes are relative
(S ≤ 1 day, M ≈ 2–3 days, L ≈ 4–5 days of focused work).

### WI-1 (P0, M): True lossless cycles — tree-aware canonical form, `# repeat` raising, D.5 burn-down [carries F1; closes old WI-1 residue]

**Problem.** F1/D1: the cycle contract is tested but not true. Authored
`# symbol` hunks and inferred concrete-path hunks are the same rewrite in
two spellings; authored `# sweep` and inferred tight ranges are the same
search at two localities; `# repeat` folds have no raising detector;
single-site demotion is pinned as “divergent” with no stated equality an
agent can rely on. `canonicalize` cannot fix any of this because it never
sees the tree it canonicalizes against.

**Decision — three exact changes, no new CLI flags.**
(1) *Tree-aware canonicalization for comparison.* Add
`canonicalize_with_tree(patch, pre_image)` in `rsmpl_patch` (backed by an
engine match pass): each `allow=many` hunk is matched against the
pre-image, its site span derived, and the hunk re-emitted under the D.5
range rule (tight range ≤ 64 lines, else `# sweep` dummy) — so authored
sweep and inferred ranged forms of the same sites canonicalize
identically. Pure `canonicalize` stays for idempotence pins; the cycle
harness compares with the tree-aware form. (2) *Spelling unification.*
In v0.1 syntactic mode a `# symbol` call hunk *is* its concrete path:
tree-aware canonicalization rewrites `- Old(E)` + `# symbol
Old=crate::old_api` to `- crate::old_api(E)` (and the `New` side
likewise), dropping the symbol/fallback directives. Authored symbol
patches and inferred concrete patches then share one canonical form;
SPEC 19.3 already tells authors to prefer concrete paths, so this
normalizes toward the documented preference instead of inventing a third
spelling. (3) *Equalities per origin (D1).* SPEC D.5 is rewritten:
native-origin cases owe tree equality; semantic-origin multi-site cases
owe `canonicalize_with_tree(S′) == canonicalize_with_tree(S)`;
semantic-origin single-site cases demoted by T375 owe tree equality plus
the fixed point `S″ == S′` byte-identical — stated as the contract, not
an exclusion. A new detector `try_repeat_fold_sweep` closes `16`:
N ≥ 2 nested same-callee pairs (`Box::new(Box::new(x))` → `Box::new(x)`
or → `x`, one uniform fold depth) anti-unify to one `# repeat` hunk,
gated by the generic engine-count totality (the emitted `# repeat`
minus must match exactly N sites in the pre-image). `12` (statement
sequences), `24` (joint let), and `27` (two-per-line) move to WI-5/WI-4
ownership in D.5; `14` divergent single-file renames and `30` trait
decl/def pairs stay excluded with sharpened one-line reasons (divergent
deltas are different intents; decl/def pairing is WI-7 grouping
territory) — everything else on today's list is deleted.

**Changes.** SPEC D.5 (rewrite: equalities, tree-aware canonical form,
exclusion list), D.4 (repeat-fold bullet), 19.3 (canonical-spelling
note). Code: tree-aware canonicalization in
[canonical.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/canonical.rs)
(match via `rsmpl_engine::apply_patch_to_tree` on a scratch tree — no
new matcher); repeat-fold detector in new
`crates/rsmpl_diff_infer/src/semantic/repeat.rs` (D5: `semantic/mod.rs`
is at 1613 lines, registration only); harness
[bin/diffrs/tests/cycle_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/cycle_e2e.rs)
switches the canonical leg to the tree-aware comparator and re-pins
`KNOWN_DIVERGENT` to the surviving list.

**Acceptance.** Cycle harness green with the exclusion list at exactly
the WI-4/WI-5-owned entries plus `14`/`30`; `25`/`26`/`16` move to
`CYCLE_OK`. Every case real-applies (never `--check` alone, T361).
History sweep still 446/446 convert, 0 failures; compressing set does
not shrink.

**Tests.** Tree-aware canonicalize idempotence + symbol→concrete and
sweep↔ranged unification pins; repeat-fold fire/veto units (uniform
depth, divergent depth veto, non-nested veto); full corpus cycle run.

**Non-goals / risks.** No native-byte round-trip (D1). Risk: tree-aware
canonicalization hides a real emitter nondeterminism — mitigated by
keeping the pure-`canonicalize` fixed-point pins and asserting `S″ == S′`
on raw bytes in the harness.

### WI-2 (P0, L): Trust envelope v2 — per-site evidence, real postconditions, honest `checked` [carries F2; hardens old WI-2]

**Problem.** F2/D2: an agent that wants to trust a patch today must
still reconstruct the interesting edits. First-match-only bindings hide
sweep sites 2…N; `verified` cannot fail; `check: true` conflates
“lowering applies” with “semantics did what the pattern said”;
`diffrs` provenance reports invented per-hunk byte wins and presents
section-visible guesses in proof's clothing; `--verify build` can only
fail *after* the worktree is already broken.

**Decision — evidence that can fail, schema `version: 2`.**
`applyrs --format json` bumps to `version: 2` (D7: meanings change).
Per rule, `bindings` becomes `sites`: one entry per emitted edit, in
span order — `{span: [s, e], bindings: {…}}` with the same 80-char /
per-site truncation (`bindings_truncated` now counts truncated *sites*).
The payload replaces `verified` + `check` with
`checked: {lowering: bool, semantics: bool, build: "skipped"|"pass"|"fail"}`:
`lowering` is today's `git apply --check` verdict; `semantics` is a new
engine postcondition pass — after materialization, the minus pattern
must match zero further sites in the post-image inside scope (except
`# optional` zero-edit and `# repeat` fixed points, which must match
zero *foldable* sites) and the plus pattern must match exactly at the
reported spans; any violation fails the run loud with
`RSMPL_MATCH_CONFLICT`, never a warning. The tautological
`check_untouched` re-derivation is deleted (byte-identity outside spans
is already guaranteed by construction: edits are the only mutation —
SPEC 24.6 is rewritten to say so and to own the postcondition instead).
`--verify build` gains check-mode semantics: with `--check` it applies
the lowering to a temporary copy of `--repo`, runs
`cargo check --quiet` there, and reports `build` without touching the
worktree; without `--check` it keeps today's post-apply run, and on
failure the JSON error names the first diagnostic *and* the applied
state explicitly (`"applied": true` in the error envelope). `diffrs
--format json` provenance becomes honest: `saved_bytes` is measured per
hunk (emitted hunk bytes vs the ordinary lines it replaced; composed
section drivers measure per stage, never equal-split), `sites` stays
covering but gains `sites_exact: bool`, and `totality` gains a third
value — `"section-visible"` entries additionally carry
`"proven": false` while `"pre-image"` carries `"proven": true`. Text
modes are unchanged in both binaries.

**Changes.** SPEC D.2 (v2 envelope, budgets recomputed, `--verify`
semantics), 24.6 (postcondition replaces re-derivation), D.4
(provenance honesty). Code: per-site binding capture threaded through
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
matchers (they already bind per candidate; keep all, bound at report
time in [applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs));
postcondition pass in new `crates/rsmpl_engine/src/postcheck.rs`;
temp-copy verify in applyrs main (std copy, no new deps); per-hunk
byte accounting in
[prov.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/prov.rs)
and its section-driver callers.

**Acceptance.** Scripted agent flow — `diffrs --format json`, inspect
only `hunks`, then `applyrs --format json --diff-max-lines 0 --check` —
reaches the same accept/reject decision as full manual inspection on
every fixture, including an over-broad sweep fixture whose extra site
appears in `sites` with its bindings before apply, and a postcondition
fixture (pattern whose plus still matches minus, e.g. `foo(E)` →
`foo(E)`) that must now fail `semantics` loud. Report overhead stays
under the recomputed D.2 budget pin. `--verify build --check` leaves
the worktree byte-identical on both pass and fail fixtures.

**Tests.** V2 envelope shape pins; per-site bindings on a 3-site sweep;
postcondition fire/no-fire matrix (repeat fixed point, optional);
tampered-post-image unit proving `semantics` can fail; verify-in-check
temp-copy pins; provenance per-hunk byte sums equal section delta.

**Non-goals / risks.** No type-level proof by default beyond opt-in
`cargo check`; no rust-analyzer. Risk: postcondition re-matching costs
a second engine pass — bounded by scope (file-scope hunks re-scan their
file once); pinned by the existing bench suite staying green.

### WI-3 (P1, M): Machine-actionable failures and an executable written contract [carries F3 + F9; completes old WI-10]

**Problem.** F3/F9: the agent loop still breaks on prose. The failures
an authoring agent hits most — zero-match, too-many, pattern-shape,
capture, symbol — have no `repair`; and the documents agents author
against (SPEC §28/§29, Appendix A, AGENTS.md, SKILL.md) are never
executed, so they can silently stop being the language the parser
accepts (§28.2 already mixes diet and long spellings as if equivalent
teaching).

**Decision — five repair producers, one conformance suite, no new user
syntax.** The JSON/text error envelope gains `repair` payloads (SPEC
D.2, additive within v2) for: zero-match with exactly one whole-file
match → `{action: "widen_scope", directive: "# sweep", range:
"@@ -1,1 @@"}` (from the existing T62 probe, not new logic); zero-match
otherwise → `{action: "read_file", file, range}` plus the T67 candidate
preview as structured `{line, text}`; too-many →
`{action: "add_directive", directive: "# allow=many"}`; pattern-shape
(C-table) → `{action: "fix_pattern", code: "C1"…"C18", write_instead:
"<SPEC 18.6 text>"}`; capture/symbol →
`{action: "add_directive"|"fix_name", …}` naming the missing
`# bind=`/`# fallback=syntax` or the available capture names (already
computed in [capture.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/capture.rs)).
Prose `suggestion` stays verbatim for humans. Conformance: new test
`bin/applyrs/tests/spec_examples.rs` extracts every fenced `diff` block
tagged `rsmpl` from SPEC §29 + Appendix A + AGENTS.md + SKILL.md and
asserts each parses and — where the surrounding text claims a match or
rejection — applies or fails as claimed against its inline Rust
snippet. §28/§29 are rewritten in canonical diet as part of this item;
fences gain the `rsmpl` info string so extraction is by tag, never
heuristics. File splits (D5): directive parsing completes its move out
of [parser.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/parser.rs)
(1633 lines) into `directives.rs`, and detector registration stays in
`chain.rs`; both files leave under 1600 lines.

**Changes.** SPEC D.2 (five repair shapes), §28/§29 diet rewrite, fence
tags. Code: repair population at the existing
`ensure_match_count` / compile / capture sites
([validate.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/validate.rs),
[errors.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/errors.rs));
the extractor test (no new dependencies).

**Acceptance.** Every extracted example passes as written; deleting a
directive from a SPEC example in a scratch edit fails the suite
(mutation-checked once during development, then reverted). Repair
objects present and pinned for the five conflicts in both text
(`repair: {…}` line) and JSON. An agent-loop probe — fail with
zero-match, apply `repair` mechanically, retry — succeeds first-retry
on the probe corpus without reading prose.

**Tests.** The conformance suite; repair pins per conflict (text +
JSON, success silence preserved); split = existing suite unmodified.

**Non-goals / risks.** No SPEC redesign, no grammar formalization beyond
the examples, no removal of prose suggestions. Risk: extractor
brittleness — mitigated by tag-based extraction from this item on.

### WI-4 (P1, M): Token-shaped raising — sub-line sites and honest single-site semantics [new; carries F4 `27`]

**Problem.** F4: diffs are lines, rewrites are tokens. Two calls on one
line (`foo(1) + foo(2)`, fixture `27-call-sweep`, 20 sites) can never
compress because pairing and totality count lines; a line carrying one
compressible delta plus an unrelated edit is split by hunk atomicity or
stays ordinary; and every N ≥ 2 gate means the most common real change —
one call site migrating — always pays ordinary bytes with no semantic
safety, even when pre-image totality could prove it.

**Decision — pair and count at token span, keep the byte guard honest.**
(1) *Sub-line pairing.* Detectors that today consume whole lines first
try a sub-line extraction: when a `-`/`+` line pair differs by exactly
one candidate subterm (same extraction discipline as WI-9 nested macros:
rendered delta occurs byte-exact once per line, plus line equals minus
line under that one replacement), the pair contributes that subterm as
its site and the line's other text becomes a literal frame the emitted
pattern must preserve via the general matcher. Totality counts
*occurrences of the minus subterm in the pre-image* (subterm totality,
as verbatim already does), not changed lines — two-per-line then counts
20, not 10, and fires. Hunk atomicity is unchanged: a frame that itself
changed vetoes. (2) *Single-site rule.* N = 1 stays ordinary for call /
method / use shapes (no token win, no fan-out safety to buy — pinned,
not apologized for), *except* where the semantic form carries safety
the ordinary line cannot: fn/item renames and moves already fire at
N = 1 and keep doing so. No slack is added to T375 anywhere in this
item: a sub-line firing must still be byte-smaller than the ordinary
lines it replaces, measured on the whole section as today. What changes
is only that multi-site truth is no longer under-counted into a veto.

**Changes.** SPEC D.4 (sub-line pairing + totality-by-occurrence, N = 1
rule stated). Code: shared sub-line extractor in
`crates/rsmpl_diff_infer/src/semantic/subline.rs` (generalizing the
nested-macro extractor in
[macros.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macros.rs));
call/method/qualified detectors accept extracted sites; occurrence
counters in [totality.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/totality.rs)
beside the line counters (line counters stay for line-shaped detectors).

**Acceptance.** Fixture `27` moves from D.5 exclusion to cycle-OK;
new diffrs bench 28 (two-per-line migration, modelled on `27`) ≥ 50%
saved; a same-line distractor fixture (second `foo` on the line
unchanged) vetoes to ordinary. History compressing set does not shrink;
every new fire real-applies byte-identical.

**Tests.** Sub-line fire/veto matrix (two-per-line, distractor,
frame-changed, multi-delta line); occurrence-totality units; cycle +
fixtures round-trip coverage.

**Non-goals / risks.** No general multi-delta-per-line templates (that
is WI-5 sequence territory). Risk: frame preservation over-matches —
the general-matcher frame plus occurrence totality are the gates, and
the distractor veto pin is an acceptance gate, not an aspiration.

### WI-5 (P1, L): Joint templates and statement sequences — anti-unification completion, C18 lifted narrowly [carries F4 `12`/`24`; completes old WI-4/WI-9]

**Problem.** F4/F5: the refactor shape real code takes most — “the same
few statements, with these holes, in this order” — has no expression in
either direction. Authored multi-statement patterns fail at C18;
`diffrs` anti-unification generalizes a single root at depth ≤ 2, so
joint `let` changes (pattern, ascription, and initializer varying
together) and repeated statement sequences stay ordinary. The engine's
`Block` arm matches only equal-length concrete sequences, so even a
hand-authored sequence template cannot vary in length.

**Decision — one new engine position, one core extension (D4/D6).**
(1) *Engine: trailing block ellipsis.* A general `Block` pattern may end
with one final pseudo-statement `...` matching any remaining statements
(zero or more); the plus side must carry the trailing `...` in the same
position or omit it to deliberately drop the tail (mirroring the
call-argument `...` replay/drop rule, SPEC 16.3 — the spelling agents
already know). No other `...` position is admitted in general patterns;
C1 otherwise stands, and statement counts before the `...` must match
exactly. This is the minimal length-variance the sequence shape needs
and nothing more. (2) *Core: joint generalization.* `antiunify.rs`
gains sequence generalization: N ≥ 2 statement sequences (2–4
statements, inside one function body) generalize statement-by-statement
with the existing hole discipline (same-variant nodes, depth ≤ 2,
≥ 1 literal anchor per statement, repeated-hole structural equality,
SPEC 15.7); a joint `let` detector (`try_let_template_sweep`) is the
single-statement special case where pattern + type + initializer holes
coexist — strictly generalizing today's type-add-only slice, which is
deleted into it. (3) *Raising:* `try_sequence_sweep` emits the sequence
as one general hunk ending in `...` when trailing statements vary;
totality is the generic engine-count check (emitted minus must match
exactly N blocks' prefixes in the pre-image). Chain order: after the
WI-4 detectors, before verbatim (verbatim stays the tighter winner for
identical lines, T422 precedent).

**Changes.** SPEC 18.6 (C1/C18 narrowed with exact diagnostics, R-table
note), 16 (block-ellipsis position), D.4 (joint-let + sequence
bullets), 15.7 (sequence holes). Code: block-ellipsis arm in
[matcher_general_ctrl.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general_ctrl.rs)
`match_block`; sequence generalization in
[antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs);
detectors in [generalized.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/generalized.rs);
engine-count totality reuse (no new hand-rolled counters).

**Acceptance.** New applyrs bench 24 (4-site sequence rewrite with
trailing variance) and diffrs bench 29 (same change, raised) each
≥ 40% saved; fixtures `12`/`24` move to cycle-OK. Probe corpus before
merging: the ten largest ordinary-only history commits are tried; at
least three gain a semantic hunk with byte-identical real-apply, or the
guards tighten until precision holds (report which, in the landing
change).

**Tests.** Block-ellipsis matcher matrix (fire, drop-tail, mid-block
`...` compile veto); anti-unify sequence matrix (length, anchor,
repeated-hole); detector fire/veto incl. control-flow member whose
header holes but body is concrete; cycle coverage.

**Non-goals / risks.** No `...` in expressions/types, no statement
ellipsis in call/fn shapes, no macro expansion. Risk: trailing-`...`
over-generality — the anchor-per-statement plus engine-count totality
gates are acceptance criteria; a sequence with no literal anchor in any
statement never fires.

### WI-6 (P1, M): Engine remainder — async/try/unsafe, statement interiors, `Fn` sugar, macro types [carries F5]

**Problem.** F5: `async { … }`, `try { … }`, `unsafe { … }`, `yield`,
macro statements/items inside blocks, parenthesized `Fn(A) -> B`
bounds, and macro types each force a whole-construct ordinary hunk.
These are not exotic: async/try blocks are the default shape of modern
Rust bodies, so body-level semantic patches stop exactly where agents
most need them.

**Decision — structural arms in the WI-5 style, verbatim where opaque.**
Extend `match_expr` with: `Async` (capture/`move` qualifiers strict,
body as `Block`), `TryBlock` (body as `Block`), `Unsafe` (body as
`Block`), `Yield` (optional value via `opt_expr_eq`); each is a small
delegation to the existing `match_block`, in a new
`matcher_general_async.rs` sibling (D5 — `matcher_general.rs` is at
1825 lines and shrinks by the move). `match_stmt` learns two arms:
`Stmt::Macro` matches only when the whole pattern statement is itself
the same macro invocation shape (delegated to the macro matcher —
interiors stay opaque, SPEC 21 stands) and `Stmt::Item` matches
structurally for `fn` items only (delegated to the fn matcher), staying
strictly token-equal for every other item kind (an item rename inside a
block remains two changes, never guessed). `match_type` gains
`Type::Macro` as verbatim-only (token-equal, never holed — same posture
as string literals in inference). Path matching gains
`PathArguments::Parenthesized` (inputs via `match_type`, output via
`match_return_type`), so `Fn(T) -> U` bounds with `T`/`U` holes match
structurally. Everything not listed — `asm!`, `format_args!` interiors,
macro expansion — still rejects. SPEC 18.6 R1/R5 rows are rewritten to
the new bounded set with the reopening dogfood cited (F5 probe corpus).

**Changes.** SPEC 18.6, 18.1 bounded-set note. Code: new matcher module
plus the three `match_stmt`/`match_type`/path arms; shape gates in
inference that hard-code variant lists (`verbatim_shape_ok` and
siblings) are updated to ask the compiler (compile + count) instead of
veto lists, so raising follows the engine automatically.

**Acceptance.** New applyrs benches 25 (`async` block body rewrite over
4 sites), 26 (`unsafe`/`try` header-hole rewrite), 27 (`Fn(T) -> U`
bound sweep) each ≥ 35% saved vs native; diffrs fixture pins prove the
raising direction for each. History sweep: compressing delta reported,
0 failures is the hard gate.

**Tests.** Per-arm matcher units (fire, qualifier-divergent veto,
label veto, macro-type verbatim fire + hole veto, parenthesized-args
fire/veto); SPEC 18.6 verbatim e2e expectations move rows, never
deleted; full fixtures + benches + cycle harness.

**Non-goals / risks.** No macro expansion (SPEC 21.5 stands), no
`...` (WI-5 owns the one new position). Risk: `syn` parses these shapes
differently across editions — parse context is already edition-default;
pins use the workspace edition (2021) and say so in SPEC.

### WI-7 (P0, L): Rename completeness v2 — git-aware follow, re-export hop, orphan evidence, `::`/qself spellings [carries F6]

**Problem.** F6: the flagship rename is still easy to get silently
wrong. Definition-only renames orphan callers with a clean `--check`;
`# follow=uses` vetoes on the workspaces agents actually have (glob
imports, macro mentions, a namesake in another module) or — worse —
searches files git ignores; and the paths real code writes
(`::crate::api(…)`, `<T as Trait>::m(…)`) cannot be expressed in either
direction at all.

**Decision — four bounded hardenings, defaults unchanged.**
(1) *Git-aware workspace.* Cross-file follow loads its search set from
`git ls-files --cached --others --exclude-standard` (tracked +
untracked-but-not-ignored), never a filesystem walk; ignored/generated
files are neither searched nor rewritten, and the set is listed in the
report (`follow_files: N`). Outside a git worktree the loader falls
back to today's walk and the report says `"follow_source": "walk"`.
(2) *One re-export hop.* The use-graph follows a single `pub use`
re-export: renaming `a::Old` also rewrites `pub use a::Old` in `b` and
uses spelled through `b::Old` when the re-export is unambiguous (exactly
one hop, exactly one target; a second hop or a glob re-export is opaque
and vetoes/skips as today). (3) *Orphan evidence for plain renames.*
Every fn/item rename *without* `# follow=uses` runs the same syntactic
index in report-only mode: the JSON gains `orphans: [{file, line,
text}]` (capped like bindings, with `orphans_truncated`) listing
same-name uses the rename leaves behind. `--check` still passes — the
rename may be intentional — but the agent sees the breakage in the
report instead of discovering it in `cargo check`. With
`--verify build` the orphans are the first diagnostic's context.
(4) *Engine spellings.* Call matching compares `leading_colon` instead
of rejecting it (`::a::b` patterns match only `::a::b` sites), and
`qself` callees match when the *written* qself type and trait path are
token-equal (no resolution — the agent spells what it means, the engine
compares it strictly). Inference then stops declining both
([paths.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/paths.rs)
leading-`::` decline deleted; qself pairs join the qualified detector
with qself text as prefix). `# symbol` stays exact-text (SPEC 19.3
stands — no resolver is built; the concrete-path preference is now
canonical per WI-1).

**Changes.** SPEC 14.8 (hop + orphan semantics), 19.3 (unchanged,
cross-referenced), D.2 (`orphans`, `follow_*` fields), D.4
(leading-`::`/qself bullets). Code: loader swap in
[applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
(`workspace_rs_files` deleted); hop + report-only mode in
[use_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph.rs)
(new `use_graph_hop.rs` sibling per D5 — the file is at 1105 lines);
leading-colon/qself arms in
[matcher_expr.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_expr.rs)
(match logic in a new `matcher_path.rs` — that file is at 1787 lines);
detector un-declines in paths.rs with totality keys extended to the
full written path.

**Acceptance.** Bench 10 with `# follow=uses` still fires and compiles
under `--verify build`; an ignored-file fixture (`.gitignore`d `.rs`
naming the old ident) is neither rewritten nor vetoing; a one-hop
re-export fixture rewrites definition + re-export + through-uses in one
hunk; a definition-only rename fixture reports its two orphan call sites
in JSON while `--check` stays green; `::`-prefixed and qself fixtures
apply authored and raise inferred, real-apply byte-identical.

**Tests.** Loader set pins (tracked/untracked/ignored); hop fire +
second-hop veto + glob-reexport veto; orphan report pins (present,
empty-omitted, truncated); leading-colon parity (bare pattern never
matches `::` site and vice versa); qself token-divergent veto. Full
workspace + cycle harness.

**Non-goals / risks.** No cross-crate resolution, no type inference, no
second re-export hop. Risk: `git ls-files` cost on huge repos — the set
is loaded only when a patch carries `# follow=uses` or a rename (orphan
scan), and the count is reported so the cost is visible.

### WI-8 (P1, L): Move + evolve for every item kind, signature-evolving moves, collection captures [carries F7]

**Problem.** F7: moves are the largest patches and the least
compressed. Only `fn` headers may evolve during a move; every other
kind needs byte identity; an `fn` that changes signature while moving
cannot move semantically at all; and moving several items costs one
scalar capture, one range discovery, and two hunks per item.

**Decision — generalize the two mechanisms that already work.**
(1) *All-kind header evolution in raising.* `move_evolve` pairing
extends from `fn` to every named item kind: items pair when their
anti-unification (WI-5 core over the parsed items) differs only in
header slots — name, visibility, and for types the leading path
segments — with attrs, generics (except the renamed segment), fields /
variants / bodies token-equal, under the existing ≥ 80%-literal-token
pairing gate (pinned constant, measured on tokens) plus cut-file
uniqueness (twin veto, as today). Emission keeps the scalar cut
(`# bind=Name`, `- I`) and makes the paste a header-template paste for
every kind: the engine's splice (today `fn`-only in
[capture.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/capture.rs))
generalizes to replacing the captured item's header span with the
instantiated plus-side header while the body rides verbatim — the same
`...`-body discipline as fn evolution. (2) *Signature-evolving fn
moves.* When the moving `fn`'s params/return/`where` also carry one
uniform delta, the paste template spells the full plus-side header
(generics/params/return/`where` verbatim from the plus item, body
spliced) — sound because the template is concrete plus-side text, never
a hole; body deltas still veto (a move that rewrites the body is two
changes). (3) *Collection captures (authored + raised).* `# bind=Name`
combined with `# allow=many` — today a `RSMPL_CAPTURE_CONFLICT` — now
binds an ordered collection: the cut deletes every match in scope
(order = source order); `# use=Name` on a pure insertion pastes all of
them at the anchor in source order with the SPEC 20.2 one-blank join.
Scalar `# bind` without `# allow=many` is byte-for-byte today's
behavior. This is the SPEC 33.2 future extension, spelled with the
directives agents already know (no `bind_list` invention, D6).
`diffrs` uses it when ≥ 3 distinct items move between the same file
pair; below that, scalar pairs stay (byte guard decides).

**Changes.** SPEC 20 (header-template paste for all kinds, collection
captures, 80% gate), 24.4 (scalar-only rule rewritten), D.4 (move
bullet). Code: pairing in `move_evolve.rs` over the anti-unify core
(item-kind matchers in new `move_evolve_items.rs`; `moves.rs` at 1672
lines gains no net lines); splice generalization + collection store in
capture.rs (collections in new `capture_many.rs`); planner pass-1
collection cuts in plan.rs.

**Acceptance.** New diffrs benches 30 (struct move + `pub` evolve),
31 (fn move + signature evolve), 32 (five-item collection move) each
≥ 70% saved; body-editing and signature+body-editing move fixtures stay
ordinary (veto pins). Authored collection move applies and re-diffs
cycle-green under WI-1.

**Tests.** Pairing matrix per kind (identical, header-evolved,
body-veto, twin veto); splice units (name/vis/path-prefix per kind,
full-header fn template); collection cut/paste ordering + blank join +
scalar unchanged pins; fixtures round-trip real-apply.

**Non-goals / risks.** No statement moves (items only), no multi-hop
path rewriting inside moved bodies (`super::` → `crate::` inside a body
is a body delta and vetoes — stated in SPEC, not silently missed).
Risk: the 80% gate admits a lookalike pair — cut-file uniqueness plus
real-apply harness coverage are the mitigation; the constant stays
named and pinned.

### WI-9 (P2, M): Macro depth — nested interiors, `macro_rules!` renames, token-tree holes [new; completes macro raising]

**Problem.** The macro surface is half a language. Raising handles
same-name interiors and one nested delta, but macros inside macro
bodies stay ordinary (`syn::visit` never enters opaque token streams —
correctly, by design), `macro_rules!` definitions cannot be renamed
semantically in either direction (the item-rename kind list omits
them), and authored `# macro=token_tree` accepts only `NAME!(...)`
verbatim bodies, so an agent cannot write the per-argument holes that
invocation mode already supports.

**Decision — deepen by rendering, never by expansion (SPEC 21.5
stands).** (1) *Nested raising by token anti-unification.* The nested
extractor generalizes from “exactly one delta macro per line” to token
trees: a `-`/`+` line pair whose token streams differ in exactly one
macro invocation *at any nesting depth* (found by recursive token-tree
walk, rendered from raw `Span::byte_range` slices as WI-9b2 already
does for depth 1) contributes that invocation pair to the interior
detectors; macros inside string literals never count (strings are
literal-only positions, pinned). Depth is capped at 3 token-tree
levels; deeper stays ordinary. (2) *`macro_rules!` rename.* Item
rename gains the `macro_rules` kind (`- macro_rules! Old` /
`+ macro_rules! New`, definition span = the full `macro_rules!`
item, bodies preserved byte-verbatim like fn bodies); call-site
following reuses the macro-name path (invocations are already
collected), and `diffrs` gains the single-pair detector arm beside
`try_item_rename`. (3) *Authored token-tree holes.* `# macro=token_tree`
patterns may name one metavariable per top-level comma-separated
argument position (`- old!(E, lit)` / `+ new!(E, lit)`), matched by the
existing token-tree arg matcher; `...` remains the whole-body form.
Delimiters stay significant (paren/bracket/brace never cross-match).

**Changes.** SPEC 21.3 (holes + nesting posture), 14.8 (kind list),
D.4 (nested + macro_rules bullets). Code: recursive extractor in
`semantic/macro_nested.rs` (new; `macros.rs` at 1536 lines gains no net
lines); kind arm in
[item_rename.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/item_rename.rs)
+ `matcher_item.rs`; token-tree hole gate in
[matcher_macro.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_macro.rs).

**Acceptance.** Benches 17/18 each gain ≥ 5 points saved (nested
interiors compress); new bench 33 (`macro_rules!` rename + 4 invocation
sites, one hunk with follow) ≥ 60% saved; depth-4 nesting fixture stays
ordinary (veto pin). History sweep 0 failures, compressing set reported.

**Tests.** Nested fire/veto (depths 1–3 fire, 4 vetoes, string-hidden
macro never counts, multi-delta line vetoes); macro_rules authored +
raised + distractor-macro untouched; token-tree hole fire + delimiter
mismatch veto; cycle coverage.

**Non-goals / risks.** No expansion, no `macro` 2.0 (`macro` item)
support, no hygiene reasoning. Risk: raw-span rendering on non-UTF8
boundaries — slicing failures already fall back to normalized rendering;
the fallback path gets a pin here.

### WI-10 (P1, M): Drift-tolerant addressing and an honest native side [new; carries F10]

**Problem.** F10: patches fail for the wrong reason and interop leaks.
A semantic patch authored against yesterday's line numbers fails
outright when the file shifted, although the agent's intent (“this
pattern, near here”) is unambiguous; the lowered “native” patch needs a
non-stock `git apply` flag; sibling binaries can skew silently; and
measuring a repo dirties it with `.history/` writes.

**Decision — hint ranges, stock-compatible output on request, checked
siblings, clean measurement.**
(1) *Anchor fallback for semantic hunks.* When a ranged semantic hunk
finds zero matches in its cited range, the engine retries once with
`# scope=file` semantics *anchored* by the hunk's body context lines
(Git-familiar: leading/trailing ` ` lines as literal anchors): the retry
matches only candidates whose enclosing item also contains the anchor
text in order. Exactly one anchored candidate → apply it and report
`"drift": {rule, cited: [s,e], actual: [s,e]}` per drifted rule in the
v2 JSON (text mode: one `drift:` stderr line). Zero or ≥ 2 → the
original `RSMPL_MATCH_CONFLICT` stands, unchanged. `# sweep` /
`# scope=file` hunks never drift-report (they made no locality claim).
This is a *matching* fallback, not a range rewrite: emitted edits still
cite original coordinates, and WI-2 postconditions still gate the result.
(2) *Stock-git lowering on request.* `applyrs --context N` (default
unchanged: 0, SPEC D.2.2) emits the *reported/output* diff with N lines
of context so `--output` artifacts apply with vanilla `git apply`;
apply/check always use the full lowering internally either way. The
flag is plumbed to `rsmpl_git` diff assembly only — no engine change.
(3) *Sibling handshake.* `gitrs` compares `CARGO_PKG_VERSION` with each
sibling's `--version` before re-exec; mismatch fails with
`RSMPL_IO_CONFLICT` naming both versions (tests override via the
existing `GITRS_*_BIN` env vars, which skip the check). (4) *Clean
measurement.* `--check` runs no longer archive to `.history/` unless
`--history` is passed; real applies keep today's always-archive
behavior and the JSON `history` field reports `null` when skipped.
SPEC D.1 is updated to say so (audit value lives in applies; checks are
probes agents run dozens of times).

**Changes.** SPEC 10/17 (anchor fallback + drift evidence), D.1
(history-on-apply), D.2 (`drift`, `--context`), D.2.2 (context note).
Code: anchored retry in plan.rs beside the existing T62 whole-file
probe (reusing its count-only machinery — the probe becomes the
fallback when anchors disambiguate); context plumbing in
[rsmpl_git diff.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_git/src/diff.rs);
handshake + history gate in the three `main.rs` files.

**Acceptance.** A patch authored 20 lines stale applies with correct
`drift` evidence and byte-identical post-tree; the ambiguous-drift
fixture (two anchor-satisfying candidates) fails with the original
conflict and applies nothing. `--output` with `--context 3` applies
under vanilla `git apply` (no `--unidiff-zero`) in an e2e pin.
Mismatched-sibling fixture fails loud; `--check` leaves no `.history/`
entry, apply leaves exactly one.

**Tests.** Drift fire/ambiguous/no-anchor matrix; context lowering
vanilla-apply pin (0-context still requires the wrapper — documented,
not hidden); handshake pass/fail pins; history gating pins.

**Non-goals / risks.** No fuzzy/textual matching (anchors are literal,
patterns still semantic), no change to original-coordinate semantics
(SPEC 10/23.4 stand). Risk: anchored retry masks a genuinely wrong
range — the exactly-one-candidate gate plus drift evidence in the
report are the mitigations; drifted applies are always visible, never
silent.

## Sequencing and gates

| Item | Depends on | Size | Gate before next item |
|---|---|---|---|
| WI-1 canonical cycles | — | M | `cycle_e2e` green at the reduced exclusion list; sweep 0 failures |
| WI-2 trust envelope v2 | WI-1 spans | L | V2 pins green; agent-flow acceptance incl. over-broad + postcondition fixtures |
| WI-3 repair + contract | WI-2 envelope | M | Example suite green; 5 repair pins; mechanical-retry probe passes |
| WI-4 sub-line raising | WI-1 | M | Bench 28 ≥ target; `27` cycle-OK; distractor veto pinned |
| WI-5 sequences | WI-4, WI-2 postconditions | L | Benches 24/29 ≥ targets; probe-corpus criterion met |
| WI-6 engine remainder | WI-5 block shape | M | Benches 25–27 ≥ targets; 18.6 rows moved, none deleted |
| WI-7 rename completeness | WI-2 | L | Ignored-file, hop, orphan, `::`/qself pins green |
| WI-8 move + evolve | WI-5 core | L | Benches 30–32 ≥ targets; body-veto pins hold |
| WI-9 macro depth | WI-4 extractor | M | Benches 17/18 +5 pts; depth/string vetoes pinned |
| WI-10 addressing + native | WI-2 | M | Drift matrix green; vanilla-apply pin; history gating pinned |

Global validation per item: `cargo test --workspace`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`./example-benches/run.sh` (README tables regenerated only when bytes
change), and `python3 tools/history_sweep.py` (0 failures; compressing
set reported, never asserted to a fixed number beyond “does not shrink
without a named reason”). No item lands with a red gate, a new `.rs`
file over 2000 lines, or an existing file pushed over 2000 lines.

## Risks / open questions

Per-item risks are stated with each item. Open questions: none —
scope, canonical form, detector order, guards, thresholds (64-line
range rule, 80% move pairing, 80-char binding truncation, nesting depth
3, collection threshold 3), schema versioning (v2 in WI-2), and
acceptance numbers are decided here. If a threshold proves wrong
against the probe corpora, the fix is a new pin plus a SPEC note, not a
silent constant edit. Two deliberate non-goals bear repeating because
they bound several items: no semantic resolver is built (SPEC 19.3
exact-text matching stands; WI-7 hardens the syntactic graph instead),
and no macro expansion is attempted anywhere (SPEC 21.5 stands; WI-9
deepens rendering-level matching only).

## Considered and rejected

- **Full type-directed resolution (rust-analyzer integration).** Would
  make `# symbol` and follow=uses “really” correct, but couples every
  `--check` to a build environment, breaks the no-`cargo`-by-default
  cost model (D2), and is un-auditable in a hackathon prototype. The
  syntactic graph + orphan evidence (WI-7) buys most of the trust at
  none of the coupling.
- **General `...` in expressions/types/statements.** Rejected beyond
  WI-5's single trailing block position: unrestricted ellipsis turns
  precise patterns into guesses and breaks totality counting. The one
  admitted position mirrors the existing call-argument rule.
- **Dropping T375 for single-site semantics.** Rejected in WI-4: bytes
  are the token proxy agents pay; single-site safety shapes (renames,
  moves) already fire. Cycle honesty for demoted singles is bought with
  a defined equality (WI-1), not with larger patches.
- **Collection syntax inventions (`# bind_many=` etc.).** Rejected in
  WI-8 in favor of `# bind=` + `# allow=many` composition — one less
  spelling for agents to learn, and the conflict it replaces was an
  error, not a feature.
- **CRLF/binary semantic compression.** Stays declared-lossy (D1):
  detectors strip `\r` by construction and patches are UTF-8; passthrough
  tree-losslessness is the correct contract there.



