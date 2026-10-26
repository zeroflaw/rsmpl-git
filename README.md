# rsmpl-git

Combine SmPL (Semantic Patch Language), Git and Rust language semantics into
an agent-centric tool for token-efficient code refactoring.

`rsmpl-git` takes an **RSmPL-Git patch** (Git-like hunks plus Rust semantic
rewrite hunks) and applies it to a repository by first lowering it to an
ordinary Git patch, then running `git apply`. See `SPEC.md` for the full
patch format (v0.1 notes mark the implemented subset). Agent? Start with
`AGENTS.md` (one-page authoring reference, ~140 lines); `SPEC.md` is the
full contract.

## Install

```sh
cargo install --path bin/rsmpl-git
# or: cargo build --release -p rsmpl-git
```

## Usage (agent-oriented)

```sh
# Inline patch (no restatement of unchanged bodies needed):
rsmpl-git --patch "$(cat change.rsmpl)" --repo ./my-repo

# Piped patch, no temp file:
cat change.rsmpl | rsmpl-git --patch - --repo ./my-repo

# From a file, preview the lowered Git diff without applying:
rsmpl-git --file change.rsmpl --repo ./my-repo --check

  # Machine-readable result (rules, files stats, diff + line counts, check):
  rsmpl-git --file change.rsmpl --repo ./my-repo --format json --check

  # Token-bounded verification: stats + check, no diff tokens ...
  rsmpl-git --file change.rsmpl --repo ./my-repo --format json --diff-max-lines 0 --check
  # ... or skim the first 40 diff lines (stats stay exact, --output stays full):
  rsmpl-git --file change.rsmpl --repo ./my-repo --format json --diff-max-lines 40 --check --output full.patch
  # Shrink the lowering itself (still appliable): 0 context lines, stats exact:
  rsmpl-git --file change.rsmpl --repo ./my-repo --check --context 0

# Discover valid @@ ranges without reading whole files:
rsmpl-git --repo ./my-repo --inspect src/lib.rs
rsmpl-git --repo ./my-repo --inspect src/lib.rs --format json

# Also list call/method/macro sites for tight call-rewrite ranges:
rsmpl-git --repo ./my-repo --inspect src/lib.rs --calls

# Scan a whole subtree in one call (grep-style `file: kind name s-e`):
rsmpl-git --repo ./my-repo --inspect src/ --calls
rsmpl-git --repo ./my-repo --inspect src --format json
# Bound large scans so they stay token-efficient (`truncated` in JSON):
rsmpl-git --repo ./my-repo --inspect src --calls --max-items 20

# Apply and save the concrete diff for review:
rsmpl-git --file change.rsmpl --repo ./my-repo --output lowered.patch
```

Exit codes: `0` applied/clean, `2` patch error (structured
`RSMPL_*`/`GIT_*` diagnostic on stderr). Text `--check` previews the lowered
diff to stdout (plain apply stays quiet); `--format json` prints
`{rules, files, diff, diff_truncated, diff_lines, diff_shown, check}` to stdout
(`rules` = per-rule match counts; `files` = per-file `added`/`removed`;
`diff_truncated` = reported diff hides a non-empty lowering;
`diff_lines`/`diff_shown` = full vs reported line counts so the next
`--diff-max-lines N` is computable); inspect JSON adds `total`/`omitted`
entry counts next to `truncated` for the same reason;
`--diff-max-lines N` (`0` omits) bounds the reported diff only
(`--check`/apply/`--output` always use the full diff — but `--output`
writes nothing when the lowering is empty, and it is written before
`--check` runs so a failing check still leaves the full diff);
`--context N` (default 3, `0` allowed) instead shrinks the lowering itself
(`--check`/apply handle any N via `git apply --unidiff-zero`; piping a saved
`--context 0` patch elsewhere needs that flag) (stats unaffected); JSON
`--check` is stdout-only (no stderr `check:` line — the `"check":"ok"`
payload is the signal), while `warning:` lines stay stderr-only in both
modes, so JSON consumers must also read stderr; failures print
`{error: {code, rule, file, range, reason, suggestion}}` to stdout
(`RSMPL_IO_CONFLICT` covers unreadable patch/repo/output files).
`--inspect` needs no patch. Stream/empty/output contract: SPEC D.2.3.

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
  `# scope=file` searches the whole file with no inspect roundtrip.
- `fn foo(...) { ... }` matches only plain functions (modifiers are
  explicit); attributes/doc comments are preserved by default. Writing
  slots explicitly evolves the signature (`- fn f(...)` /
  `+ pub async fn f<T>(...) -> Vec<T>` keeps params/body, changes the
  rest); `...`/same-metavariable slots are preserved, bodies never
  rewritten. Other items rename with a bare pair (`- struct Old` /
  `+ struct New`, also `enum`/`trait`/`type`/`const`/`static`/`mod`/
  `union`/`impl` self-ty): one ident edit, vis/generics/bodies preserved.
- Call rewrites (`- foo(E)` / `+ bar(E)`) need `# allow=many` for multiple
  sites and `# optional` to tolerate zero matches.
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
