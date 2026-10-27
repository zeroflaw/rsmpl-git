# gitrs

> Built for the Meta Global AI Developer Hackathon — an experiment in agentic, token-efficient refactoring.

Instead of agents manually deleting and adding large sections of code, `gitrs`
provides an understandable semantic patching language for human and machine:
name the file and range once, describe the change as a Rust-shaped pattern,
and let the engine lower it to a concrete Git diff.

Combine SmPL (Semantic Patch Language), Git and Rust language semantics into
an agent-centric tool for token-efficient code refactoring.

`gitrs apply` takes an **RSmPL-Git patch** (Git-like hunks plus Rust
semantic rewrite hunks) and applies it to a repository by first lowering
it to an ordinary Git patch, then running `git apply` (via the `applyrs`
binary). `gitrs diff` compresses pending git changes into a semantic
patch (via the `diffrs` binary). See `SPEC.md` for the full
patch format (v0.1 notes mark the implemented subset). Agent? Start with
`AGENTS.md` (one-page authoring reference, ~140 lines); `SPEC.md` is the
full contract; `skills/gitrs/SKILL.md` is the tool-driven patch-first
loop (read → author → check → apply).

## Install

```sh
cargo install --path bin/applyrs
cargo install --path bin/gitrs
cargo install --path bin/diffrs
# or: cargo build --release -p gitrs -p applyrs -p diffrs
```

`gitrs apply` invokes the sibling `applyrs` binary (and `gitrs diff`
the sibling `diffrs`), so keep both on `PATH`; a release build places
them together in `target/release/`.

Requires `git` on `PATH` for the check/apply step (the CLI shells out to
`git apply`); without it that step fails with ``failed to run `git apply`
(is git installed?)``. Matching, lowering, and `--output`
need no git.

## Usage

Run from the repo root (else add `--repo ./my-repo` to any command).

### Preview (nothing applied)

```sh
cat change.rsmpl | gitrs apply --check
gitrs apply --file change.rsmpl --check
gitrs apply --patch "$(cat change.rsmpl)" --check
gitrs apply --file change.rsmpl --repo /tmp/empty --check  # lint only; `no match found` = well-formed (SPEC D.2.4)
```

### Apply

```sh
gitrs apply --file change.rsmpl --output lowered.patch
```

Every run archives the raw patch to `<repo>/.history/{id}.rsmpl` (last 128 kept).
Range discovery is a targeted file read: cite `@@ -start,len` (`L = end-start+1`);
small file already held? `@@ -1,1 @@` + `# scope=file` searches the whole file (SPEC 17.3/D.3).

### Token-bounded JSON

```sh
gitrs apply --file change.rsmpl --format json --check
gitrs apply --file change.rsmpl --format json --diff-max-lines 0 --check
gitrs apply --file change.rsmpl --format json --diff-max-lines 40 --check --output full.patch
```

### Diff (native changes → semantic patch)

```sh
gitrs diff --repo ./my-repo              # pending unstaged changes as RSmPL (stdout)
gitrs diff --staged                      # index vs HEAD instead of worktree
gitrs diff HEAD~1                        # history: changes since a revision
gitrs diff --format json                 # {"version":1,"files":[…],"patch":"…"}
```

v1 compresses fn/item renames (one ident delta), N-site
heterogeneous fn/method renames (one ident delta → one slot-sweep
`V Q fn … G (…) W` hunk), and N-site call rewrites (same callee/arity
delta → one `# allow=many` hunk), N-site method-to-function
fan-outs (`recv.old(args)` → `new_fn(recv, args)` → one `# allow=many`
`Receiver` hunk), and N-site method-to-method renames
(`recv.old(args)` → `recv.new(args)` → one `# allow=many` `Receiver`
hunk), and N-site `&`-receiver strips (`&recv.m(lits)` → `recv` → one
`# allow=many` general-expr hunk), and N-site use-path prefix
renames (`use P::Old::T` → `use P::New::T` → one `# allow=many`
hunk) into semantic hunks — whole-file
first, then per balanced
hunk (T349); unrecognized runs stay 0-context ordinary hunks (no
untouched lines, still round-trip).
Output always parses and round-trips through `gitrs apply --check`
against the pre-change tree (measured: 10-site 2-arg sweep 548 B →
148 B; trivial renames stay ordinary — a semantic hunk is kept only
when byte-smaller than the ordinary lines it replaces, T375). Untracked files are included as
new-file sections; binary files are stripped with a `warning:` line and `.history/` audit
files are skipped with a `warning:`. Empty changes print `(no changes)`
on stderr with empty stdout (never pipe empties into `gitrs apply` —
it rejects empty input). Round-trip recipe:

```sh
gitrs diff > change.rsmpl && git stash -q && gitrs apply --file change.rsmpl --check
```

History dogfood: `bin/diffrs/tests/history_e2e.rs` converts a
curated selection of this repo's own commits (`checkout c` +
`diffrs c~1`, empty tree for root) and round-trips each through
`applyrs --check` plus a real-apply byte-compared against the child
tree (T361: `--check` is blind to sweep over-match) — failing
conversions become pins there.

`--format` picks text vs JSON; `--diff-max-lines N` (`0` omits) trims the
preview only — `--check`/apply/`--output` always use the full 0-context
lowering (still appliable via `git apply --unidiff-zero`). Exit `0` clean,
`2` patch error (`RSMPL_*`/`GIT_*` + suggestion). Full envelope contract:
SPEC D.2/D.2.3; history: SPEC D.1.

## Token savings

`example-benches/` (20 benches) compares the agent-authored `patch.rsmpl`
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
| 11-repeat-general-expr (nested `E + 0`, 1 hunk) | 109 B / 8 ln | 213 B / 11 ln | 49% |
| 12-amp-receiver (4 sites, 1 hunk) | 102 B / 7 ln | 255 B / 12 ln | 60% |
| 13-call-sweep-scaling (20 sites, 1 hunk) | 104 B / 7 ln | 649 B / 33 ln | 84% |
| 14-slot-sweep (4 sigs, 1 hunk) | 210 B / 12 ln | 405 B / 15 ln | 48% |
| 15-pat-sweep (4 ascribed lets, 1 hunk) | 149 B / 9 ln | 331 B / 12 ln | 55% |
| 16-symbol-sweep (5 qualified sites, 1 hunk) | 179 B / 10 ln | 425 B / 14 ln | 58% |
| 17-macro-sweep (5 macro sites, 1 hunk) | 128 B / 7 ln | 285 B / 14 ln | 55% |
| 18-attrs-sweep (4 attributed sigs, 1 hunk) | 210 B / 12 ln | 407 B / 15 ln | 48% |
| 19-impl-sweep (4 methods, 1 hunk) | 186 B / 12 ln | 391 B / 15 ln | 52% |
| 20-trait-sweep (3 methods, 1 hunk) | 154 B / 9 ln | 303 B / 12 ln | 49% |

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
- Ordinary hunks are language-agnostic (`Cargo.toml`/`.toml` included); semantic hunks are Rust-only (SPEC 11).

## Layout

```
bin/gitrs/          dispatcher: `gitrs apply` -> applyrs, `gitrs diff` -> diffrs
bin/applyrs/        apply CLI: patch -> lowered Git diff -> `git apply`
bin/diffrs/         diff CLI: git changes -> compressed semantic patch
crates/rsmpl_patch/   patch document model + parser
crates/rsmpl_pattern/ Rust pattern language (metavars, ellipsis, fn slots)
crates/rsmpl_engine/  semantic matching + plan/move engine
crates/rsmpl_git/     unified-diff lowering + `git apply` wrapper
crates/rsmpl_diff*/   inverse mapping: git source -> infer -> emit RSmPL (see T340)
test-fixtures/        binary-driven e2e fixtures (see bin tests)
```

No single `.rs` file exceeds 2000 lines (enforced by test).
