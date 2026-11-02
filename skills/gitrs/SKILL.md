---
name: gitrs
description: Patch-first Rust refactoring with gitrs semantic hunks (read, author, check, apply).
---

# gitrs skill

Use `gitrs` for every Rust refactoring or modification — never hand-edit
what a semantic hunk can express. Text edits that need no semantics still go
through `gitrs` as ordinary hunks, so one tool owns every change.

## When to use / when not to

- Use: rename (fn/item/field/variant), call rewrites, moves, signature evolution, `let`/expr/type rewrites — anything with `-`/`+` pattern lines.
- Viewing only: use a targeted file read — the CLI lists nothing.
- Small file already held: skip discovery entirely — `@@ -1,1 @@` plus `# sweep` searches the whole file (SPEC 17.3).

## Minimal patch

```diff rsmpl
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_foo
- fn foo
+ fn bar
```

Single path suffices; `---`/`+++`/`index` are optional metadata; `rule=` may be omitted when a `#` directive line is present (diagnostics show `<unnamed>`; add `rule=name` when a patch has several hunks); elide the `+new,len` side (ignored for RSMPL hunks, SPEC 10.4). Every `@@ -old,len @@` cites ORIGINAL pre-patch lines, never rebased (SPEC 10). A hunk is RSmPL iff the header carries
`rule=` or at least one `#` directive line follows it; directive-free
renames use the terse form plus `rule=` (SPEC 7.1).

## Directives (SPEC 9.3)

| Directive | Meaning |
|---|---|
| `# allow=many` / `# sweep` | many matches; `# sweep` = `# allow=many` + `# scope=file` (diet) |
| `# repeat` | fold nested same-rule matches bottom-up (call + general-expr shapes; name args with `# expr`, `...` bodies stay verbatim) |
| `# optional` | zero matches OK (default: error) |
| `# sweep` | fused file sweep; dummy `@@ -1,1 @@`, no discovery |
| `E` / `T` / `P` / `V Q G W` / `I` / `N` | inferred (no declaration; SPEC 15.8); `N` is the fn-header name hole (binds per-site, replays; `# ident X` declares others) |
| `# bind=N` / `# use=N` | cut (`- I` + `# bind`) / paste (`+ N` + `# use`); `I` infers `item` |
| `# attrs=preserve` | default: ignore + preserve attrs/doc comments; write `#[...]` on its own diff line (single-line `#[a] let x = E;` never matches) |
| `# root=item\|expr\|stmt\|type` | force parse context |
| `# fallback=syntax` | syntactic match when `# symbol` can't resolve; bare `# symbol` verifies against the crate-local index (`symbol: resolved`); with `# follow=uses`, skip ambiguous/opaque sites instead of vetoing |
| `# follow=uses` | rewrite workspace use sites alongside a fn/item/field/variant/assoc rename (fn/item: unique-definition only, multi-def vetoes; fields anchor `.old` by receiver type — fn params, `let x: C`, `let x = S{…}`, `self` in `impl S` — plus `S`-labels with shorthand `S { old }` → `S { new: old }`, siblings never veto (`shadowed`/`off-target` skips); variants claim `E::Old` paths, bare `Old` needs `use E::*` else `second-hop`; assoc claims container-proven `Self::`/`S::`/`Tr::`/qself tails with per-block `shadowed` skips; trait-method headers claim `impl Tr for S` defs + anchored `recv.old()`/`S::old` calls; `mod` claims `old::`/`crate::old::` path uses plus its file move) (definition-only without it, with `orphans` evidence; raised as `item_intent` (items) / `member_rename` pre-pass (members, file/section-pure partial) / `mod_rename` pre-pass (`mod` + file move, file/section-pure partial) / `assoc_rename` pre-pass (assoc, one section per def file) / `trait_method_rename` pre-pass (trait-method, single trait hunk) when the cross-file probe proves the group) |
| `# returns=none` | minus-side requires no return type (fn only) |

## Shapes (SPEC 14/15/16/18.6)

```diff rsmpl
@@ -5,5 @@ rule=rename_calls
# allow=many
- foo(E)
+ bar(E)
```

```diff rsmpl
@@ -5,5 @@ rule=method_to_function
# allow=many
- Receiver.old_method(Arg)
+ new_function(Receiver, Arg)
```

- Fn rename: `fn foo(...)` matches plain fns only; write `pub`/`async`/generics explicitly (SPEC 14.1). Explicit slots evolve, `...`/same-meta
  preserved, bodies never rewritten (SPEC 14.7). A `trait T {` header + fn pair rewrites trait methods incl. `;` decls (never `pub`, SPEC 13.4).
