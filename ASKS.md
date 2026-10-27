# ASKS — improvement notes from dogfood use

Enhancements discovered while using `gitrs`/`diffrs` on this repo.
Promote to TASKS.md when they earn it; keep this list short.

## Open

- Method→method rename sweep in diffrs (`a.foo(x)` → `a.bar(x)`
  fan-out, mirroring T345's method→function arm). Filed as T347.
- Same-file multi-section rebase question: close-range hunks split
  across two `diff --git` sections for one file failed while one
  merged section applied (T345 REVIEW). Filed as T348 to determine
  whether cross-section same-file rebasing is broken or the ranges
  were mis-authored.
- Patch-authoring sharp edge (not a defect): a leading-space `@@`
  header is a valid unified-diff context line, so the patch lowers
  to an empty no-op (`check: true`, `(no changes)`) instead of
  failing fast. A `warning:` on no-op lowerings (or an explicit
  empty-hunk diagnostic) would save debugging time. Not filed —
  needs a CLI-minimality weigh-up first (T338 found no clutter;
  any new flag/diagnostic must clear the same bar).
- Large-patch sharp edge (not a defect): a ~1 MB `diffrs` conversion
  exceeds the shell's inline-arg limit (`Argument list too long` via
  `--patch "$(…)"`) — large patches must go through `--file`
  (already the documented path). Not filed — no tool change.
- Ordinary-hunk authoring sharp edge (T355 dogfood): a wrong
  `-old,len`/`+new,len` count fails as `GIT_HUNK_CONFLICT`
  (content mismatch) rather than flagging the header/body length
  inconsistency. A count-vs-body validation with a precise error
  would save authoring rounds. Not filed — needs a weigh-up
  against parser minimality.
- Engine coverage gap (T360 probe, no dogfood): a reference-of-
  metavar call arg (`- f(E0, &E1)` / `+ f(E0, E1)`) never matches —
  `classify_args` treats `&E1` as verbatim `Concrete`, and whole-
  pattern calls never reach the general matcher's structural
  `Reference` arm. Verbatim deltas (`- f(E0, &t[k])` /
  `+ f(E0, t[k])`) work. Not filed — zero history hits; promote
  only if a future commit shows N ≥ 2 shared-delta pairs.
- Unbalanced insert+rename sweep (T362 dogfood, 38f28ea): 6x
  `.filter_map(|d| match d {…})` → `.rev().find_map(|d| match
  d {…})` — a 1-minus/2-plus pair per site (insert + rename).
  Correctly passthrough today (detectors need balanced counts);
  compressing it needs unbalanced-pair inference + an engine
  shape for chained-callee plus sides. Not filed — single
  commit; promote with a second independent exhibit.
- De-indent move compression (T362 dogfood, 3baecc1): 821-line
  module-flatten de-indent (balanced text, indentation-only
  delta). Correctly ordinary today; compressing it needs move
  inference in diffrs. Not filed — single commit; promote with
  a second exhibit.
- Empty-tree UX (T362 weigh-up, deferred): `diffrs <empty-tree>`
  on a repo lacking the object fails with git’s `unknown
  revision` (clean exit 2, not silent). A targeted hint for the
  well-known hash would serve the T346 root recipe, but it
  special-cases a magic value in a pure source path — needs the
  T338 CLI-minimality weigh-up before filing.

- Verbatim identical-pair sweep (T364 probe, no task): N≥2
  textually identical `-/+` line pairs both parsing as `syn::Expr`
  (e.g. 712ad9d’s 2x `&cand.body_text` →
  `std::slice::from_ref(&cand.body_text)`) compress to one verbatim
  `# allow=many` general-expr hunk -- the engine probe-passes
  (`matches == 2`). Rejected for diffrs: the minus text also occurs
  in an unchanged match guard, so the sweep over-matches (3 rewrites
  vs 2 sites; history byte-compare fails). Sub-expression matching
  is inherent (the terminator never reaches the pattern), so no
  textual tightening fixes it -- needs content-aware occurrence
  counting (layering break) or engine scoping. Not filed.
- Vis-evolution sweep (T364 probe, no task): N≥2 `fn name` →
  `pub(crate) fn name` pairs with differing names (27f2aba, N=2).
  Probe-passes per name, but fn-name slots don’t exist (the slot
  sweep hardcodes one name) -- per-name hunks have no token win.
  Not filed.
