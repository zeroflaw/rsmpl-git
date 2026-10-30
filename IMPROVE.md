# IMPROVE — RSmPL as a verifiable agent contract: honest evidence, everyday-Rust coverage, raising depth

## Goal

Make RSmPL the contract an agent can trust in both directions without
reconstructing and inspecting every edit by hand:

- **Lowering** (semantic → native, `applyrs` via `gitrs apply`) must prove,
  per site, what it touched, what it deliberately left alone, and what it
  checked — in machine fields an agent can assert on, not prose, and never
  in fields that are true by construction.
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
WI-1…WI-8 largely landed, WI-9/WI-10 did not, and one WI-8 slice
(collection captures) never landed; each is carried forward explicitly
below. Fresh gaps found by this review are new items. Work stops at
10 items.

## Success criteria

1. **Trust (WI-1).** No success field is true by construction. An agent can
   accept or reject a patch from the JSON reports alone — per-site
   bindings (with truncation marked, never silent), per-rule postcondition
   evidence, orphan evidence — including one deliberately over-broad patch
   whose extra site is visible *before* apply.
2. **Everyday Rust matches (WI-2, WI-3, WI-4).** Macro subterms, array
   repeats, `const` blocks, `&raw` pointers, and destructuring patterns no
   longer veto whole patterns silently; impl/trait headers select the
   block they name in every scope. Anything still unsupported fails at
   compile time with an exact diagnostic, never as a bare
   `no match found` the agent must debug by reading the file.
3. **Raising breadth without precision loss (WI-4, WI-5, WI-8, WI-9).**
   Trait+impl method renames, hetero call-arg rewrites, multi-rename
   sections, and field/variant renames compress with pre-image totality
   and real-apply byte-compare (T361 doctrine: `--check` alone never
   proves a sweep). `python3 tools/history_sweep.py` stays at 0 failures
   and the compressing set does not shrink without a named reason in the
   landing change.
4. **Cycles (WI-4, WI-8).** The `cycle_e2e` harness stays green; the SPEC
   D.5 exclusion list shrinks or gains sharpened owners — it never grows
   silently. Single-site demotion stays a *defined* fixed-point equality.
5. **Self-correction (WI-2, WI-3).** Every new compile-time fence carries a
   machine `repair` payload with a concrete `write_instead` template an
   agent can apply without parsing English; no `repair` echoes prose.
6. **Gates.** `cargo test --workspace`, `cargo clippy --all-targets --
   -D warnings`, `cargo fmt --check`, and `./example-benches/run.sh` stay
   green per item; no `.rs` file exceeds 2000 lines (split before growing,
   D5); bench byte targets in each item are met or the item does not land.

## Review findings (evidence)

Verified this run at HEAD `7d7cc14` (491 commits): **1939 tests passing,
0 failed (summed across all `test result` lines); clippy and fmt clean;
`./example-benches/run.sh` re-measures byte-identical to the committed
tables (working tree clean after the run); history sweep sample
`--limit 60` converts 60/60 with 0 failures.** The architecture is sound
and stays: [rsmpl_patch](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/model.rs)
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
siblings. The gaps are evidence honesty, everyday-Rust matcher coverage,
and the remaining raising/veto surface — not correctness of what fires.

### Carry-forward audit of the previous plan

| Old item | Verdict | Evidence / residue |
|---|---|---|
| WI-1 canonical cycles | Landed | Tree-aware canonical form + repeat-fold detector + SPEC D.5 equalities exist; 24 `CYCLE_OK`, 16 pinned divergent (12 defined single-site demotions, `05` empty, `12`/`14`/`30` hetero) in [cycle_e2e.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/cycle_e2e.rs) |
| WI-2 trust envelope | Landed, evidence still over-claims | Per-site `sites`, v2 envelope, temp-copy `--verify build`, honest diffrs provenance present; `checked.semantics` is hardcoded `true` and binding truncation is unmarked (F1) |
| WI-3 repair + contract | Landed, generic tail weak | Five repair producers + `spec_examples` suite + diet rewrite landed; the long-tail filler echoes the English reason as `write_instead` (F10) |
| WI-4 sub-line raising | Landed | Occurrence-counted sub-line sweeps; fixture `27` cycle-OK; bench 28 at 89% |
| WI-5 sequences | Landed | Joint let (24), sequence (40), single-block (41) cycle-OK; `12` header+let mix declared a permanent non-goal in SPEC D.5 — accepted, not carried |
| WI-6 engine remainder | Landed | `async`/`try`/`unsafe`/`yield`, stmt interiors, `Fn(A) -> B`, verbatim macro types; the `Expr` arms in F2 were never in its scope |
| WI-7 rename completeness | Landed | Git-aware follow, one `pub use` hop, orphan evidence, `::`/qself parity both directions; glob/macro/shadow vetoes remain by design |
| WI-8 move + evolve | Partial | Header evolution for every item kind (slices 1–8) + signature-evolving `fn` moves (slice 9) landed; collection captures (slice ③) never landed (F6) |
| WI-9 macro depth | Not landed | `macro_rules!` rename, token-tree per-arg holes, and nested interiors are all still absent (F7) |
| WI-10 addressing + native | Not landed | No drift fallback, no `--context`, no sibling handshake, `--check` still archives to `.history/` (F8) |

### Findings

