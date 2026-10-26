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
| `# repeat` | fold nested same-rule matches bottom-up (call shapes; name args with `# expr`, `...` bodies stay verbatim) |
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
  hunk (`Box::new(Box::new(x))` → `x`, SPEC 17.8).
- Method-call rewrite: bind the receiver explicitly
  (`Receiver.old_method(Arg)` → `new_function(Receiver, Arg)`, SPEC 18.5).
- Item rename: `- struct Old` / `+ struct New` (also `enum`/`trait`/`type`/
  `const`/`static`/`mod`/`union`/`impl`), one ident edit (SPEC 14.8).
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
  Full supported-vs-rejected table: SPEC 18.6.

## Loop (SPEC D.3.3)

```sh
# read the target file first — note the 1-based spans to rewrite
rsmpl-git --file change.rsmpl --format json --diff-max-lines 0 --context 0 --check
rsmpl-git --file change.rsmpl --output full.patch   # full diff while preview stays small
rsmpl-git --file change.rsmpl                       # apply for real
```

- `--context 0` shrinks the lowering itself (still appliable);
  `--diff-max-lines` trims only the preview; truncated stdout is
  review-only, never pipe it to `git apply` (SPEC D.2.1/D.2.2).
- Lint without a checkout: `--repo /tmp/empty --check` — `no match found`
  is the pass signal (SPEC D.2.4); re-check against the real repo after.
- Piped patches avoid temp files: `cat change.rsmpl | rsmpl-git --patch -`
  (canonical stdin spelling; `--file -` is the symmetric alias, SPEC D.1).

## Errors: code → fix (SPEC 25)

| Code | Fix |
|---|---|
| `RSMPL_RANGE_CONFLICT` | read the file, fix the range |
| `RSMPL_PARSE_CONFLICT` | target doesn't parse / empty patch — read file, add `diff --git` |
| `RSMPL_PATTERN_CONFLICT` | check SPEC 18.6 table; unknown directive/header-tag/`# root=` rejected; `# repeat` needs a call shape + bare spelling |
| `RSMPL_MATCH_CONFLICT` | `no match found` (+ nearest preview in `reason`) — fix range/pattern, add `# optional`, or retry with `# scope=file` + `@@ -1,1 @@`; too many — add `# allow=many` |
| `RSMPL_SYMBOL_CONFLICT` | add `# fallback=syntax` |
| `RSMPL_CAPTURE_CONFLICT` | `# use=N` without `# bind=N` — fix order/names |
| `GIT_HUNK_CONFLICT` | rebase the ordinary-hunk context |
| `RSMPL_IO_CONFLICT` | unreadable path — fix per suggestion; absolute/`..`-escaping targets rejected |

JSON failures (`--format json`) print `{error: {code, rule, file, range,
reason, suggestion}}` to stdout only, exit 2.

## Dogfood rule

Every future Rust refactor starts here. Any failure, missing shape, or
confusing diagnostic becomes a TASKS.md work item before working around
it — the tool improves instead of the workaround sticking.

Full contract: SPEC.md. One-page reference: AGENTS.md.
