# IMPROVE — RSmPL token efficiency, diffrs conversion, lossless cycles

## Goal

Make RSmPL smaller to author and emit, make `diffrs` compress far more
real native diffs, and make both directions lossless under a tested
canonical definition. Backwards compatibility is not required;
breaking changes to `SPEC.md` and emitted patch text are allowed.

## Success criteria

1. Every `example-benches/applyrs/*` and `diffrs/*` bench is
   byte-smaller as RSmPL than as native, including today's losers
   `09-sig-evolution` (-1%) and `10-item-rename` (-7%).
2. `tools/history_sweep.py`: 0 failures (today 392/392 convert) and
   the compressing set grows from 34 commits; `cargo test
   --workspace`, `cargo clippy --all-targets`, `cargo fmt --check`
   stay green; no `.rs` file exceeds 2000 lines.
3. Lossless, defined in WI-4: state-lossless native → semantic →
   native (post-tree byte-identical) and canonical-lossless semantic
   → native → semantic (canonical forms equal, second cycle is a
   fixed point), over the in-scope corpus of WI-4.

## Review findings (evidence)

Architecture is sound and heavily tested (109 suites, 1179 tests):
`crates/rsmpl_patch` (document/parser, [model.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/model.rs),
[parser.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_patch/src/parser.rs)),
`crates/rsmpl_pattern` (pattern compiler,
[pattern.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_pattern/src/pattern.rs)),
`crates/rsmpl_engine` (matchers + original-coordinate planner,
[lib.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/lib.rs)),
`crates/rsmpl_git` (0-context lowering), `crates/rsmpl_diff_source`
(`git diff -U0` + untracked), `crates/rsmpl_diff_infer` (detector
chain, [semantic/mod.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/mod.rs)),
facaded by `crates/rsmpl_diff`; `bin/gitrs` only re-execs siblings.
The gaps are surface verbosity, inference breadth, and cycle identity
— not correctness of what already fires.

- **F1 Verbose emission dominates small hunks.** A diffrs call sweep
  (`diffrs/01`, 111 B) spends 33 B on `rule=calls_foo_to_bar`, 13 B
  each on `# allow=many` / `# scope=file`, 10 B per `# expr E{i}`;
  payload `- foo(E0)` / `+ bar(E0)` is 20 B. Slot sweeps always emit
  four slot declarations (`# visibility V`, `# qualifier_list Q`,
  `# generic_params G`, `# where_clause W` = 70 B) even for plain
  sites. This is why single-ident benches lose to native.
- **F2 Inference is narrow by construction.** Every detector in
  `rsmpl_diff_infer` requires N ≥ 2, balanced single-line pairs, one
  shared delta; singles survive only via the T375 byte-win guard
  (18 of 60 diffrs fixtures are `*-stays-ordinary` veto pins).
  History agrees: only 34/392 commits compress at all; the rest are
  correct ordinary passthrough with near-zero savings (macro benches
  17/18 save only 14–19%).
- **F3 Engine can apply shapes diffrs never emits.** `call_parts`
  rejects multi-segment, leading-`::`, `<T>`-turbofish and `qself`
  callees ([mod.rs:875](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/mod.rs:875));
  totality counters repeat those exclusions. The engine already
  matches qualified paths, structural turbofish and macro renames
  (applyrs benches 16/17), so this is infer-only work.
- **F4 No anti-unification.** Verbatim sweeps need byte-identical
  lines; call/let/type changes whose sites differ in arguments,
  types or patterns stay ordinary. There is no type sweep, no
  signature-evolution sweep (`fn_rename_pair` requires byte-equal
  suffixes), no macro sweep in diffrs.
- **F5 Cycles are one-directional.** The harness proves native →
  semantic → native by real-apply byte-compare
  ([fixtures_roundtrip.rs](/home/ai/git/tools/rsmpl-git/bin/diffrs/tests/fixtures_roundtrip.rs),
  history_e2e). Nothing tests semantic → native → semantic:
  diffrs always re-emits `@@ -1,1 @@` + `# scope=file`, renames
  rules deterministically but verbosely, and drops `index` lines,
  hunk titles and header form — authored locality and intent are
  unrecoverable, and no canonical form defines "same patch".
