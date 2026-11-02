# gitrs demos

Hands-on traces of `gitrs apply` and `gitrs diff`. Each demo runs in a
throwaway work copy, prints every command before running it, and ends with
a byte-size comparison against the equivalent native Git patch. These are
the same traces captured for the promo video.

## Prerequisites

- **git** — the demos shell out to `git init`, `git apply`, and `git diff`.
- **Rust** (cargo + rustc) — install via [rustup](https://rustup.rs):
  `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.

No other setup: each `run.sh` builds the binaries it needs
(`cargo build --release -p gitrs -p applyrs -p diffrs`) on first run.

## Run

From the repo root:

```sh
./demo/01-apply-rename-calls/run.sh
./demo/02-diff-call-sweep/run.sh
./demo/03-apply-move-fn/run.sh
```

Pass `--keep` to leave the work copy behind for inspection (its path is
printed); otherwise it is deleted on exit. Prefer to drive it yourself?
Each demo dir has a `MANUAL.md` with the same steps as copy-pasteable
commands plus the expected output.

## The demos

| demo | what it shows | sizes |
|---|---|---|
| 01-apply-rename-calls | one 5-line patch renames `foo`→`bar` at 12 call sites: `--check` preview, apply, then `grep \| wc -l` proves all 12 landed | 83 B vs 409 B (80% smaller) |
| 02-diff-call-sweep | `gitrs diff` compresses 12 changed call sites into one 5-line semantic patch; compared against `git diff -U0` of the same change | 75 B vs 416 B (82% smaller) |
| 03-apply-move-fn | a 9-line cut+paste patch moves a 44-line fn from `src/old.rs` to `src/new.rs`; the body is named once, never restated | 142 B vs 1,576 B (91% smaller) |

Sizes are from the current build and may shift as the engine evolves;
each script re-measures them live. The native baseline is the 0-context
Git diff — the smallest patch producing the same end state — so the
savings are lower bounds (a hand-written patch with context would be
larger). See `example-benches/` for the full measurement suite these
demos are drawn from.
