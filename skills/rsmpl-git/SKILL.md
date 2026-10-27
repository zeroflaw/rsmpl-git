---
name: rsmpl-git
description: Patch-first Rust refactoring with rsmpl-git semantic hunks (read, author, check, apply).
---

# rsmpl-git skill

Use `rsmpl-git` for every Rust refactoring or modification — never hand-edit
what a semantic hunk can express. Text edits that need no semantics still go
through `rsmpl-git` as ordinary hunks, so one tool owns every change.

## When to use / when not to

- Use: rename (fn/item), call rewrites, moves, signature evolution,
  `let`/expr/type rewrites — anything with `-`/`+` pattern lines.
- Viewing only: use a targeted file read — the CLI lists nothing.
- Small file already held: skip discovery entirely — `@@ -1,1 @@` plus
  `# scope=file` searches the whole file (SPEC 17.3).

## Minimal patch

```diff
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

Single path suffices; `---`/`+++`/`index` are optional metadata; `rule=`
may be omitted for single hunks; elide the `+new,len` side (ignored for
RSmPL hunks, SPEC 10.4). Every `@@ -old,len @@` cites ORIGINAL pre-patch
lines, never rebased (SPEC 10). Bare renames keep `# lang=rust` as the
marker; hunks with other directives omit it (SPEC 7.1).

## Directives (SPEC 9.3)

| Directive | Meaning |
|---|---|
| `# allow=many` | many matches may rewrite (default `one`) |
| `# repeat` | fold nested same-rule matches bottom-up (call + general-expr shapes; name args with `# expr`, `...` bodies stay verbatim) |
| `# optional` | zero matches OK (default: error) |
| `# scope=file` | whole-file search; dummy `@@ -1,1 @@`, no discovery |
| `# expr E` / `# type T` / `# item I` / `# pat P` | declare metavariable |
| `# item I` + `# bind=N` / `# use=N` | cut / paste a moved item |
| `# attrs=preserve` | default: ignore + preserve attrs/doc comments |
| `# root=item\|expr\|stmt\|type` | force parse context |
| `# fallback=syntax` | syntactic match when `# symbol` can't resolve |
| `# returns=none` | minus-side requires no return type (fn only) |

## Shapes (SPEC 14/15/16/18.6)

```diff
@@ -5,5 @@ rule=rename_calls
# allow=many
# expr E
- foo(E)
+ bar(E)
```

```diff
@@ -5,5 @@ rule=method_to_function
# allow=many
# expr Receiver
# expr Arg
- Receiver.old_method(Arg)
+ new_function(Receiver, Arg)
```

- Fn rename: `fn foo(...)` matches plain fns only; write `pub`/`async`/
  generics explicitly (SPEC 14.1). Explicit slots evolve, `...`/same-meta
  preserved, bodies never rewritten (SPEC 14.7).
- Call rewrite: add `# repeat` to fold nested same-rule matches in one
  hunk (`Box::new(Box::new(x))` → `x`, SPEC 17.8). General-expression
  rewrites (`E + 1` → `E + 2`) fold the same way; `let`/type/macro
  hunks reject `# repeat` — emit one ordered hunk per nesting level.
- Method-call rewrite: bind the receiver explicitly
  (`Receiver.old_method(Arg)` → `new_function(Receiver, Arg)`, SPEC 18.5).
- Symbol rewrite: `- Old(...)` / `+ New(...)` with `# allow=many`,
  `# fallback=syntax` (required in v0.1 — a bare `# symbol` hunk fails),
  `# symbol Old=crate::old_api` + `# symbol New=crate::new_api`.
  v0.1 matches the full path exactly (bare `old_api(...)` does not match
  `crate::old_api`); for plain renames prefer `- crate::old_api(E)` /
  `+ crate::new_api(E)` — identical matching, fewer directives (SPEC 19).
- Item rename: `- struct Old` / `+ struct New` (also `enum`/`trait`/`type`/
  `const`/`static`/`mod`/`union`/`impl`), one ident edit (SPEC 14.8).
  File, inline-mod, and block-level (fn bodies) items all match.
  A bare `impl Old` / `impl New` pair renames both the self-ty and the
  trait-side path (`impl Old for Foo` → `impl New for Foo`); add
  `# allow=many` when several headers match.
