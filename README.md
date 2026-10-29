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
hunk), and N-site verbatim identical-line sweeps (identical
single-line pairs → one `# allow=many` hunk with literal lines,
 subterm-totality checked against the pre-image file, T403)
 into semantic hunks — whole-file
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

`example-benches/applyrs/` (20 benches) compares the agent-authored `patch.rsmpl`
against the lowered 0-context `native.patch` for the same end state —
same result, RSmPL input vs full Git spelling. Re-measure any time with
`./example-benches/run.sh`. (A hand-written native patch with conventional
context would be larger, so these are lower bounds.)

| bench | rsmpl | native | saved |
|---|---|---|---|
| 01-rename-calls-many (12 sites, 1 hunk) | 83 B / 5 ln | 409 B / 21 ln | 80% |
| 02-move-fn-cross-file (44-line fn, no restate) | 142 B / 9 ln | 1576 B / 97 ln | 91% |
| 03-scope-file-sweep (9 sites, no discovery) | 83 B / 5 ln | 445 B / 20 ln | 81% |
| 04-method-to-function (4 sites) | 134 B / 5 ln | 347 B / 15 ln | 61% |
| 05-use-prefix (5 imports, 1 hunk) | 98 B / 5 ln | 347 B / 14 ln | 72% |
| 06-repeat-fold (nested `Box::new`, 1 hunk) | 85 B / 6 ln | 209 B / 9 ln | 59% |
| 07-let-and-type (7 sites, 3 hunks) | 208 B / 15 ln | 593 B / 21 ln | 65% |
| 08-rename-fn-big-body (54-line body kept) | 110 B / 6 ln | 149 B / 6 ln | 26% |
| 09-sig-evolution (2 signatures, bodies kept) | 202 B / 12 ln | 251 B / 9 ln | 20% |
| 10-item-rename (3 bare-pair renames) | 189 B / 12 ln | 210 B / 12 ln | 10% |
| 11-repeat-general-expr (nested `E + 0`, 1 hunk) | 82 B / 6 ln | 213 B / 11 ln | 62% |
| 12-amp-receiver (4 sites, 1 hunk) | 75 B / 5 ln | 255 B / 12 ln | 71% |
| 13-call-sweep-scaling (20 sites, 1 hunk) | 77 B / 5 ln | 649 B / 33 ln | 88% |
| 14-slot-sweep (4 sigs, 1 hunk) | 122 B / 7 ln | 405 B / 15 ln | 70% |
| 15-pat-sweep (4 ascribed lets, 1 hunk) | 105 B / 5 ln | 331 B / 12 ln | 68% |
| 16-symbol-sweep (5 qualified sites, 1 hunk) | 152 B / 8 ln | 425 B / 14 ln | 64% |
| 17-macro-sweep (5 macro sites, 1 hunk) | 110 B / 6 ln | 285 B / 14 ln | 61% |
| 18-attrs-sweep (4 attributed sigs, 1 hunk) | 122 B / 7 ln | 407 B / 15 ln | 70% |
| 19-impl-sweep (4 methods, 1 hunk) | 156 B / 10 ln | 391 B / 15 ln | 60% |
| 20-trait-sweep (3 methods, 1 hunk) | 124 B / 7 ln | 303 B / 12 ln | 59% |

`example-benches/diffrs/` (20 benches) compares `git diff -U0` (`native.patch`,
`index` lines stripped) against the `diffrs` compression (`sem.patch`):

