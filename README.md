# rsmpl-git

Combine SmPL (Semantic Patch Language), Git and Rust language semantics into
an agent-centric tool for token-efficient code refactoring.

`rsmpl-git` takes an **RSmPL-Git patch** (Git-like hunks plus Rust semantic
rewrite hunks) and applies it to a repository by first lowering it to an
ordinary Git patch, then running `git apply`. See `SPEC.md` for the full
patch format (v0.1 notes mark the implemented subset). Agent? Start with
`AGENTS.md` (one-page authoring reference, ~140 lines); `SPEC.md` is the
full contract; `skills/rsmpl-git/SKILL.md` is the tool-driven patch-first
loop (read → author → check → apply).

## Install

```sh
cargo install --path bin/rsmpl-git
# or: cargo build --release -p rsmpl-git
```

Requires `git` on `PATH` for the check/apply step (the CLI shells out to
`git apply`); without it that step fails with ``failed to run `git apply`
(is git installed?)``. Matching, lowering, and `--output`
need no git.

## Usage (agent-oriented)

```sh
# Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
# Piped patch, no temp file (no input flags means stdin):
cat change.rsmpl | rsmpl-git --check

# From a file, preview the lowered Git diff without applying:
rsmpl-git --file change.rsmpl --check

# Inline, no temp file (archived to <repo>/.history/{id}.rsmpl, last 128 kept):
rsmpl-git --patch "$(cat change.rsmpl)" --check

  # Machine-readable result (rules, files stats, diff + line counts, check):
  rsmpl-git --file change.rsmpl --format json --check

  # Token-bounded verification: stats + check, no diff tokens ...
  rsmpl-git --file change.rsmpl --format json --diff-max-lines 0 --check
  # ... or skim the first 40 diff lines (stats stay exact, --output stays full):
  rsmpl-git --file change.rsmpl --format json --diff-max-lines 40 --check --output full.patch
  # Rule of thumb: `--format` picks text vs JSON, `--diff-max-lines`
  # trims the preview only (the lowering itself is always 0-context,
  # still appliable), `--output` always saves the full diff.
  # Defaults stay: text report; `--repo` defaults to `.`.
  # Lint authoring without a checkout (parse/directives/captures):
  # mkdir -p /tmp/empty && rsmpl-git --file change.rsmpl --repo /tmp/empty --check
  # (`no match found` = well-formed; triage table: SPEC D.2.4).

# Range discovery is a targeted file read: note the 1-based spans to
# rewrite and cite `@@ -start,len` (`L = end-start+1`). Small file already
# held? `@@ -1,1 @@` + `# scope=file` searches the whole file (SPEC 17.3/D.3).

# Apply and save the concrete diff for review:
rsmpl-git --file change.rsmpl --output lowered.patch
```

Exit codes: `0` applied/clean, `2` patch error (structured
`RSMPL_*`/`GIT_*` diagnostic on stderr). Text `--check` previews the lowered
diff to stdout (plain apply stays quiet); `--format json` prints
`{version, rules, files, diff, diff_truncated, diff_lines, diff_shown, check, history}` to stdout
(`version` = report schema, currently `1`; `rules` = per-rule match counts + per-match `spans`; `files` = per-file `added`/`removed`;
`diff_truncated` = reported diff hides a non-empty lowering;
`diff_lines`/`diff_shown` = full vs reported line counts so the next
`--diff-max-lines N` is computable);
`--diff-max-lines N` (`0` omits) bounds the reported diff only
(`--check`/apply/`--output` always use the full diff — but `--output`
writes nothing when the lowering is empty, and it is written before
`--check` runs so a failing check still leaves the full diff).
The lowering itself is always 0-context (minimal tokens, still appliable:
`--check`/apply use `git apply --unidiff-zero`; piping a saved patch
elsewhere needs that flag) (stats unaffected); JSON
mode is stdout-only for both success and failure (no stderr `check:` or
`error:` lines — the `"check":true` payload or the `{version, error: {...}}`
envelope is the signal), while `warning:` lines stay stderr-only in both
modes, so JSON consumers must also read stderr; failures print
`{version, error: {code, rule, file, range, reason, suggestion}}` to stdout only
(`RSMPL_IO_CONFLICT` covers unreadable patch/repo/output files and
flag-usage errors — bad flags envelop in JSON mode, stay clap text
otherwise).
Every run archives the raw patch to `<repo>/.history/{id}.rsmpl` (last 128
kept; path on stderr `history:` + JSON `history`).
Stream/empty/output contract: SPEC D.2.3.

## Token savings

`example-benches/` (10 benches) compares the agent-authored `patch.rsmpl`
against the lowered 0-context `native.patch` for the same end state —
same result, RSmPL input vs full Git spelling. Re-measure any time with
`./example-benches/run.sh`. (A hand-written native patch with conventional
context would be larger, so these are lower bounds.)

| bench | rsmpl | native | saved |
|---|---|---|---|
| 01-rename-calls-many (12 sites, 1 hunk) | 92 B / 6 ln | 409 B / 21 ln | 78% |
| 02-move-fn-cross-file (44-line fn, no restate) | 151 B / 10 ln | 1576 B / 97 ln | 90% |
| 03-scope-file-sweep (9 sites, no discovery) | 110 B / 7 ln | 445 B / 20 ln | 75% |
| 04-method-to-function (4 sites) | 161 B / 7 ln | 347 B / 15 ln | 54% |
| 05-use-prefix (5 imports, 1 hunk) | 98 B / 5 ln | 347 B / 14 ln | 72% |
| 06-repeat-fold (nested `Box::new`, 1 hunk) | 94 B / 7 ln | 209 B / 9 ln | 55% |
| 07-let-and-type (7 sites, 3 hunks) | 226 B / 17 ln | 593 B / 21 ln | 62% |
| 08-rename-fn-big-body (54-line body kept) | 122 B / 7 ln | 149 B / 6 ln | 18% |
| 09-sig-evolution (2 signatures, bodies kept) | 253 B / 14 ln | 251 B / 9 ln | -1% |
| 10-item-rename (3 bare-pair renames) | 225 B / 15 ln | 210 B / 12 ln | -7% |

Fan-out edits save 50–90%. Single-ident edits are size-parity — the payoff
there is semantic safety (slot-exact matching, attrs preserved,
machine-checked match counts) at ~zero token premium.

## Minimal semantic hunk

```diff
diff --git src/lib.rs
@@ -1,5 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
   }
