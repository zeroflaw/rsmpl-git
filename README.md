# rsmpl-git

Combine SmPL (Semantic Patch Language), Git and Rust language semantics into
an agent-centric tool for token-efficient code refactoring.

`rsmpl-git` takes an **RSmPL-Git patch** (Git-like hunks plus Rust semantic
rewrite hunks) and applies it to a repository by first lowering it to an
ordinary Git patch, then running `git apply`. See `SPEC.md` for the full
patch format (v0.1 notes mark the implemented subset).

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
rsmpl-git --file change.rsmpl --repo ./my-repo --emit-diff --check

# Machine-readable result (rules, files_changed, diff, check):
rsmpl-git --file change.rsmpl --repo ./my-repo --format json --emit-diff --check

# Discover valid @@ ranges without reading whole files:
rsmpl-git --repo ./my-repo --inspect src/lib.rs
rsmpl-git --repo ./my-repo --inspect src/lib.rs --format json

# Apply and save the concrete diff for review:
rsmpl-git --file change.rsmpl --repo ./my-repo --output lowered.patch --verbose
```

Exit codes: `0` applied/clean, `2` patch error (structured
`RSMPL_*`/`GIT_*` diagnostic on stderr). `--format json` prints
`{rules, files_changed, diff, check}` to stdout; `--inspect` needs no patch.

## Minimal semantic hunk

```diff
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,5 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

Key rules for authors:

- Every semantic hunk is scoped by its file section + old-side range, always
  interpreted in **original-source coordinates** (never rebased by earlier
  hunks in the same patch).
- `fn foo(...) { ... }` matches only plain functions (modifiers are
  explicit); attributes/doc comments are preserved by default.
- Call rewrites (`- foo(E)` / `+ bar(E)`) need `# allow=many` for multiple
  sites and `# optional` to tolerate zero matches.
- Beyond calls: `let` statements (`- let x = E;` / `+ let y = E;`), general
  expressions (`- E + 1` / `+ E.wrapping_add(1)`), and types
  (`- Vec<T>` / `+ Box<[T]>` with `# type T`) rewrite structurally. A bare
  `P` never matches an annotated `x: T` — write `P: T`.
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