- **F6 General matcher stops at control flow.** SPEC 18.6 R1/R2/R4
  permanently route `if`/`match`/loops/closures, `let-else` targets
  and `P @` subpatterns to ordinary hunks;
  [matcher_general.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_engine/src/matcher_general.rs)
  `match_expr` returns `false` for those `syn::Expr` variants, and
  general patterns reject `...` (C1). Real edits inside bodies
  therefore cannot compress in either direction.

## Key decisions

- **D1 Diet is canonical, long form stays parseable.** WI-1 adds a
  terse canonical spelling; the parser keeps accepting today's long
  form so `.history/` archives and fixtures remain readable. Only
  emitters (diffrs, docs, benches) switch. Rejected: removing the
  long form (breaks the audit archive for no token gain) and a
  positional cryptic syntax like `# m f` (unreviewable; SPEC 2.2
  git-familiarity wins over ~4 B/hunk).
- **D2 Losslessness is judged on file trees + canonical patches,
  never raw native bytes.** The 0-context `similar` lowering cannot
  reproduce an arbitrary hand-written native patch byte-for-byte,
  and `index`/title stripping is deliberate. Binary/gitlink strips
  (warned, SPEC D.4) are declared lossy and out of scope.
- **D3 Inference stays precision-first.** Every new detector keeps
  pre-image totality (count must equal pair count, zero opaque hits)
  and the T375 byte-win guard, now measured in diet bytes. A detector
  that cannot prove totality stays ordinary — no guessing (SPEC 2.6).
- **D4 One anti-unification core, thin detectors.** WI-3 builds a
  single least-general-generalization pass over `syn` trees; WI-2
  and WI-5 reuse it instead of growing per-shape string surgery.
- **D5 Split before growing.** `parser.rs` (1989), `moves.rs`
  (1954), `semantic/mod.rs` (1846) are near the 2000-line gate;
  new code lands in new modules per the T246 precedent.

## Work plan

Order is dependency order: WI-1 → WI-4 → WI-2 → WI-3 → WI-5.
WI-1 first because every later emitter targets diet bytes and the
T375 guard must measure the final spelling. WI-4 second because it
locks the cycle contract before coverage expands. Estimated sizes
are relative (S ≤ 1 day, M ≈ 2–3 days, L ≈ 4–5 days of focused work).

### WI-1 (P0, M): Diet RSmPL — inferred metavars, fused sweep header, terse fn/item forms

**Problem.** F1: directive and header overhead, not payload, decides
whether semantic beats native; it also inflates every LLM-authored
patch (AGENTS.md recipes) and every diffrs emission.

**Decision — the diet, exactly.**
1. *Inferred metavariables.* An undeclared identifier is a
   metavariable iff it is a single uppercase ASCII letter, or
   matches `[A-Z][A-Za-z0-9_]*` and occurs in a metavariable
   position. Kind is inferred from position: fn-header slots →
   `visibility`/`qualifier_list`/`generic_params`/`where_clause`
   by slot; `let` pattern position → `pat`; type position →
   `type`; callee/method/receiver/argument position → `expr`;
   `- I` alone → `item`. Ambiguous position (same name in two
   kinds) is `RSMPL_PATTERN_CONFLICT` naming both positions.
   Explicit `# expr E` etc. remain legal, must agree with inference
   (disagreement is the same conflict), and are never required.
   Literal escape is unchanged: backtick `` `E` `` is literal.
   Sigiled mode (`# metavars=sigiled`, `$E`) is unchanged.
2. *Fused sweep header.* `# sweep` is exactly `# allow=many` +
   `# scope=file`. Diffrs emits only `# sweep`. Ranged multi-match
   keeps spelling `# allow=many` without `# scope=file`.
3. *Rule names.* `rule=` stays optional. Diffrs emits `r1`, `r2`, …
   in document order per patch (still unique, still the JSON key);
   human names remain legal for authored patches.
4. *Terse fn rename.* `- fn foo` / `+ fn bar` (bare, no `(...)`,
   braces or `...` lines) is a rename-only fn pattern: params,
   return type, generics and where clause are unconstrained
   (preserved), while any visibility/qualifier tokens written
   before `fn` (e.g. `- pub fn foo`) must match literally. This is
   shorthand for today's `fn foo(...) { ... }` form, usable only
   when neither side writes slot or body text; any slot text keeps
   the long form. Item renames already have this shape and are
   unchanged.