```

Minimal header: `diff --git <path>` alone suffices (no
`---`/`+++`/`index`); `rule=` may be omitted; elide the new side
(`@@ -old,len @@`, new side ignored, SPEC 10.4; copy-old stays
compatible).
The `# lang=rust` line is the RSmPL marker here (a hunk with no `#`
lines is an ordinary Git hunk); hunks that already carry other
directives omit it (SPEC 7.1).

Key rules for authors:

- Every semantic hunk is scoped by its file section + old-side range, always
  interpreted in **original-source coordinates** (never rebased by earlier
  hunks in the same patch). Small file already held? `@@ -1,1 @@` +
  `# scope=file` searches the whole file with no discovery roundtrip.
- `fn foo(...) { ... }` matches only plain functions (modifiers are
  explicit); attributes/doc comments are preserved by default. Writing
  slots explicitly evolves the signature (`- fn f(...)` /
  `+ pub async fn f<T>(...) -> Vec<T>` keeps params/body, changes the
  rest); `...`/same-metavariable slots are preserved, bodies never
  rewritten. Other items rename with a bare pair (`- struct Old` /
  `+ struct New`, also `enum`/`trait`/`type`/`const`/`static`/`mod`/
  `union`/`impl`, where `impl Old` covers the self-ty and the trait-side
  path): one ident edit, vis/generics/bodies preserved.
 - Call rewrites (`- foo(E)` / `+ bar(E)`) need `# allow=many` for multiple
   sites and `# optional` to tolerate zero matches; `# repeat` folds
   nested same-rule matches into one hunk, bottom-up (call + general-expr
   shapes, SPEC 17.8).
- Beyond calls: `let` statements (`- let x = E;` / `+ let y = E;`), general
  expressions (`- E + 1` / `+ E.wrapping_add(1)`), and types
  (`- Vec<T>` / `+ Box<[T]>` with `# type T`) rewrite structurally. A bare
  `P` never matches an annotated `x: T` — write `P: T`. Full supported-vs-rejected
  table with exact errors: SPEC 18.6.
- Moves use `# item I` + `# bind=Name` (cut) and `# use=Name` (paste);
  paste hunks may precede cut hunks.
- Ordinary Git hunks and semantic hunks can mix but must not overlap.

## Layout

```
bin/rsmpl-git/        CLI: patch -> lowered Git diff -> `git apply`
crates/rsmpl_patch/   patch document model + parser
crates/rsmpl_pattern/ Rust pattern language (metavars, ellipsis, fn slots)
crates/rsmpl_engine/  semantic matching + plan/move engine
crates/rsmpl_git/     unified-diff lowering + `git apply` wrapper
test-fixtures/        binary-driven e2e fixtures (see bin tests)
```

No single `.rs` file exceeds 2000 lines (enforced by test).
