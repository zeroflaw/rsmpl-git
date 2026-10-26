# AGENTS.md — rsmpl-git one-page authoring reference

Agent? Start here. This file alone suffices for rename + call-rewrite +
move patches. SPEC.md is the full contract; citations below name the
normative section (SPEC 9.3, SPEC 14.1, SPEC 18.6, SPEC D.3, …).

## Patch skeleton

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

Rules: every semantic hunk = `@@ -old,len +new,len @@ rule=name`
header + `# lang=rust` directive + `-`/`+`/context lines. All `-old,len`
ranges cite the ORIGINAL pre-patch file, never rebased by earlier hunks
(SPEC 10). Ordinary Git hunks (no `#` directives) may mix in but must not
overlap semantic ranges (SPEC 10.6, SPEC 11).

## Directives you need

| Directive | Meaning (SPEC 9.3) |
|---|---|
| `# lang=rust` | required: marks a semantic hunk |
| `# allow=many` | many matches may rewrite (default `one`) |
| `# optional` | zero matches OK (default: error) |
| `# expr E` / `# type T` / `# item I` / `# pat P` | declare metavariable (SPEC 15) |
| `# item I` + `# bind=Name` / `# use=Name` | cut / paste a moved item (SPEC 9.3) |
| `# attrs=preserve` | default: ignore + preserve attrs/doc comments (SPEC 14.6) |
| `# root=item\|expr\|stmt\|type` | force parse context when ambiguous (SPEC 13.4) |
| `# fallback=syntax` | allow syntactic match when `# symbol` can't resolve (SPEC 19) |
| `# returns=none` | minus-side requires no return type (SPEC 14.2, fn only) |

Everything else lives in the full table (SPEC 9.3); you rarely need it.

## Shapes that work

Fn rename — `fn foo(...)` matches only plain fns; write `pub`/`async`/
generics explicitly to match them (SPEC 14.1). Slots written explicitly
evolve, `...`/same-metavar slots are preserved, bodies are never rewritten
(SPEC 14.7). Other items rename with a bare pair, one ident edit,
vis/generics/bodies preserved (SPEC 14.8):

```diff
- struct Old
+ struct New
```

Call rewrite — needs `# allow=many` for multiple sites, `# optional` to
tolerate zero (SPEC 17). Metavars bind once, repeats must match
structurally (SPEC 15.7, SPEC 17.7); literal `E` needs backticks when `E`
is declared (SPEC 15.4):

```diff
@@ -5,5 +5,5 @@ rule=rename_calls
# lang=rust
# allow=many
# expr E
- foo(E)
+ bar(E)
```

Move — cut with `# item I` + `# bind=Helper` + `- I`, paste with
`# use=Helper` + `+ Helper`; paste hunks may precede cut hunks; cross-file
moves are two file sections (SPEC 9.3, SPEC 10.5).

Beyond calls — `let` (`- let x = E;` / `+ let y = E;`), general
expressions (`- E + 1`), types (`- Vec<T>` / `+ Box<[T]>` with `# type T`).
A bare `P` never matches annotated `x: T` — write `P: T`. Bounded shape
set: full supported-vs-rejected table with exact errors is SPEC 18.6;
ellipsis positions are SPEC 16.

## Ranges without reading files

```sh
rsmpl-git --repo ./my-repo --inspect src/lib.rs
rsmpl-git --repo ./my-repo --inspect src/lib.rs --calls
rsmpl-git --repo ./my-repo --inspect src/ --calls
rsmpl-git --repo ./my-repo --inspect src --calls --max-items 20
```

Item spans locate definitions, `--calls` lists call/method/macro sites for
tight rewrite ranges, directory scan covers a subtree in one call, and
`--max-items N` bounds large scans (SPEC D.3, SPEC D.3.1, SPEC D.3.2,
SPEC D.3.3). Spans are 1-based inclusive, directly usable as `@@ -start,len`.

## Verify cheaply

```sh
rsmpl-git --patch - --repo ./my-repo < change.rsmpl
rsmpl-git --file change.rsmpl --repo ./my-repo --check
rsmpl-git --file change.rsmpl --repo ./my-repo --format json --check
rsmpl-git --file change.rsmpl --repo ./my-repo --format json --diff-max-lines 0 --check
rsmpl-git --file change.rsmpl --repo ./my-repo --check --context 0
rsmpl-git --file change.rsmpl --repo ./my-repo --output full.patch
```

Piped patches avoid temp files (SPEC D.1). JSON prints
`{rules, files_changed, files, diff, diff_truncated, check}` (SPEC D.2);
`--diff-max-lines N` (`0` omits) bounds only the reported diff while
`--check`/`--output`/apply use the full lowering (SPEC D.2.1); `--context N`
(default 3, `0` allowed) shrinks the lowering itself (SPEC D.2.2, SPEC 26.1).
`--inspect` takes no patch and rejects `--check`/`--output`/
`--diff-max-lines`/`--context`.

## Errors: code → fix (SPEC 25)

| Code | Fix |
|---|---|
| `RSMPL_RANGE_CONFLICT` | range exceeds file / not original coordinates — re-`--inspect` |
| `RSMPL_PARSE_CONFLICT` | target file doesn't parse as Rust — read the file |
| `RSMPL_PATTERN_CONFLICT` | pattern doesn't parse — check SPEC 18.6 table |
| `RSMPL_MATCH_CONFLICT` | `no match found` (range/pattern wrong, add `# optional`) or too many (add `# allow=many`) |
| `RSMPL_SYMBOL_CONFLICT` | add `# fallback=syntax` for syntactic matching |
| `RSMPL_CAPTURE_CONFLICT` | `# use=Name` without earlier `# bind=Name` — fix order/names |
| `GIT_HUNK_CONFLICT` | ordinary hunk failed — rebase the hunk context |
| `RSMPL_IO_CONFLICT` | unreadable patch/repo/output file — check paths |

With `--format json`, failures print `{error: {code, rule, file, range,
reason, suggestion}}` to stdout, exit 2 (SPEC D.2).

## Full reference

Format: SPEC 9.3 (directives), SPEC 13.4 (parse contexts), SPEC 14.1
(functions), SPEC 14.8 (other items), SPEC 15 (metavariables), SPEC 16
(ellipsis), SPEC 17.7 (equality), SPEC 18.6 (reject table), SPEC 19
(symbols), SPEC 25 (errors), SPEC 26.1 (lowering). CLI: SPEC D.1 (stdin),
SPEC D.2 (JSON), SPEC D.2.1 (budget), SPEC D.2.2 (context), SPEC D.3
(inspect), SPEC D.3.1 (calls), SPEC D.3.2 (dir), SPEC D.3.3 (budget).
Recommended loop: SPEC D.3.3 (inspect → author → check → apply).
