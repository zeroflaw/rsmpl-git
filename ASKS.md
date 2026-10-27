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
- diffrs history-per-commit recipe (`checkout c` + `diffrs c~1` as
  regression-case generator, converting this repo's own commits).
  Already backlog as T346.