| bench | native | semantic | saved |
|---|---|---|---|
| 01-call-sweep (12 call sites, 1 hunk) | 416 B / 28 ln | 75 B / 5 ln | 82% |
| 02-ordinary-edit (1 line, stays ordinary) | 89 B / 6 ln | 72 B / 6 ln | 19% |
| 03-move (same-file reorder, cut+paste) | 1857 B / 89 ln | 106 B / 9 ln | 94% |
| 04-peel (6 call sites, 2 peel hunks) | 1006 B / 22 ln | 425 B / 18 ln | 58% |
| 05-stranded (3 sites, stranded join + insertion) | 349 B / 15 ln | 196 B / 11 ln | 44% |
| 06-literal-cross-hunk (2 sites, section driver) | 679 B / 19 ln | 464 B / 17 ln | 32% |
| 07-ref-strip (2 sites, ref-strip sweep) | 212 B / 8 ln | 90 B / 5 ln | 58% |
| 08-slot-sweep (2 fns, slot sweep) | 540 B / 9 ln | 146 B / 7 ln | 73% |
| 09-arg-add (3 sites, arg-add sweep + hetero ordinary) | 489 B / 18 ln | 295 B / 13 ln | 40% |
| 10-same-literal (3 sites, same-literal sweep) | 386 B / 10 ln | 148 B / 5 ln | 62% |
| 11-let-sweep (2 let sites, verbatim sweep) | 319 B / 9 ln | 185 B / 7 ln | 42% |
| 12-verbatim (2 sites, verbatim sweep) | 199 B / 9 ln | 123 B / 7 ln | 38% |
| 13-verbatim-return (2 sites, Return-arm verbatim sweep) | 266 B / 9 ln | 146 B / 7 ln | 45% |
| 14-verbatim-struct-field (2 sites, field-label verbatim sweep) | 281 B / 9 ln | 127 B / 7 ln | 55% |
| 15-verbatim-unary (2 sites, Unary-arm verbatim sweep) | 219 B / 9 ln | 123 B / 7 ln | 44% |
| 16-call-literal (2 sites, literal sweep + hetero ordinary) | 548 B / 16 ln | 361 B / 13 ln | 34% |
| 17-macro-veto (6 sites, macro gates stay ordinary) | 1208 B / 24 ln | 1040 B / 24 ln | 14% |
| 18-macro-invocation-veto (2 sites + pure addition, stays ordinary) | 378 B / 14 ln | 308 B / 14 ln | 19% |
| 19-ref-strip-scale (5 sites, chained-receiver ref-strip sweep) | 436 B / 14 ln | 86 B / 5 ln | 80% |
| 20-method-to-function (3 sites, method-to-function sweep) | 237 B / 10 ln | 109 B / 5 ln | 54% |

Diet spelling (`# sweep`, inferred metavars, terse renames) keeps fan-out
edits at 50–90% and single-ident edits byte-positive — plus semantic
safety (slot-exact matching, attrs preserved, machine-checked counts).

## Minimal semantic hunk

```diff
diff --git src/lib.rs
@@ -1,5 @@ rule=rename_foo
- fn foo
+ fn bar
```

Minimal header: `diff --git <path>` alone suffices (no
`---`/`+++`/`index`); `rule=` may be omitted; elide the new side
(`@@ -old,len @@`, new side ignored, SPEC 10.4; copy-old stays
compatible).
Hunk detection is `rule=` or any `#` line (SPEC 7.1); `# lang=rust`
is accepted but never required. Directive-free renames use the terse
form plus `rule=` (above).

Key rules for authors:

- Every semantic hunk is scoped by its file section + old-side range, always
  interpreted in **original-source coordinates** (never rebased by earlier
  hunks in the same patch). Small file already held? `@@ -1,1 @@` +
  `# sweep` (`# allow=many` + `# scope=file`) searches the whole file with no discovery roundtrip.
- `fn foo(...) { ... }` matches only plain functions (modifiers are
  explicit); attributes/doc comments are preserved by default. Writing
  slots explicitly evolves the signature (`- fn f(...)` /
  `+ pub async fn f<T>(...) -> Vec<T>` keeps params/body, changes the
  rest); `...`/same-metavariable slots are preserved, bodies never
  rewritten. Other items rename with a bare pair (`- struct Old` /
  `+ struct New`, also `enum`/`trait`/`type`/`const`/`static`/`mod`/
  `union`/`impl`, where `impl Old` covers the self-ty and the trait-side
  path): one ident edit, vis/generics/bodies preserved.
 - Call rewrites (`- foo(E)` / `+ bar(E)`, `E` inferred) need `# sweep` for multiple
   sites and `# optional` to tolerate zero matches; `# repeat` folds
   nested same-rule matches into one hunk, bottom-up (call + general-expr
   shapes, SPEC 17.8).
- Beyond calls: `let` statements (`- let x = E;` / `+ let y = E;`), general
  expressions (`- E + 1` / `+ E.wrapping_add(1)`), and types
  (`- Vec<T>` / `+ Box<[T]>`, `T` inferred) rewrite structurally. A bare
  `P` never matches an annotated `x: T` — write `P: T`. Full supported-vs-rejected
  table with exact errors: SPEC 18.6.
- Moves use `# bind=Name` (cut, `- I` infers `item`) and `# use=Name` (paste);
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
test-fixtures/        binary-driven e2e fixtures: applyrs/ + diffrs/ (see bin tests)
```

No single `.rs` file exceeds 2000 lines (enforced by test).