- **F1. The trust envelope still over-claims.** The v2 success payload in
  [applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
  prints `"checked": {"lowering": true, "semantics": true, …}`
  unconditionally: `semantics` is `true` even for a ranged rename where
  the only postcondition that exists — the sweep no-op gate in
  [postcheck.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck.rs),
  which covers only non-`optional` file-scope `# allow=many` hunks — never
  ran (probe-verified: a ranged `fn g` rename reports
  `semantics: true`). Per-site binding values are cut at 80 chars in
  `sites_json` with no marker: `sites_truncated` counts only omitted
  *entries*, so an agent cannot tell a truncated binding from a complete
  one and may act on half an expression.
- **F2. Everyday expressions silently veto whole patterns.** The general
  matcher in [matcher_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general.rs)
  (`match_expr`, 1997 lines) has no arms for `Expr::Macro`, `Expr::Repeat`
  (`[0u8; 8]`), `Expr::RawAddr` (`&raw const x`), `Expr::Const`
  (`const { … }`), or `Expr::Infer` (`_`), so each falls to `_ => false`.
  Probe-verified at HEAD: `- let b = format!("x{}", 1);` (byte-identical
  macro subterm) and `- let buf = [0u8; 8];` both fail with
  `no match found`, and the candidate preview names line 1 (the
  `# sweep` dummy start) — the agent learns nothing. SPEC 18.6 promises
  fail-fast, but these are match-time silent rejections. The verbatim
  precedent exists twice already: `Type::Macro` (WI-6c) and
  `Stmt::Macro` (WI-6b) match token-equal and never bind holes.
- **F3. Pattern positions accept only whole-pattern holes.**
  `match_pat` in [matcher_general_ctrl.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general_ctrl.rs)
  binds only a whole-pattern `P` (or `P @ sub`); every composite pattern
  compares token-equal. Probe-verified: `- let Some(P) = E;` over two
  real sites fails `no match found`. Destructuring — `Some(P)`, `(P, E)`,
  `S { f: P }` — is how Rust code actually binds, so `let`/`match`/`for`/
  closure patterns cannot abstract the positions agents most need to
  vary; C8 rejects embedded holes at compile time only for `let`, while
  arm/for/closure positions fail silently at match time. Raising is
  symmetric: the let detectors bind whole-pattern `P0` only.
- **F4. Impl/trait headers do not select the block they name.** SPEC 13.4
  documents headers as "advisory, not filters": under `# scope=file` the
  gate in [matcher_impl.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_impl.rs)
  is skipped entirely, and ranged matching falls back to token-containment
  plus range overlap. Probe-verified: a `# sweep` hunk headed
  `impl Tr for A {` renamed `fn m` in **both** `impl Tr for A` and the
  inherent `impl A` — the header an agent writes to scope the rewrite is
  ignored, and only the reported spans reveal it after the fact. The
  familiar spelling promises scoping the semantics does not deliver.
- **F5. Raising cannot see trait+impl method renames or hetero arg
  rewrites.** Probe-verified with `diffrs` at HEAD: renaming
  `greet` → `hello` across a trait declaration and two impls (3 sites,
  one intent) stays three ordinary hunks (SPEC D.5 pins this as `30`);
  wrapping `use_it(x)` → `use_it(x.clone())` at 3 sites stays ordinary,
  although the authored form `- use_it(E)` / `+ use_it(E.clone())`
  applies cleanly today — the engine is ready, only the detector is
  missing. Grouping in [group.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/group.rs)
  requires ≥2 *files*, so a same-file trait+impl intent could not share
  a `rename_*` rule even if the hunks were emitted.
- **F6. Collection captures never landed (old WI-8 ③).** SPEC 20.3/20.4/
  24.4 still make `# bind=<Name>` scalar-only and `# bind` + `# allow=many`
  a `RSMPL_CAPTURE_CONFLICT` (SPEC lines 3854, 3920, 4309; §32 lists
  collection captures as a non-goal, §33.2 as future work). Moving five
  items costs five range discoveries, five binds, and ten hunks authored;
  `diffrs` moves emit the same scalar fan-out.
- **F7. Macro depth never landed (old WI-9).** Item-rename kinds in
  [item_rename.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/item_rename.rs)
  omit `macro_rules!` (no rename in either direction); `# macro=token_tree`
  accepts only `NAME!(...)` verbatim bodies — probe-verified:
  `- greet!(E)` / `+ hello!(E)` fails at compile time with
  "supports only `NAME!(...)` with verbatim body"; macro interiors inside
  macro bodies stay ordinary (SPEC D.4), because `syn::visit` never enters
  opaque token streams and the token-tree matcher compares bodies whole.
- **F8. Addressing and the native side never landed (old WI-10).** A
  ranged patch authored against shifted line numbers fails with a range/
  match conflict and costs a full re-read (probe-verified: +3 drift lines
  → `RSMPL_MATCH_CONFLICT`); the lowering is always 0-context, so
  `--output` artifacts need `git apply --unidiff-zero`, not stock git;
  [gitrs](/home/ai/git/tools/rsmpl-git/bin/gitrs/src/main.rs) re-execs
  siblings with no version check; and `save_history` in
  [applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs)
  archives every run — including `--check` failures — into
  `<repo>/.history/` (128 kept), dirtying the worktree it measures
  (probe runs visibly littered `/tmp` probe repos). Separately, the
  `diffrs` JSON envelope is still `version: 1` with hand-built error
  payloads in [diffrs main](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs),
  while `applyrs`/`gitrs` emit `version: 2` — an agent must learn two
  schemas, and a revision range (`HEAD~2..HEAD`) silently downgrades
  every detector to section-visible evidence because
  [rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs)
  `pre_images` runs `git show <range>:<path>`, which always fails.
- **F9. Multi-rename sections stay ordinary.** `try_item_rename` fires
  only for a single pair (whole-section at
  [chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs):340,
  or one pair per balanced hunk); a `-U0` hunk holding eight distinct
  renames matches no chain detector and passes through whole — SPEC D.5
  fixture `14` re-infers as 1 semantic + 7 ordinary. Distinct deltas are
  distinct intents, but each is independently compressible; the missing
  piece is pair-level partitioning, not grouping.
- **F10. The generic repair filler echoes prose.** `fill_generic_repair`
  in [validate.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/validate.rs)
  fills any unpinned `PatternConflict`/`CaptureConflict` by copying the
  English `message` into `repair.write_instead`. The payload is present
  (WI-3's acceptance holds mechanically) but not machine-actionable: an
  agent cannot apply a sentence as a template. New fences must not
  repeat this, and the filler should key templates by error class.
- **F11. Field and variant renames have no shape at all.** Item renames
  cover item names only; there is no authored or raised form for
  renaming a struct field (declaration + accesses + literal/pattern
  labels) or an enum variant (declaration + constructions + patterns) —
  two of the most common Rust refactors — so both pay full ordinary
  bytes in both directions with no semantic precision. The member
  matcher can already bind field positions in expressions
  (`match_member`), but definition sites are unreachable.

## Key decisions

- **D1. Losslessness is judged per origin corpus, in writing.** Native-origin
  diffs owe state-losslessness (post-tree byte-identical). Semantic-origin
  patches owe canonical-losslessness for multi-site shapes and a *defined*
  fixed-point equality (`S″ == S′`) for single-site shapes the byte guard
  demotes. Raw native bytes never round-trip (0-context lowering,
  `index`/title stripping) — declared, as today. Binary/gitlink/non-UTF-8/
  CRLF semantic compression stays out of scope; passthrough cycles must
  remain tree-lossless.
- **D2. No success field may be true by construction.** Every `checked`
  claim must be computed by a check that can fail on real input
  (plus-instantiation at reported spans, per-site bindings, orphan
  enumeration). If a check cannot fail, it is removed or downgraded to
  an honest label (`reparse-only`, `partial`), never renamed.
- **D3. Inference stays precision-first.** Every new detector keeps
  pre-image totality (count must equal pair count, zero opaque hits) and
  the T375 byte-win guard measured in diet bytes. A detector that cannot
  prove totality stays ordinary — no guessing (SPEC 2.6).
- **D4. One anti-unification core, thin detectors.** WI-5 and WI-9 reuse
  the existing [antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
  core instead of growing per-shape string surgery. Engine work is
  structural `syn` matching — concrete, fail-loud, no expansion, no
  guessing.
- **D5. Split before growing.** New code lands in new modules; no item
  pushes a file over 2000 lines, and any file over 1600 lines touched by
  an item leaves under 1600 or with its new code in a sibling module.
  [matcher_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general.rs)
  at 1997 lines is split by WI-2 before any arm is added.
- **D6. Familiar syntax wins over clever syntax.** New surface reuses
  Git/SmPL/Rust spellings (`...`, `#` directives, concrete paths, context
  headers that look like the block they select). Diet remains canonical;
  the parser keeps accepting long forms so `.history/` archives stay
  readable. Any new spelling must be explainable in one AGENTS.md row.
- **D7. Breaking changes are taken where they buy trust.** JSON schema
  stays at its current version for additive keys; a key whose *meaning*
  changes bumps the version with the old key removed, per SPEC D.2's own
  rule. SPEC text moves with code in the same item.

## Work plan

Order is dependency order: WI-1 → WI-2 → WI-3 → WI-4 → WI-5 → WI-6 →
WI-7 → WI-8 → WI-9 → WI-10. WI-1 first because every later item emits
into the envelope it makes honest; WI-4's header filter precedes its own
raising half. Sizes are relative (S ≤ 1 day, M ≈ 2–3 days, L ≈ 4–5 days
of focused work).

### WI-1 (P0, M): Honest `checked` — per-rule postconditions, marked truncation [carries F1; hardens old WI-2]

**Problem.** F1/D2: `checked.semantics: true` is printed for every
success, including ranged renames where no postcondition ran, and
80-char binding truncation is invisible. An agent trusting the envelope
cannot distinguish "the engine re-verified the rewrite" from "the file
still parses".

**Decision — evidence that can fail, per rule.** (1) *Plus-instantiation
postcondition.* After materialization, every rewrite hunk's plus pattern
must re-match at each reported span in the post-image (for `# repeat`
hunks, at each outermost reported span; `# optional` zero-edit hunks are
`not-applicable`). Any span where the plus template does not instantiate
fails the run loud with `RSMPL_MATCH_CONFLICT` — this check can fail on
real input (attribute merges, evolution splices, capture pastes), unlike
the deleted-by-construction re-derivation it completes. The existing
sweep no-op gate stays as the second postcondition for file-scope
sweeps. Minus-must-be-zero is *not* adopted: widening rewrites
legitimately re-match their own output (`foo(E)` → `foo(foo(E))`,
probe-verified under the previous plan). (2) *Envelope.* Each `rules[]`
entry gains `semantics: "instantiated" | "postcondition" | "reparse-only"
| "not-applicable"` (`postcondition` = sweep gate + instantiation both
ran); the global becomes `checked: {lowering: bool, semantics:
"verified" | "partial"}` — `"verified"` iff every rewrite rule is
`instantiated`/`postcondition`, else `"partial"`. Insertions, cuts,
pastes, and ordinary hunks are `reparse-only` and force `"partial"`:
honest, never `true`. Schema bumps to `version: 3` (D7: a key's meaning
changes; the boolean is removed, not aliased). Text mode's `check:` line
gains the same word. (3) *Truncation honesty.* A binding value cut at
80 chars gains a trailing `…` and the rule entry gains
`bindings_truncated_chars: N` counting cut values; entry-count
`sites_truncated` keeps its meaning. Same treatment for `orphans` text.

**Changes.** SPEC D.2 (v3 envelope, budget recomputed), 24.6
(instantiation postcondition). Code: postcondition extension in
[postcheck.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/postcheck.rs)
(re-match via the existing matchers against the materialized tree —
no new matcher); per-rule semantics threaded through
[plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
into `RuleReport`; envelope + truncation markers in
[applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs).

**Acceptance.** The identity-sweep fixture still fails loud; a new
tampered-instantiation unit test proves `semantics` can fail (plus
template that cannot re-match its own span); JSON pins show a ranged
rename as `instantiated`, a sweep as `postcondition`, an insertion
patch as global `"partial"` (never `"verified"`); an 81-char binding
pins the `…` marker and count. Report overhead stays under the
recomputed D.2 budget pin.

**Tests.** V3 envelope shape pins (success + verify paths); per-rule
semantics matrix (rename / sweep / insertion / optional / repeat);
instantiation fire/no-fire units; truncation marker pins; full
workspace + cycle harness.

**Non-goals / risks.** No type-level proof beyond opt-in
`--verify build`; no minus-zero check (widening counterexample above).
Risk: instantiation re-matching costs a second engine pass per rewrite
hunk — bounded by reported spans (point re-matches, not file rescans);
pinned by the bench suite staying green.

### WI-2 (P0, M): Everyday expressions — verbatim macro/repeat/raw/const arms + compile-time shape lint [carries F2; closes old WI-6 residue]

**Problem.** F2: a byte-identical `format!(…)` or `[0u8; 8]` inside a
pattern silently vetoes the whole match, and the failure preview points
at line 1. These are not exotic shapes; they are the default texture of
Rust bodies, so general/`let` patches stop exactly where agents work.

**Decision — verbatim where opaque, structural where safe, loud where
unsupported.** (1) *New `match_expr` arms* in the WI-6 style:
`Expr::Macro` matches verbatim-only (`render_eq`, never holed — the
`Type::Macro`/`Stmt::Macro` precedent; a substituted hole inside can
never render-equal, so holes reject by construction); `Expr::Repeat`
recurses on element and length expressions; `Expr::RawAddr` compares
mutability strictly and recurses; `Expr::Const` matches its block via
`match_block`; `Expr::Infer` (`_`) matches any expression and binds
nothing (plus-side `_` writes literally, mirroring `Type::Infer`).
(2) *Compile-time shape lint (new C19).* Pattern compilation walks the
minus/plus `syn` trees; any remaining unsupported node in a *pattern*
position (`Expr::Verbatim`, a declared hole inside a macro body, a
`Pat`/`Type` verbatim) fails at `RSMPL_PATTERN_CONFLICT` with the exact
node kind and write-instead, carrying a `fix_pattern` repair with a
concrete template — never a match-time `no match found`. The zero-match
preview for general/call/`let` shapes gains a shape-aware tier: the
nearest line containing the pattern's head token (callee/macro name/
`let`), replacing the line-1 fallback for `# sweep` hunks.
(3) *Repair hygiene (F10).* `fill_generic_repair` stops copying the
English message into `write_instead`: the filler keys one concrete
template per error class (`pattern` → the C-row's write-instead text,
`capture` → the missing-name form, `symbol` → `# fallback=syntax`),
and a conformance pin asserts no emitted `repair.write_instead` equals
its own `reason`. (4) *Split first (D5):* `match_type`/`match_path` move from
[matcher_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general.rs)
(1997 lines) into a new `matcher_types.rs`; the file leaves under
1600 lines.

**Changes.** SPEC 18.6 (R1 row gains the five arms; new C19 row; R-table
"permanent scope" sentence narrowed), D.2 (C19 repair shape). Code: arms
+ lint in `matcher_general.rs` / new `matcher_types.rs`; lint walk in
[pattern_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/pattern_general.rs)
(compile keeps the `syn` trees); preview tier in
[suggest.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/suggest.rs).

**Acceptance.** Both HEAD probes now match: `- let b = format!("x{}", 1);`
rewrites under `# sweep`, and `- let buf = [0u8; 8];` →
`[0u8; 16]` applies; a hole-inside-macro pattern fails C19 at compile
time (text + JSON pins, target file untouched); new applyrs bench 28
(macro-bearing `let` sweep over 4 sites) ≥ 40% saved. History sweep
0 failures; compressing set does not shrink.

**Tests.** Per-arm matcher units (verbatim fire, hole veto, repeat
length-hole fire, raw-addr mutability veto, const-block fire); C19
compile pins; preview-tier pins (head-token line, not line 1); SPEC
18.6 verbatim expectations move rows, never deleted.

**Non-goals / risks.** No macro expansion (SPEC 21.5 stands); no holes
inside macro bodies in the general matcher (token-tree holes are WI-7).
Risk: verbatim macro equality is whitespace-sensitive via `render_eq`
token normalization — same posture as the two existing precedents,
pinned by a spacing-variant unit.

### WI-3 (P1, M): Structural pattern positions — destructuring holes in `let`/`match`/`for`/closures [carries F3]

**Problem.** F3: `let Some(P) = E;` cannot match `let Some(v) = opt;`.
Agents must either spell every binding concretely (no fan-out) or drop
to ordinary hunks for the most common binding shape in Rust.

**Decision — bounded composite `Pat` matching, one core.** `match_pat`
gains structural arms, tried only after the whole-`P` and `P @ sub`
fast paths: `TupleStruct` (path token-equal, elements recurse),
`Tuple` (arity-equal, elements recurse), `Struct` (path token-equal,
fields matched by member name with recursion, `..` rest presence must
agree on both sides), `Paren`, `Reference` (mutability strict), and
`Slice` (arity-equal, elements recurse). `Or` stays concrete-only
(token-equal) in v1 — binding under alternation is ambiguous and is
rejected at compile time when it carries a hole (C8 text updated, same
posture for every pattern position, not just `let`). Plus-side
instantiation reuses the existing text substitution over recorded
binding spans; no new renderer. Raising follows the engine: the let
detectors' pattern generalization in
[antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
gains `gen_pat` (same-variant composites at depth ≤ 2, ≥ 1 literal
anchor — the variant path — repeated-hole equality per SPEC 15.7), so
joint let templates with destructuring deltas raise instead of staying
ordinary.

**Changes.** SPEC 15.1/15.7 (composite pattern holes), 18.6 (C8
narrowed uniformly across positions), D.4 (let-template bullet). Code:
`match_pat` arms in
[matcher_general_ctrl.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general_ctrl.rs);
C8 gate unification in pattern compilation; `gen_pat` in
`antiunify.rs` consumed by `try_let_template_sweep` / single-block.

**Acceptance.** The HEAD probe fires: `- let Some(P) = E;` /
`+ let Some(P) = E.unwrap_or(0);` rewrites both sites under `# sweep`
and re-diffs cycle-green; a struct-pattern fixture
(`S { f: P }`) and a tuple fixture fire; an `Or`-with-hole pattern
fails C8 at compile time in `let`, arm, and closure positions alike.
New diffrs bench 30 (destructuring let migration, 3 sites) ≥ 50% saved.

**Tests.** `match_pat` matrix per arm (fire, arity veto, rest-presence
veto, nested composite); C8 position-uniformity pins; anti-unify
`gen_pat` fire/veto units; fixtures round-trip real-apply; cycle harness.

**Non-goals / risks.** No `Or` holes, no guard-position holes beyond
the existing expression matcher, no `@` except the existing
placeholder-head form. Risk: field-order sensitivity in `Struct`
patterns — matching is by member name, source order irrelevant,
pinned by a reordered-fields unit.

### WI-4 (P0/P1, L): Impl/trait headers select their block — and raising learns trait+impl renames [carries F4 + F5-trait; closes D.5 `30`]

**Problem.** F4+F5: the header spelling promises block selection the
engine does not deliver (sweep renames across every impl), while the
inverse direction cannot compress a trait+impl method rename at all.
Both directions of the same context shape are untrustworthy.

**Decision — header is a filter in every scope; one detector emits the
grouped intent.** (1) *Engine.* The impl/trait header is parsed once at
compile time into (kind, self-type tokens, trait-path tokens) and
compared token-normalized against each candidate block's `syn` header:
inherent impls require equal self type and no trait; trait impls
require both halves equal; trait headers require equal trait path.
The comparison applies under *every* scope including `# sweep`; the
range-overlap fallback and the token-containment heuristic are deleted.
This is a breaking SPEC 13.4 change: "advisory" becomes "filter", which
is what the spelling has always suggested (D6). Bench 19/20 headers
(`impl Svc`, trait sweeps) still match their named blocks.
(2) *Raising.* New detector `try_trait_method_rename`: pairs spanning
one trait block (declarations `fn m(…);` and defaults) plus impl
definitions of the same trait, all sharing one ident delta with
per-site suffixes preserved, ≥ 2 sites with ≥ 1 trait-side and ≥ 1
impl-side (pure impl-only classes stay with the slot/impl sweeps).
Emission is one trait-context hunk plus one impl-context hunk per
distinct impl block (headers verbatim from the pre-image, now real
filters per (1)), all sharing `rule=rename_Old_to_New`: grouping in
[group.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/group.rs)
is extended so this detector (only) may group within one file;
canonical form already preserves shared `rename_*` names. Totality is
per-hunk engine-count: the trait minus matches exactly its trait sites,
each impl minus exactly that block's definitions.

**Changes.** SPEC 13.4 (header filter, breaking), 29.9, D.4
(trait-method detector bullet), D.5 (`30` re-pinned — see Acceptance).
Code: header parse/filter in
[matcher_impl.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_impl.rs)
(header struct in `rsmpl_pattern` beside `ImplContext`); detector in
new `crates/rsmpl_diff_infer/src/semantic/trait_methods.rs`; grouping
extension in `group.rs`; chain registration in
[chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs).

**Acceptance.** The HEAD over-match probe now rewrites only
`impl Tr for A` under `# sweep` (inherent `impl A` untouched, pinned);
the HEAD raising probe compresses trait + 2 impls into grouped hunks
and real-applies byte-identical; a divergent-suffix fixture (one impl
changes its signature too) vetoes to ordinary. Fixture `30` moves to
`CYCLE_OK` if the authored form canonicalizes to the raised form under
the tree-aware comparator, else stays pinned with the sharpened reason
named here (authored single-hunk vs raised multi-hunk intent).

**Tests.** Header-filter matrix (inherent vs trait impl, generic self
type, sweep scope); raising fire/veto units (decl-only stays ordinary,
impl-only veto, mixed-signature veto);
grouped same-file rule pins; fixtures + cycle harness.

**Non-goals / risks.** No cross-trait unification (two traits declaring
the same method stay two hunks), no supertrait chasing. Risk: strict
header equality breaks patches that relied on advisory sloppiness —
intended (D7); the failure mode is a loud zero-match with the WI-2
shape-aware preview, not a silent wrong rewrite.

### WI-5 (P1, M): Hetero call-arg templates — raising learns wrap/transform migrations [carries F5-calls]

**Problem.** F5: `foo(x)` → `foo(x.clone())` at N sites is a canonical
API migration, applies cleanly when authored, and never raises. The
detector family covers callee deltas, concrete arg additions, and
concrete literal swaps — everything except a uniform *wrapper* around
a varying argument.

**Decision — anti-unify argument positions, engine already ready.** New
detectors `try_call_arg_template_sweep` (calls) and its method sibling:
N ≥ 2 pairs, uniform arity, callee token-equal *or* one shared callee
delta (disjoint from `call_sweep`, which keeps token-equal-args
classes), where each argument position independently is (a) token-equal
concrete, (b) a whole-arg hole `E{i}`, or (c) a uniform wrapper — one
`E{i}` occurrence inside a plus template with ≥ 1 literal anchor
(`E.clone()`, `&E`, `f(E)`), generalized by extracting the existing
`antiunify_plus_with_hole` core from the let-template code in
[antiunify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/antiunify.rs)
(D4). Plus templates may permute holes (`f(E0, E1)` → `f(E1, E0)`);
dropped or duplicated holes veto (a duplicated hole would need
repeat-equality the emission cannot state). At least one position must
carry a real delta; macro-hiding arguments veto (T448 precedent);
all-identical classes stay with verbatim. Totality is the real-pair
engine probe (`template_pair_holds`): the emitted pair must match
exactly N sites in the pre-image. Chain placement: after
`call_literal` / `method_same_literal`, before the cross-hunk verbatim
stage (tighter templates win over identical-line grouping).

**Changes.** SPEC D.4 (call/method arg-template bullets, detector names
`call_arg_template`, `method_arg_template`). Code: shared arg-template
core in `antiunify.rs`; detectors in
[calls.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/calls.rs)
and [methods.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/methods.rs)
(no net growth — shared core only); provenance names registered in
[prov.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/prov.rs).

**Acceptance.** The HEAD wrap probe compresses (3 sites, one hunk) and
real-applies byte-identical; new diffrs bench 31 (wrap migration,
5 sites) ≥ 50% saved; arg-swap fixture fires with permuted holes;
duplicated-hole and hetero-tail fixtures veto to ordinary. Authored
round-trip of the emitted form is pinned (it already applies today).

**Tests.** Anti-unify arg matrix (wrap, borrow, permute, drop veto,
duplicate veto, macro-arg veto); detector fire/veto incl. hetero
distractor site (untouched twin vetoes via totality); fixtures +
cycle harness; history compressing set reported.

**Non-goals / risks.** No statement-level templates beyond the call
(the sequence/`let` detectors own statements), no arity-changing
wrappers (arg-add owns concrete tails). Risk: wrapper over-generality
— the literal-anchor + real-pair totality gates are acceptance
criteria, not aspirations.

### WI-6 (P1, L): Collection captures — `# bind` + `# allow=many` moves many items as one [carries F6; completes old WI-8]

**Problem.** F6: multi-item moves — the largest patches there are —
cost one scalar capture per item in both directions, five range
discoveries for five helpers. SPEC 20.3 explicitly defers the syntax;
this item defines it with directives agents already know (D6).

**Decision — ordered collections from existing directives.** (1) A cut
hunk combining `# bind=Name` with `# allow=many` (today a
`RSMPL_CAPTURE_CONFLICT`) binds an *ordered collection*: every match in
scope, in source order, each the exact item span as scalar cuts record
today. Match-count rules apply to the collection (zero without
`# optional` fails; `# optional` binds the empty collection). (2) A
paste hunk (`# use=Name`, pure insertion, `+ Name`) splices all items
at the anchor in source order with the SPEC 20.2 one-blank join; the
same collection may be pasted more than once (scalar fan-out parity).
Evolution pastes (`+ <prefix>fn <New>(...) { ... }`) with a collection
capture fail at `RSMPL_CAPTURE_CONFLICT` — one header template cannot
evolve many items unambiguously. (3) The cut rule's report carries
`matches` = item count and one span per item (WI-1 `sites` evidence
extends naturally). (4) `diffrs` move inference emits the collection
form when ≥ 3 items move between the same file pair and its bytes beat
the scalar spelling under T375 (both spellings measured; smaller wins).

**Changes.** SPEC 20.3/20.4 (collection semantics; the "invalid in
v0.1" fences rewritten), 24.4, §32/§33.2 (non-goal/future entries
removed), D.4 (move bullet). Code: collection store beside scalar in
[capture.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/capture.rs)
(collections in new `capture_many.rs`, D5); planner pass-1 collection
cuts in [plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs);
validation updates in
[validate.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/validate.rs);
move emission choice in
[moves.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/moves.rs)
(no net growth — emission helper in `move_parts.rs`).

**Acceptance.** Authored five-item cross-file move is one cut + one
paste, applies byte-identical, and re-diffs cycle-green; paste-order
and blank-join pins match SPEC 20.2; a collection + evolution paste
fails with the exact capture diagnostic; new diffrs bench 32
(five-item move) ≥ 70% saved; every scalar move fixture and bench
unchanged byte-for-byte.

**Tests.** Collection cut/paste units (ordering, empty-optional,
double paste, evolution veto, twin veto still applies per item);
planner overlap pins (collection cut vs ordinary hunk on same lines);
move round-trip fixtures; cycle harness.

**Non-goals / risks.** No statement collections (items only), no
per-item evolution, no list-separator syntax invention. Risk:
collection cuts widen the blast radius of one hunk — mitigated by
per-item spans in the report (WI-1) and the unchanged overlap checks;
a partially-overlapping collection fails whole, never partially.

### WI-7 (P2, M): Macro depth — `macro_rules!` renames, token-tree holes, one-level interiors [carries F7; completes old WI-9]

**Problem.** F7: macros are Rust's largest opaque surface. Definitions
cannot be renamed, authored token-tree patterns cannot abstract one
argument, and edits inside macro bodies never compress — so macro-heavy
changes (the `17`/`18` benches sit at 46%/41%, the worst in the suite)
pay near-ordinary bytes.

**Decision — deepen by rendering, never by expansion (SPEC 21.5
stands).** (1) *`macro_rules!` rename.* Item rename gains the
`macro_rules` kind in
[item_rename.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/item_rename.rs)
(`- macro_rules! Old` / `+ macro_rules! New`; definition span is the
full item, body preserved byte-verbatim like fn bodies). `# follow=uses`
extends to invocation sites (bare and qualified last-segment) via the
existing macro collector; `diffrs` gains the single-pair detector arm
(`macro_rules_rename`, groupable like `macro_rename`). (2) *Token-tree
holes.* `# macro=token_tree` patterns may name one metavariable per
top-level comma-separated argument (`- old!(E, lit)` /
`+ new!(E, lit)`), matched by the existing token-tree argument matcher
in [matcher_macro.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_macro.rs);
`...` remains the whole-body form; delimiters stay significant. The
verbatim-only compile fence is narrowed to multi-hole/non-argument
positions with updated C-table text. (3) *One-level interiors in
raising.* The macro-interior detector emits token-tree hunks (concrete
or single-hole per (2)) for same-name macro pairs whose bodies differ
in exactly one top-level argument — reaching one level inside bodies
the `syn` visitor cannot enter, matched by the token-tree engine that
already compares bodies argument-wise. Recursive token descent and
macros-inside-strings stay ordinary (strings are opaque literals).

**Changes.** SPEC 14.8 (kind list), 21.3 (holes + interior posture),
D.4 (macro bullets). Code: kind arm in `item_rename.rs` +
[matcher_item.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_item.rs);
follow collector arm in
[use_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph.rs);
token-tree hole gate in `matcher_macro.rs`; interior emission in
[macros.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/macros.rs)
(extraction helpers in existing `macro_parts.rs`; no net growth).

**Acceptance.** Benches 17/18 each gain ≥ 5 points saved; new bench 33
(`macro_rules!` rename + 4 invocation sites with follow) ≥ 60% saved;
depth-2 body edit (macro inside a macro argument's tokens) stays
ordinary (veto pin); string-hidden macro text never counts. History
sweep 0 failures, compressing set reported.

**Tests.** macro_rules authored + raised + follow + distractor pins;
token-tree hole fire / delimiter-mismatch veto / multi-hole veto;
interior one-level fire + two-level veto; raw-span slicing fallback
pin; cycle coverage.

**Non-goals / risks.** No expansion, no `macro` 2.0 items, no hygiene
reasoning, no recursive token-tree descent. Risk: token-tree argument
splitting on unbalanced bodies — the existing string-aware splitter is
the only splitter, and split failure declines to ordinary.

### WI-8 (P1, M): Pair-partition raising — multi-rename sections compress pair by pair [carries F9; closes D.5 `14`]

**Problem.** F9: eight renames in one file are eight intents, each
individually compressible, yet the section passes through ordinary
because every rename detector demands the whole section (or whole
hunk) share one shape.

**Decision — partition balanced hunks by single-pair detectors.** When
the pair chain declines a balanced hunk with ≥ 2 pairs, a partition
pass tries each pair singly through the single-pair rename detectors
(item rename incl. its plain-`fn` arm; a `{-terminated fn header pair
keeps today's single-pair gate). The hunk partitions only if *every*
pair is claimed (hunk atomicity, WI-9e fixed-point precedent: a mixed
hunk stays ordinary whole, never half-semantic). Emission is one file
section with one hunk per pair (tight per-pair ranges from the changed
lines, positional `rN` rules; distinct deltas never group — `group`
stays `""`). Whole-section and per-hunk chain results are untouched:
partition runs only where both declined, so existing fires keep their
exact bytes.

**Changes.** SPEC D.4 (partition paragraph), D.5 (`14` re-pinned).
Code: partition pass in
[chain.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/chain.rs)
`compress_section_per_hunk` fallback, reusing the single-pair entry in
[rsmpl_diff_infer lib](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/lib.rs);
provenance per emitted hunk via existing `prov.rs` accounting.

**Acceptance.** A new diffrs fixture with 8 mixed item/fn renames in
one section compresses all 8 and real-applies byte-identical; a mixed
fixture (7 renames + 1 unclassifiable pair) stays ordinary whole
(atomicity pin); fixture `14`'s tree legs hold and its D.5 entry moves
to the defined partition form (or `CYCLE_OK` if the tree-aware
canonical forms converge — the landing change states which).

**Tests.** Partition fire/veto matrix (all-claimed, one-unclaimed,
fn-header gate per pair, cross-hunk pairs); T375 per-hunk byte pins
(a partition that costs more than ordinary declines); fixtures +
cycle harness; history compressing set reported.

**Non-goals / risks.** No partitioning for sweeps/templates (classes
that must share one template keep whole-hunk semantics), no divergent-
delta grouping. Risk: many small hunks cost more than one ordinary
hunk on tiny renames — the per-hunk T375 guard is the gate, pinned.

### WI-9 (P1, L): Field and variant renames — definition, uses, and raising [carries F11]

**Problem.** F11: renaming `Config.timeout` or `Color::Red` is a daily
refactor with no semantic spelling. Agents hand-write every site as
ordinary hunks, and `diffrs` shows them the same bytes back.

**Decision — context-pair shapes in the impl-context spelling family
(D6).** (1) *Authored.* Two context shapes reuse the header+pair form
agents already know from §29.9: a variant rename is an `enum` header
context with a bare ident pair (suffix — discriminant, fields —
preserved token-equal; only the ident span edits); a field rename is a
`struct` header context with a `- Old: T` / `+ New: T` pair (type
token-equal preserved, ident-span edit; named fields only). Both
classify as rename kinds (`VariantRename`, `FieldRename`) so
`# follow=uses`, orphan evidence (WI-7c plumbing), and report spans
come along without new directives. Follow sites are syntactic:
variants — `Enum::Old` paths in expressions and patterns; fields —
`.old` accesses, struct-literal labels (shorthand `S { old }` rewrites
to the expanded `New: old`, pinned — the local binding keeps its name),
and struct-pattern labels. Multi-definition ambiguity vetoes/skips
exactly as fn renames do (SPEC 14.8). (2) *Raising.* A detector pairs
the definition-line delta with use-site lines sharing the ident delta
and emits the definition hunk *with* `# follow=uses` only when a
scratch-apply probe proves the follow rewrite reproduces exactly the
changed use lines (template-probe totality); otherwise the definition
hunk alone raises and uses stay ordinary.

**Changes.** SPEC 13.4 (two context shapes), 14.8 (follow-up table +
follow semantics for members), D.4 (detector bullets). Code: context
classification in
[pattern_classify.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/pattern_classify.rs);
matching in new `crates/rsmpl_engine/src/matcher_member.rs`; follow
collectors in [use_graph.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/use_graph.rs)
(new `use_graph_member.rs` sibling, D5); detector in new
`crates/rsmpl_diff_infer/src/semantic/member.rs`.

**Acceptance.** Authored field rename + follow rewrites declaration,
accesses, literal labels (incl. shorthand expansion pin), and pattern
labels in one patch; without follow, JSON `orphans` lists the sites
while `--check` stays green. Variant rename covers construction and
match-pattern sites. New diffrs benches 34 (field rename, 6 sites) and
35 (variant rename, 5 sites) each ≥ 50% saved, real-apply identical.

**Tests.** Definition matcher matrix (discriminant/field-suffix
preservation, tuple-variant name-only, union fields veto — unions keep
ordinary); follow fire/veto (shadowed field name, shorthand, pattern
labels); raising probe-totality veto (follow would touch an unchanged
site → definition-only); fixtures + cycle harness.

**Non-goals / risks.** No tuple-struct positional fields, no rename of
locals/params (ordinary hunks), no cross-crate resolution. Risk:
field-name collisions across structs — the header names the struct,
follow is definition-anchored (only uses of *that* struct's fields
provable by literal/struct-path spelling rewrite; unprovable `.old`
accesses on unknown receivers are orphan evidence, never rewritten —
stated in SPEC, pinned by a same-name-other-struct fixture).

### WI-10 (P1, L): CLI contract — drift-tolerant addressing, stock-git output, sibling handshake, honest diffrs envelope [carries F8; completes old WI-10]

**Problem.** F8: patches fail for the wrong reason (line drift), the
"native" output needs a non-stock flag, sibling binaries can skew
silently, measuring a repo dirties it, and the two JSON envelopes
disagree. Each leak costs an agent a round-trip or a wrong conclusion.

**Decision — five bounded fixes, no new subcommands.** (1) *Anchor
fallback.* When a ranged semantic hunk finds zero matches in its cited
range, the engine retries once with file scope *anchored* by the hunk's
body context lines (leading/trailing ` ` lines as literal anchors, in
order): exactly one anchored candidate → apply it and report
`"drift": {rule, cited: [s,e], actual: [s,e]}` in JSON (one `drift:`
stderr line in text mode). Zero or ≥ 2 → the original
`RSMPL_MATCH_CONFLICT` stands unchanged. `# sweep`/`# scope=file`
hunks never drift-report (they made no locality claim). Emitted edits
still cite original coordinates; WI-1 postconditions still gate.
(2) *Stock-git output.* `applyrs --context N` (default 0, SPEC D.2.2
canonical form unchanged) renders the *reported/`--output`* diff with
N context lines via the existing `*_with_context` builders in
[rsmpl_git diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_git/src/diff.rs),
so `--output` artifacts apply with vanilla `git apply`; apply/`--check`
internally always use the 0-context lowering. (3) *Sibling handshake.*
`gitrs` reads each sibling's `--version` before re-exec and fails with
`RSMPL_IO_CONFLICT` naming both versions on mismatch; `GITRS_*_BIN`
overrides (test doubles) skip the check. (4) *Clean measurement.*
`--check` runs skip the `.history/` archive unless `--history` is
passed; real applies keep always-archive; JSON reports
`"history": null` when skipped. (5) *Envelope parity + rev honesty.*
`diffrs` JSON bumps to `version: 2` with the same error object as
`applyrs` (keys `code/rule/file/range/reason/suggestion/repair`, via
`rsmpl_patch::StructuredError` — the hand-built payload in
[diffrs main](/home/ai/git/tools/rsmpl-git/bin/diffrs/src/main.rs) is
deleted); revision ranges `A..B`/`A...B` resolve their pre-image base
to `A` in [rsmpl_diff](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff/src/lib.rs),
and a failed pre-image fetch for a changed `.rs` file warns on stderr
naming the `proven` downgrade instead of silently weakening totality.

**Changes.** SPEC 10/17 (anchor fallback + drift evidence), D.1
(history-on-apply), D.2 (`drift`, `--context`, diffrs v2), D.2.2
(presentation-context note), D.4 (rev-range base). Code: anchored retry
in [plan.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/plan.rs)
beside the T62 whole-file probe (its count machinery becomes the
fallback when anchors disambiguate); context plumbing + history gate +
drift field in [applyrs main](/home/ai/git/tools/rsmpl-git/bin/applyrs/src/main.rs);
handshake in [gitrs main](/home/ai/git/tools/rsmpl-git/bin/gitrs/src/main.rs);
envelope + base resolution in diffrs main / `rsmpl_diff`.

**Acceptance.** A patch authored 20 lines stale applies with correct
`drift` evidence and byte-identical post-tree; the ambiguous-drift
fixture (two anchor-satisfying candidates) fails with the original
conflict and applies nothing. `--output --context 3` applies under
vanilla `git apply` in an e2e pin. Mismatched-sibling fixture fails
loud; `--check` leaves no `.history/` entry, apply leaves exactly one.
`diffrs HEAD~2..HEAD --format json` on a compressing range reports
`proven: true`; diffrs error JSON carries the same keys as applyrs.

**Tests.** Drift fire/ambiguous/no-anchor matrix; context vanilla-apply
pin; handshake pass/fail pins; history gating pins; envelope key-parity
pin (applyrs vs diffrs error objects); rev-range provenance pin.

**Non-goals / risks.** No fuzzy/textual matching (anchors are literal,
patterns still semantic), no original-coordinate change (SPEC 10/23.4
stand), no two-revision `diffrs A B` form (ranges cover it). Risk:
anchored retry masks a genuinely wrong range — the exactly-one gate
plus always-visible drift evidence are the mitigations.

## Sequencing and gates

| Item | Depends on | Size | Gate before next item |
|---|---|---|---|
| WI-1 honest `checked` | — | M | V3 pins green; tampered-instantiation test proves failure is reachable |
| WI-2 everyday expressions | — (split first) | M | HEAD probes F2 match; C19 pins; bench 28 ≥ target |
| WI-3 structural patterns | WI-2 lint | M | Destructuring probe fires; C8 uniformity pins; bench 30 ≥ target |
| WI-4 headers + trait raising | WI-2 preview | L | Over-match probe scoped; raising probe grouped; `30` re-pinned |
| WI-5 hetero arg templates | — | M | Wrap probe compresses; bench 31 ≥ target; veto matrix green |
| WI-6 collection captures | WI-1 spans | L | Five-item authored + raised moves cycle-green; scalar pins unchanged |
| WI-7 macro depth | WI-2 | M | Benches 17/18 +5 pts; bench 33 ≥ target; depth vetoes pinned |
| WI-8 pair partition | — | M | 8-rename fixture compresses; atomicity pin; `14` re-pinned |
| WI-9 field/variant renames | WI-1 orphans | L | Benches 34/35 ≥ targets; shorthand + collision pins green |
| WI-10 CLI contract | WI-1 envelope | L | Drift matrix green; vanilla-apply pin; envelope parity pin |

Global validation per item: `cargo test --workspace`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`./example-benches/run.sh` (README tables regenerated only when bytes
change), and `python3 tools/history_sweep.py` (0 failures; compressing
set reported, never asserted to a fixed number beyond "does not shrink
without a named reason"). No item lands with a red gate, a new `.rs`
file over 2000 lines, or an existing file pushed over 2000 lines.

## Risks / open questions

Per-item risks are stated with each item. Open questions: none —
scope, canonical form, detector order, guards, thresholds (64-line
range rule, 80% move pairing, 80-char binding truncation, collection
threshold 3, macro interior depth one token level), schema versioning
(v3 in WI-1, diffrs v2 in WI-10), and acceptance numbers are decided
here. If a threshold proves wrong against the probe corpora, the fix
is a new pin plus a SPEC note, not a silent constant edit. Two
deliberate non-goals bear repeating because they bound several items:
no semantic resolver is built (SPEC 19.3 exact-text matching stands;
WI-9 hardens the syntactic graph instead), and no macro expansion is
attempted anywhere (SPEC 21.5 stands; WI-7 deepens rendering-level
matching only).

## Considered and rejected

- **Full type-directed resolution (rust-analyzer integration).** Would
  make `# symbol` and follow=uses "really" correct, but couples every
  `--check` to a build environment, breaks the no-`cargo`-by-default
  cost model, and is un-auditable in a hackathon prototype. Syntactic
  precision + orphan evidence (WI-9) buys most of the trust.
- **General `...` in expressions/types/statements.** Rejected beyond
  the existing trailing-block and WI-3 composite positions:
  unrestricted ellipsis turns precise patterns into guesses and breaks
  totality counting.
- **Dropping T375 for single-site semantics.** Rejected as before:
  bytes are the token proxy agents pay; single-site safety shapes
  (renames, moves) already fire. Cycle honesty for demoted singles is
  bought with a defined equality (D1), not larger patches.
- **Prose-echo repairs for new fences.** Rejected (F10): every new
  compile-time fence ships a concrete `write_instead` template or does
  not ship; the generic filler keys templates by error class instead
  of copying the message.
- **Recursive token-tree macro descent.** Rejected in WI-7: one
  argument level is provable with the existing token-tree matcher;
  deeper descent invents a second macro engine inside the first.
- **CRLF/binary semantic compression.** Stays declared-lossy (D1):
  detectors strip `\r` by construction and patches are UTF-8;
  passthrough tree-losslessness is the correct contract there.