5. *Marker.* `# lang=rust` is no longer needed when the hunk has
   `rule=` or any directive; a directive-free rename uses the terse
   form plus `rule=`. Hunk detection (SPEC 7.1) becomes: `rule=`
   present, or ≥ 1 `#` directive line.

**Changes.** SPEC 7.1, 8.2, 9.3, 9.4, 14.1, 15.1 + Appendix A
(canonical examples rewritten in diet). Code:
`rsmpl_patch::model/parser` (detection, `# sweep` expansion);
`rsmpl_pattern::metavar` (inference table built during
`compile`, explicit-vs-inferred agreement), `pattern::classify`
(terse fn arm before `looks_like_fn`), `fn_pattern` (reuse with
all-ellipsis slots); every emitter format string in
`rsmpl_diff_infer::semantic/*` and `totality` needle shapes are
untouched (matching semantics do not change). Docs: AGENTS.md,
`skills/gitrs/SKILL.md`, README bench patches regenerated.

**Impact (acceptance, measured by `./example-benches/run.sh`).**
Diffrs `01` 111 B → ≤ 70 B; slot sweep loses ~85 B of declarations
+ rule text; applyrs `09` and `10` flip to ≥ 10% saved; no bench
regresses. T375 comparisons run on diet bytes (singles re-evaluated
in WI-2 may now pass; WI-1 alone must not flip any fixture's
fire/veto outcome except by byte-guard arithmetic, which is pinned).

**Tests.** New `bin/applyrs/tests/diet_e2e.rs`: inferred kinds per
position, ambiguity conflict, backtick literal still literal,
`# sweep` ≡ long form (same JSON `rules[].spans`), terse rename
matches/does-not-match the SPEC 14.1 modifier matrix, `rule=`-only
detection, long-form fixtures all still pass unmodified. Diffrs
unit tests updated to diet strings; `fixtures_roundtrip` unchanged
semantically. Gate: full workspace + benches + fmt/clippy.

**Non-goals / risks.** No matcher changes; no removal of long form.
Risk: inference surprises on uppercase literals (e.g. const `MAX`
as a call argument) — mitigated because consts in argument position
were already shadowed by a declared `MAX` today (SPEC 15.4), and
backticks remain the escape; the ambiguity conflict fails loud.

### WI-2 (P0, M): diffrs — qualified-path, turbofish and macro-invocation sweeps

**Problem.** F3/F2: the most idiomatic Rust call sites —
`crate::api(x)`, `mod::f::<T>(x)`, `recv.m::<T>(x)`, `old_mac!(x)` —
are vetoed in inference although the engine rewrites them today.

**Decision — three detectors, in chain order after the existing
method sweeps and before `try_call_sweep` (tightest first):**
1. `try_qualified_call_sweep`: pairs parse as `syn::Expr::Call`
   whose callee path has ≥ 2 segments (leading `::` and `qself`
   included); one shared old/new path delta in exactly the last
   segment, prefix segments token-equal, args per-site token-equal,
   uniform arity. Emission is the concrete-path form
   `- crate::old_api(E0, …)` / `+ crate::new_api(E0, …)` with
   `# sweep` — explicitly *not* `# symbol` (SPEC 19.3 already
   prefers concrete paths; no fallback directive tax).
2. `try_turbofish_sweep`: bare or qualified calls and method calls
   carrying `::<…>`; turbofish arguments are either token-equal
   across sites (emitted verbatim) or differ only by one type
   position, which is bound with an inferred `type` metavariable
   (WI-1). Bare-without-turbofish never matches a turbofish site
   (SPEC 18.6 R3 parity is preserved in totality keys).
3. `try_macro_rename_sweep`: pairs are single macro invocations
   `old!(…)` → `new!(…)` (also `old![…]`, `old!{…}`) with
   token-equal bodies; emits `# macro=token_tree` +
   `- old!(...)` / `+ new!(...)` under `# sweep`. Macro *interiors*
   are never inferred (SPEC 21.3 composition stands).

**Changes.** SPEC D.4 (three bullets + veto paragraph shrinks).
Code: generalize `call_parts` in
[semantic/mod.rs](/home/ai/git/tools/rsmpl-git/crates/rsmpl_diff_infer/src/semantic/mod.rs)
to return the full path (keep the single-segment wrapper for old
detectors); new `semantic/paths.rs` for detectors 1–2 and
`semantic/macros.rs` for 3 (D5 split); `totality.rs` gains
path+arity+turbofish-keyed counters reusing `opaque_hit_lines`
unchanged. No engine or pattern changes — probe first with
hand-authored patches proving each emission `--check`s and
real-applies (the T430 precedent).