- Call rewrite: add `# repeat` to fold nested same-rule matches in one
  hunk (`Box::new(Box::new(x))` → `x`, SPEC 17.8). General-expression
  rewrites (`E + 1` → `E + 2`) fold the same way; `let`/type/macro
  hunks reject `# repeat` — emit one ordered hunk per nesting level.
- Method-call rewrite: bind the receiver with one bare metavar, which
  binds the whole chain (`E.to_string()`); composite receivers never match — except a leading `&` (`- &E.path()`, SPEC 18.5).
- Symbol rewrite: `- Old(...)` / `+ New(...)` with `# allow=many`,
  bare `# symbol` (crate-index verified, vendored copies never match,
  `symbol: resolved`) or `# fallback=syntax` (exact text, `symbol: syntax`),
  `# symbol Old=crate::old_api` + `# symbol New=crate::new_api`.
  `use crate::old_api;` lets bare `old_api(...)` match; for plain renames
  prefer `- crate::old_api(E)` / `+ crate::new_api(E)` (SPEC 19).
- Item rename: `- struct Old` / `+ struct New` (also `enum`/`trait`/`type`/
  `const`/`static`/`mod`/`union`/`impl`), one ident edit — or one ident
  edit plus one closed-set vis step (`- pub struct Old` / `+ pub(crate) struct New`;
  same-name plus vis-only is a pure vis evolution, `- struct S` / `+ pub struct S`
  (uniform N≥2 steps sweep as `- struct N` / `+ pub struct N`) (SPEC 14.8).
  File, inline-mod, and block-level (fn bodies) items all match.
  A bare `impl Old`/`impl New` pair renames self-ty + trait-side path; add
  `# allow=many` when several match; definition only — follow up
  type/value/literal/variant/use sites per the SPEC 14.8 table. A joint
  `impl Trait for Type` rename states its trait + self-type pairs as two
  `# follow=uses` hunks sharing one rule (trait first); the composed
  `impl` header is follow-rewritten, never written.
- Definition RHS evolution: `- type A = Vec<T>;` / `+ type A = Box<[T]>;` (const/static likewise; struct-field/enum-payload/discriminant likewise under their container header; name preserved, SPEC 14.8).
- Move: cut `- I` (`# bind=N`; `I` infers `item`), paste `+ N` (`# use=N`);
  paste may precede cut; cross-file = two sections (SPEC 20); same-anchor pastes land in doc order with one blank between;
  `as` paste (`+ fn old as new(...)`) renames fn + self-calls; shadow/field/macro/attr interiors veto (line named, SPEC 20.2).
- Use-path prefix: `- use crate::old` / `+ use crate::new` (`# allow=many`) moves imports;
  grouped/glob/`as` tails preserved, members at the matched position rewrite too; regroup
  merges N≥2 `- use a::b::X;` into `+ use a::b::{X, Y};` (same order/vis, no attrs) (SPEC 14.9).
- Beyond calls: `- let x = E;`/`+ let y = E;`, `- E + 1`, `- Vec<T>` /
  `+ Box<[T]>` (`T` inferred), 2–4-line `let` sequences (per-position
  `P0`/`E0` holes). Bare `P` never matches `x: T` — write `P: T`.
  Table: SPEC 18.6 (mixed `let-else`/macro-type shapes stay ordinary hunks, permanent scope; macro *statements* match whole-shape opaque and `fn` items header-strict + body-structural inside blocks; `match`/loops/closures/`async`/`try`/`unsafe`/`yield`/`break`/`continue`/`let` match structurally).
- Ordinary hunks: text edits with no `#` lines; headers stay two-sided
  (`@@ -1,4 +1,5 @@` — new-side elision is RSmPL-only, SPEC 10.4/11; blank insertion needs bare `+`, `- foo` removal a `-- ` body line, SPEC 11).
- Macros: plain call patterns never reach inside macro invocations;
  `# macro=token_tree` renames verbatim (`M!(...)`→`N!(...)`) or binds one hole
  per top-level arg; shared-enclosing lines raise as one sweep (SPEC 21).

## Loop (SPEC D.3)

```sh
cargo build -p applyrs -p diffrs -p gitrs # gitrs re-execs target/debug siblings: rebuild first, else probes test a stale engine; then read target (1-based spans)
gitrs apply --file change.rsmpl --format json --diff-max-lines 0 --check
gitrs apply --file change.rsmpl --output full.patch   # full diff while preview stays small
gitrs apply --file change.rsmpl                       # apply for real
```

- Verified lowering is always 0-context; `--diff-max-lines` trims only the
  preview, `--context N` renders reported/`--output` for vanilla `git apply`;
  truncated stdout is review-only, never pipe it to `git apply` (D.2.1/D.2.2).
- Lint without a checkout: `--repo /tmp/empty --check` — `no match found` is the pass signal (SPEC D.2.4); re-check after.
- Verify the JSON `matches` count equals the sites you read — fewer means
  a site (e.g. inside a macro) needs an ordinary hunk; per-rule `spans`
  (`[[start,end], …]`, original-source lines) need no re-read.
- Piped patches avoid temp files: `cat change.rsmpl | gitrs apply --check`
  (no input flags means stdin; `--file -` is the explicit alias; `--patch`
  passes the diff inline, archived to `<repo>/.history/`, SPEC D.1).
  Inverse: `gitrs diff` prints pending changes as RSmPL (v1 renames/slot/call-sweeps compressed,
  binaries stripped, `.history/` skipped; SPEC D.4) — `--patch TEXT`/`--file PATH` raise from native text, `--check` verifies (T361; exit 2 naming file + line).

## Errors: code → fix (SPEC 25)

| Code | Fix |
|---|---|
| `RSMPL_RANGE_CONFLICT` | read the file, fix the range |
| `RSMPL_PARSE_CONFLICT` | target doesn't parse / empty patch — read file, add `diff --git` (count mismatch: if counts look right, check 1-char marker indent — an unprefixed line never enters the body; when the error names `suspect line N`, prefix line N with its marker and recount — `expected_header` is null there, SPEC 25.2) |
| `RSMPL_PATTERN_CONFLICT` | check SPEC 18.6 table; unknown directive/header-tag/`# root=` rejected; `# root=item` needs a single declared metavar (C16); `# repeat` needs a call/general-expr shape + bare spelling; `# symbol` is call-shape-only (C14); lone `# fallback` without `# symbol` + non-default `# attrs=` on call/macro shapes rejected (C15); same name under two kinds rejected, check SPEC 9.4; general template rewriting bound binder idents without binder identity rejected (C21 — multi-leaf destructuring renames need a single-binder spelling or an ordinary hunk; Stmt single-leaf destructuring completes like `let`; sequences exempt except a rename-once `let` rename (plain/destructuring, with authored init-substitution of later in-window uses, per-occurrence exclusion with hole-bound surfacing, outer-scope `new` declining follow with `capture-risk` orphans), which completes like `let`) |
| `RSMPL_MATCH_CONFLICT` | `no match found` (+ best-match preview in `reason`) — fix range/pattern, add `# optional`, or retry with `# sweep` + `@@ -1,1 @@`; stale ranged hunk retries once file-scoped with `drift` evidence (terse: uniqueness gate; anchored: context must agree, else original conflict, SPEC 10.8); too many — add `# allow=many`; if the preview shows mixed `let-else`/macro-type, use an ordinary hunk (SPEC 18.6 R1/R2/R4/R5; fn-body `use` — SPEC 14.9); matcher partial overlap (`partially overlapping matches cannot be merged`, MATCH, rule+file, no suggestion, SPEC 17.6/25.4) vs plan-stage overlap (`overlapping edits cannot be merged`, GIT, file-only + suggestion, SPEC 10.6/25.7) |
| `RSMPL_SYMBOL_CONFLICT` | add `# fallback=syntax` |
| `RSMPL_CAPTURE_CONFLICT` | `# use=N` without `# bind=N` anywhere in the patch — fix names (order irrelevant, paste may precede cut, SPEC 20.3) |
| `GIT_HUNK_CONFLICT` | rebase the ordinary-hunk context (the file line differs from the hunk text: paste `actual` from the repair or re-read the file; blank file lines are empty context: ` ` or empty line in-body) |
| `RSMPL_BUILD_CONFLICT` | post-lowering `--verify build` failure — read `repair.file` at `repair.range` and fix the build breakage (SPEC 25.10) |
| `RSMPL_IO_CONFLICT` | unreadable path or bad flags (`--format json` → stdout envelope) — fix per suggestion; absolute/`..`-escaping targets rejected |

JSON failures (`--format json`) print `{version, error: {code, rule, file, range, reason, suggestion, repair}}` to stdout only, exit 2. `repair` carries machine self-correction on every patch-input failure (SPEC D.2/25.9); `null` only on environmental IO.

## Dogfood rule

Every future Rust refactor starts here. Any failure, missing shape, or
confusing diagnostic becomes a TASKS.md work item before working around
it — the tool improves instead of the workaround sticking.

Full contract: SPEC.md. One-page reference: AGENTS.md.