- Move: cut `- I` (`# item I` + `# bind=N`), paste `+ N` (`# use=N`);
  paste may precede cut; cross-file = two sections (SPEC 20).
- Use-path prefix: `- use crate::old` / `+ use crate::new` (`# allow=many`)
  moves imports after a `mod` rename; grouped/glob/`as` tails preserved,
  segment-wise (SPEC 14.9).
- Beyond calls: `- let x = E;`/`+ let y = E;`, `- E + 1`, `- Vec<T>` /
  `+ Box<[T]>` (`# type T`). Bare `P` never matches `x: T` — write `P: T`.
  Table: SPEC 18.6 (control-flow/`let-else`/`P @`/macro-type shapes stay ordinary hunks, permanent scope).
- Ordinary hunks: text edits with no `#` lines; headers stay two-sided
  (`@@ -1,4 +1,5 @@` — new-side elision is RSmPL-only, SPEC 10.4/11).
- Macros: plain call patterns never reach inside macro invocations;
  `# macro=token_tree` is verbatim-only (`- M!(...)` / `+ N!(...)` renames);
  edits inside macro bodies go through ordinary hunks (SPEC 21).

## Loop (SPEC D.3)

```sh
# read the target file first — note the 1-based spans to rewrite
rsmpl-git --file change.rsmpl --format json --diff-max-lines 0 --check
rsmpl-git --file change.rsmpl --output full.patch   # full diff while preview stays small
rsmpl-git --file change.rsmpl                       # apply for real
```

- The lowering itself is always 0-context (minimal tokens, still appliable);
  `--diff-max-lines` trims only the preview; truncated stdout is
  review-only, never pipe it to `git apply` (SPEC D.2.1/D.2.2).
- Lint without a checkout: `--repo /tmp/empty --check` — `no match found`
  is the pass signal (SPEC D.2.4); re-check against the real repo after.
- Verify the JSON `matches` count equals the sites you read before
  applying — fewer means a site (e.g. inside a macro) needs an ordinary hunk.
  Each rule entry also carries `spans` (`[[start,end], …]`,
  original-source 1-based lines) so verification needs no re-read.
- Piped patches avoid temp files: `cat change.rsmpl | rsmpl-git --check`
  (no input flags means stdin; `--file -` is the explicit alias, SPEC D.1).

## Errors: code → fix (SPEC 25)

| Code | Fix |
|---|---|
| `RSMPL_RANGE_CONFLICT` | read the file, fix the range |
| `RSMPL_PARSE_CONFLICT` | target doesn't parse / empty patch — read file, add `diff --git` |
| `RSMPL_PATTERN_CONFLICT` | check SPEC 18.6 table; unknown directive/header-tag/`# root=` rejected; `# repeat` needs a call/general-expr shape + bare spelling; `# symbol` is call-shape-only (C14) |
| `RSMPL_MATCH_CONFLICT` | `no match found` (+ nearest preview in `reason`) — fix range/pattern, add `# optional`, or retry with `# scope=file` + `@@ -1,1 @@`; too many — add `# allow=many` |
| `RSMPL_SYMBOL_CONFLICT` | add `# fallback=syntax` |
| `RSMPL_CAPTURE_CONFLICT` | `# use=N` without `# bind=N` — fix order/names |
| `GIT_HUNK_CONFLICT` | rebase the ordinary-hunk context |
| `RSMPL_IO_CONFLICT` | unreadable path or bad flags (`--format json` → stdout envelope) — fix per suggestion; absolute/`..`-escaping targets rejected |

JSON failures (`--format json`) print `{version, error: {code, rule, file,
range, reason, suggestion}}` to stdout only, exit 2.

## Dogfood rule

Every future Rust refactor starts here. Any failure, missing shape, or
confusing diagnostic becomes a TASKS.md work item before working around
it — the tool improves instead of the workaround sticking.

Full contract: SPEC.md. One-page reference: AGENTS.md.