**Impact (acceptance).** New diffrs benches 21–23 (qualified,
turbofish, macro rename; modelled on applyrs 16/17 repos) each
≥ 40% saved; benches 17/18 keep their ordinary macro-interior
residue but their invocation renames now compress (17: ≥ 35%
saved). History compressing set must not shrink; any new fire is
reported by the sweep, none may fail round-trip.

**Tests.** Unit fires + vetoes per detector (prefix-divergent,
turbofish-arity-divergent, body-divergent macro, distractor
`other::old_api` untouched — mirror applyrs bench 16). Diffrs
fixtures 61+ (applyrs cap rules do not bind diffrs naming; follow
the `NN-name` convention): one fire + one veto each. Full
`fixtures_roundtrip` real-apply byte-compare per fixture.

**Non-goals / risks.** No `# symbol` resolution work (WI-5 of the
engine is untouched; syntactic exact-path is the shipped semantic).
Risk: over-matching qualified names across modules — totality
counts matchable units by full path text in the pre-image, so a
second `crate::old_api` site vetoes exactly as bare callees do.

### WI-3 (P0, L): diffrs — anti-unification sweeps for let/type/signature evolution

**Problem.** F4: sites that differ in more than one slot — the
normal shape of a real refactor — have no detector. Verbatim needs
identical lines; `fn_rename_pair` needs identical suffixes; types
and `let` patterns have no sweep at all.

**Decision — one core, three thin detectors.**
Core `semantic/antiunify.rs`: given N ≥ 2 `(minus, plus)` `syn`
tree pairs of the same variant, compute the least general
generalization — a template pair where every position that is not
token-equal across all sites (within minus, within plus, or
between the sides) becomes a fresh inferred metavariable
(`E0…` expr, `T0…` type, `P0…` pat, by node kind), and every
token-equal position stays literal. Guards: same `syn` variant at
every generalized node; generalization depth ≤ 2 below the root
(deeper differences stay literal-or-veto, never a template that is
mostly holes); ≥ 1 literal anchor token must survive per side
(a template of pure metavars is rejected — it would match
anything); repeated positions that differ at one site must differ
identically at all sites or stay independent holes per SPEC 15.7.
Detectors, in chain order before verbatim (verbatim stays the
tighter winner for identical lines, T422 precedent):
1. `try_let_sweep`: root `Stmt::Local`; generalizes pattern, type
   ascription and initializer. Emits `- let P0: T0 = E0;` style
   templates with only the holes that actually vary.
2. `try_type_sweep`: root `syn::Type`; e.g. `Vec<T>` → `Box<[T]>`
   across sites with varying `T`. Emits `# type` holes via WI-1
   inference, ranged or `# sweep` by site spread (WI-4 rule).
3. `try_sig_evolution_sweep`: root fn header; name plus any of
   vis/qualifier/generic/param/return/where slots may carry the
   delta, bodies untouched. Emits the slot template with concrete
   text only where all sites agree, slot metavars elsewhere —
   strictly generalizing today's rename-only slot sweep, which
   becomes its zero-delta special case and is deleted.

