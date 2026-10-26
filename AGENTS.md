# AGENTS.md — rsmpl-git one-page authoring reference

Agent? Start here. This file alone suffices for rename + call-rewrite +
move patches. SPEC.md is the full contract; citations below name the
normative section (SPEC 9.3, SPEC 14.1, SPEC 18.6, SPEC D.3, …).
Skill: `skills/rsmpl-git/SKILL.md` teaches the patch-first loop
(read → author → check → apply) for tool-driven agents.

## Patch skeleton

```diff
diff --git src/lib.rs
@@ -1,5 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
   }
```

Minimal recipe: `diff --git <path>` alone suffices (single path ≡
`a/X b/X`; `---`/`+++`/`index` lines are optional metadata); `rule=`
may be omitted (diagnostics show `<unnamed>`; add `rule=name` when a
patch has several hunks). For RSmPL hunks the `+new,len` side is
ignored (SPEC 10.4), so elide it (`@@ -old,len @@`); copying the old
side verbatim (`@@ -old,len +old,len @@`) stays compatible.

Rules: every semantic hunk = `@@ -old,len @@ rule=name`
header (+new,len elided, RSmPL-only) + at least one `#` directive line + `-`/`+`/context lines
(`# lang=rust` alone is the marker for bare renames; hunks that
already carry other directives omit it, SPEC 7.1). All `-old,len`
ranges cite the ORIGINAL pre-patch file, never rebased by earlier hunks
(SPEC 10). Ordinary Git hunks (no `#` directives) may mix in but must not
overlap semantic ranges (SPEC 10.6, SPEC 11).

## Directives you need

| Directive | Meaning (SPEC 9.3) |
|---|---|
| `# lang=rust` | optional: marker for directive-free hunks; omit when other `#` lines present (SPEC 7.1) |
| `# allow=many` | many matches may rewrite (default `one`) |
| `# scope=file` | search whole file; `@@ -1,1 @@` dummy, no discovery needed (SPEC 17.3) |
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
@@ -5,5 @@ rule=rename_calls
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

## Ranges (file reads, not a CLI listing)

```sh
# Read the target file, note the 1-based spans to rewrite, cite
# @@ -start,len with L = end-start+1 (e.g. fn foo 1-3 → @@ -1,3 @@).
```

Small file already held? Skip discovery: `@@ -1,1 @@` + `# scope=file`
searches the whole file (SPEC 17.3).

## Verify cheaply

```sh
# Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
rsmpl-git --patch - < change.rsmpl
rsmpl-git --file change.rsmpl --check
rsmpl-git --file change.rsmpl --format json --check
rsmpl-git --file change.rsmpl --format json --diff-max-lines 0 --check
rsmpl-git --file change.rsmpl --check --context 0
rsmpl-git --file change.rsmpl --output full.patch
```

Piped patches avoid temp files (SPEC D.1). JSON prints
`{rules, files, diff, diff_truncated, diff_lines, diff_shown, check}` (SPEC D.2);
JSON success and failure are both stdout-only (no stderr `check:`/`error:`
lines; stderr keeps `warning:` lines only); `--output` writes
the full diff before `--check` runs but nothing when the lowering is empty;
truncated stdout is review-only, never pipe it to `git apply` (SPEC D.2.3);
`--diff-max-lines N` (`0` omits) bounds only the reported diff while
`--check`/`--output`/apply use the full lowering (SPEC D.2.1); `--context N`
(default 3, `0` allowed) shrinks the lowering itself (SPEC D.2.2, SPEC 26.1).

## Errors: code → fix (SPEC 25)

| Code | Fix |
|---|---|
| `RSMPL_RANGE_CONFLICT` | range exceeds file — read the file, fix the range |
| `RSMPL_PARSE_CONFLICT` | target file doesn't parse as Rust — read the file; empty patch — provide a `diff --git` section (empty input is rejected, not a no-op) |
| `RSMPL_PATTERN_CONFLICT` | pattern doesn't parse — check SPEC 18.6 table; unknown directive (`# foo`, `# edition`, `# format_scope`) is rejected, check SPEC 9.3; unknown header tag (anything but `rule=`) is rejected, check SPEC 8.3; unknown `# root=` (only `item/expr/stmt/type`) is rejected, check SPEC 13.4; `# identifier/# path/# lifetime/# field` is rejected, check SPEC 15.1 |
| `RSMPL_MATCH_CONFLICT` | `no match found` (+ nearest-candidate preview in `reason`) — read the file, then fix range/pattern or add `# optional`; if the suggestion says exactly one match exists elsewhere, retry with `# scope=file` + `@@ -1,1 @@`; too many matches — add `# allow=many` |
| `RSMPL_SYMBOL_CONFLICT` | add `# fallback=syntax` for syntactic matching |
| `RSMPL_CAPTURE_CONFLICT` | `# use=Name` without earlier `# bind=Name` — fix order/names |
| `GIT_HUNK_CONFLICT` | ordinary hunk failed — rebase the hunk context |
| `RSMPL_IO_CONFLICT` | unreadable patch/repo/output file — check paths; absolute/`..`-escaping targets rejected (SPEC 27.6) |

With `--format json`, failures print `{error: {code, rule, file, range,
reason, suggestion}}` to stdout only (no stderr `error:` line), exit 2 (SPEC D.2).

## Full reference

Format: SPEC 9.3 (directives), SPEC 13.4 (parse contexts), SPEC 14.1
(functions), SPEC 14.8 (other items), SPEC 15 (metavariables), SPEC 16
(ellipsis), SPEC 17.7 (equality), SPEC 18.6 (reject table), SPEC 19
(symbols), SPEC 25 (errors), SPEC 26.1 (lowering). CLI: SPEC D.1 (stdin),
SPEC D.2 (JSON), SPEC D.2.1 (budget), SPEC D.2.2 (context), SPEC D.3
(ranges).
Recommended loop: SPEC D.3 (read → author → check → apply).