**Changes.** SPEC D.4 (three bullets; delete the rename-only slot
bullet's suffix-equality clause), SPEC 15.7 note that inferred
holes follow the same structural-equality rule. Code: new
`antiunify.rs` (~600 lines, own module per D5); detectors in
`semantic/generalized.rs`; `totality.rs` gains a generic
template counter: parse the emitted minus template with
`rsmpl_pattern::compile` + engine match-count over the pre-image
must equal N (reuse `apply_patch_to_tree` on a scratch tree —
sound by construction, replaces three hand-rolled counters).
Existing call/method detectors are *not* rewritten onto the core
in this item (they carry peel/literal special cases); migration
is a follow-up only if the core proves out.

**Impact (acceptance).** New benches: diffrs 24 (let sweep with
varying args), 25 (type sweep), 26 (evolved signatures + call
sites stay separate hunks — signatures compress, calls follow
existing sweeps) each ≥ 30% saved. Probe corpus before merging:
the ten largest ordinary-only history commits from the sweep
report are tried; at least three must gain a semantic hunk with
byte-identical real-apply, or the guards are tightened until
precision holds (report which, in the PR description).

**Tests.** Anti-unify unit matrix: equal→literal, differing→hole,
kind-mismatch→veto, depth-3→veto, pure-hole→veto, repeated-hole
equality. Detector fires/vetoes mirroring applyrs benches 07/09/15
repos converted through diffrs. Every fixture through
`fixtures_roundtrip` (real-apply, since `--check` is blind to
over-match, T361).

**Non-goals / risks.** No statement-sequence or control-flow
generalization (WI-5). No cross-hunk grouping beyond today's
section driver. Main risk is template over-generality; the
anchor-token + totality-by-engine-match guards are the mitigation,
and both are acceptance gates, not aspirations.

### WI-4 (P1, M): Lossless canonical cycles — range-faithful emission and the S→N→S harness

**Problem.** F5/D2: "lossless" is currently only half-tested and
half-defined. Authored ranges, rule identity and directive intent
cannot survive a cycle, so agents cannot trust `gitrs diff` output
as a re-appliable, re-diffable artifact.

**Decision — canonical form and emission rules.**
Canonical RSmPL (the form diffrs emits and the cycle compares):
diet spelling (WI-1); directives in fixed order (`# sweep`, then
inferred-kind groups in declaration order of first use); hunks
sorted by (file, first matched old-line); one hunk per detector
class per file. Emission range rule: if all sites of a class lie
within one contiguous original range of ≤ 64 lines and ≥ 2 sites
share that range's file region, emit that real range with
`# allow=many` (no `# scope=file`); otherwise `# sweep`. Moves and
deletes keep today's real ranges/anchors. Rule ids are `r1…` in
canonical hunk order (WI-1), making the form fully deterministic:
same pre/post trees ⇒ byte-identical canonical patch.
Lossless corpus (in scope): all 38 applyrs fixtures, 60 diffrs
fixtures, 40 benches, plus history_e2e pins. Out of scope by D2:
binary/gitlink sections, non-UTF-8, CRLF semantic compression
(passthrough cycle must still be tree-lossless), macro interiors.

**Changes.** SPEC: new D.5 "Canonical form and lossless cycles"
(definitions above, corpus, exclusions); D.4 range-rule bullet.
Code: `rsmpl_diff_infer` emission takes site spans (detectors
already compute them via totality; thread them through instead of
discarding) and chooses range-vs-sweep; a `canonicalize` function
in `rsmpl_patch` (parse → re-emit canonical) backs the harness.
No new CLI flags: diffrs output is canonical by construction and
`canonicalize` is a library + test API only. New test
`bin/diffrs/tests/cycle_e2e.rs`: for each corpus case, (a) apply
authored/emitted S to pre-tree → post; (b) `git diff -U0` pre→post
→ diffrs → S′; assert tree(S′) == post and
`canonicalize(S′) == canonicalize(S)` for semantic cases, plain
tree equality for ordinary cases; (c) one more iteration asserts
S′′ == S′ byte-identical (fixed point). Failures are reported as
(per-case, first-diverging hunk) — no silent skips.

**Impact (acceptance).** Cycle harness green over the whole
corpus; every failure found while building it is either fixed in
this item (emission determinism, range choice) or the case is
moved to an explicit `#[ignore]`-style exclusion list in D.5 with
a one-line reason — the list, not the harness, is the review
surface. Ranged emission also cuts re-apply over-match risk on
evolved trees (spans in JSON now cite real ranges again).

**Tests.** Above harness plus unit pins: canonicalize is
idempotent on every fixture patch; directive-order and rule-id
determinism under shuffled input hunk order (where order is
semantically free) is *not* claimed — document order is input.

**Non-goals / risks.** No new CLI surface for end users; no
attempt to make native bytes round-trip (D2). Risk: ranged hunks
reintroduce range-drift failures diffrs avoided with `scope=file`;
the 64-line contiguity rule and totality-by-pre-image keep the
range exactly over proven sites, and the harness real-applies
every case.

### WI-5 (P1, L): Engine — control-flow, let-else and @-pattern matching (SPEC 18.6 R1/R2/R4)

**Problem.** F6: any edit whose target expression *is* or
*contains as its root* an `if`/`match`/loop/closure/block, any
`let-else`, and any `P @ sub` pattern forces an ordinary hunk that
restates the whole construct — the largest remaining authored-token
sink and the reason body-level diffrs classes (WI-3) stop at
statement roots.

**Decision — bounded, explicit expansion (no `...` in general
patterns; C1 stands).** Extend `matcher_general::match_expr` with
structural arms, each field matched by existing `match_expr` /
`match_type` / pattern matching, metavariables binding exactly as
elsewhere: `If` (cond, then-block as statement sequence, else
branch), `Match` (scrutinee + arms; arm pattern via the `let`
pattern matcher, guard, body), `Block` (statement sequence,
lengths must be equal — sequences are concrete, this is not
ellipsis), `Loop`/`While`/`ForLoop` (label must be token-equal,
body as `Block`), `Closure` (param patterns + types, return type,
body), `Let` expr (pattern + scrutinee), `Return` already exists.
`match_local` accepts `let-else`: the diverge block must be a
`Block` arm matched structurally (usually written concretely).
Pattern matcher accepts `P @ sub` by binding `P` to the whole
target pattern text and matching `sub` recursively. Everything
not listed (async blocks, try blocks, unsafe blocks, yield,
macro types R5) stays rejected. SPEC 18.6: R1/R2/R4 move from
"permanent" to implemented rows with exact semantics; the
reopen rule ("only with a dogfood case") is satisfied by WI-3's
probe corpus, whose ordinary residue is the dogfood — cite the
three largest in the SPEC change.

**Changes.** SPEC 13.4 (parse contexts unchanged), 18.6 table,
18.1 note, 15 pattern note. Code:
`matcher_general.rs` arms + a `match_block`/`match_pat` helper —
file is 1751 lines, so helpers land in a new
`matcher_general_ctrl.rs` submodule (D5); `pattern.rs`
`check_let_pat` admits the `@` form. Diffrs needs no change:
verbatim and WI-3 templates whose roots are now matchable stop
being vetoed by `verbatim_shape_ok` / engine-match totality —
update those shape gates to ask the engine (compile + count)
instead of a hard-coded variant list.

**Impact (acceptance).** New applyrs benches 21 (`if` cond
rewrite over 4 sites), 22 (`match` scrutinee rewrite), 23
(`let-else` initializer rewrite) each ≥ 35% saved vs native;
diffrs counterparts or fixture pins prove the raising direction.
History sweep: report compressing-set delta; zero round-trip
failures is the hard gate, growth is reported not asserted.

**Tests.** Per-arm matcher units (fire, field-divergent veto,
label-divergent veto, arm-count veto), `let-else` fire/veto,
`P @` binding + repeated-`P` equality, plus the SPEC 18.6
verbatim e2e files updated (`general_compile_verbatim_e2e.rs`
expectations move rows, not deleted). Full fixtures + benches.

**Non-goals / risks.** No `...` inside blocks, no statement
ellipsis, no macro expansion. Risk: structural `Block` equality
makes long bodies brittle — intended (SPEC 2.4 explicit syntax);
authors keep ordinary hunks for body edits and use these arms
for header/scrutinee/condition-shaped rewrites.

## Sequencing and gates

| Item | Depends on | Size | Gate before next item |
|---|---|---|---|
| WI-1 diet | — | M | benches green, 09/10 flip, no fixture outcome flips except pinned byte-guard cases |
| WI-4 cycles | WI-1 | M | `cycle_e2e` green over corpus or D.5 exclusion list reviewed |
| WI-2 paths/macros | WI-1 | M | new benches ≥ targets, history sweep 0 failures |
| WI-3 anti-unify | WI-1, WI-4 | L | probe-corpus criterion met, fixtures real-apply green |
| WI-5 control flow | WI-3 gates | L | 18.6 verbatim suites updated, sweep 0 failures |

Global validation per item: `cargo test --workspace`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`./example-benches/run.sh` (README tables regenerated only when
bytes change), `python3 tools/history_sweep.py` (0 failures;
compressing set reported). No item lands with a red gate or with a
new `.rs` file over, or an existing file pushed over, 2000 lines.

## Risks / open questions

Risks are per-item above. Open questions: none — scope, canonical
form, detector order, guards and acceptance thresholds are decided
here; WI-3's probe-corpus outcome tightens guards empirically but
does not reopen the design.

