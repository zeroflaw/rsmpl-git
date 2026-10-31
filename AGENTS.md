# AGENTS.md — gitrs one-page authoring reference

Agent? Start here. This file alone suffices for rename + call-rewrite +
move patches. SPEC.md is the full contract; citations below name the
normative section (SPEC 9.3, SPEC 14.1, SPEC 18.6, SPEC D.3, …).
Skill: `skills/gitrs/SKILL.md` teaches the patch-first loop
(read → author → check → apply) for tool-driven agents.

## Patch skeleton

```diff rsmpl
diff --git src/lib.rs
@@ -1,5 @@ rule=rename_foo
- fn foo
+ fn bar
```

Minimal recipe: `diff --git <path>` alone suffices (single path ≡
`a/X b/X`; `---`/`+++`/`index` lines are optional metadata); `rule=`
may be omitted (diagnostics show `<unnamed>`; add `rule=name` when a
patch has several hunks). For RSmPL hunks the `+new,len` side is
ignored (SPEC 10.4), so elide it (`@@ -old,len @@`); copying the old
side verbatim (`@@ -old,len +old,len @@`) stays compatible.

Rules: every semantic hunk = `@@ -old,len @@` header with `rule=name` or at least one `#` directive line + `-`/`+`/context lines
(directive-free renames use the terse form plus `rule=`; hunks that already carry other directives omit `# lang=rust`, SPEC 7.1). All `-old,len`
ranges cite the ORIGINAL pre-patch file, never rebased by earlier hunks
(SPEC 10). Ordinary Git hunks (no `#` directives) may mix in but must not
overlap semantic ranges (SPEC 10.6, SPEC 11).

## Directives you need

| Directive | Meaning (SPEC 9.3) |
|---|---|
| `# sweep` | fused `# allow=many` + `# scope=file` (WI-1a); ranged multi-match keeps `# allow=many` alone |
| `# allow=many` | many matches may rewrite (default `one`); `# sweep` is the fused file-sweep form |
| `# repeat` | fold nested same-rule matches bottom-up: `Box::new(Box::new(x))` → `x` in one call-rewrite hunk (SPEC 17.8; call + general-expr shapes, `...` bodies opaque) |
| `# scope=file` | search whole file; `@@ -1,1 @@` dummy, no discovery needed (SPEC 17.3); prefer `# sweep` for sweeps |
| `# optional` | zero matches OK (default: error) |
| `E` / `T` / `P` / `V Q G W` / `I` | inferred metavariables — no declaration needed (SPEC 15.8); explicit `# expr E` etc. still legal |
| `# bind=Name` / `# use=Name` | cut (`- I` + `# bind=`) / paste (`+ Name` + `# use=`) a moved item; `- I` infers `item` (SPEC 15.8); `+ # allow=many` on the cut binds an ordered collection (SPEC 20.3) |
| `# attrs=preserve` | default: ignore + preserve attrs/doc comments (SPEC 14.6) |
| `# root=item\|expr\|stmt\|type` | force parse context when ambiguous (SPEC 13.4) |
| `# fallback=syntax` | allow syntactic match when `# symbol` can't resolve (SPEC 19); with `# follow=uses`, skip ambiguous/opaque sites instead of vetoing (SPEC 14.8) |
| `# follow=uses` | rewrite workspace use sites alongside a fn/item/field/variant rename (unique-definition only, multi-def vetoes; one-hop `pub use` re-exports followed, second-hop/off-target/glob-re-export vetoes; fields claim `.old` + `S`-labels with shorthand `S { old }` → `S { new: old }`, variants claim `E::Old` paths while bare `Old` stays opaque); definition-only without it, with `orphans` evidence (SPEC 14.8) |
| `# returns=none` | minus-side requires no return type (SPEC 14.2, fn only) |

Everything else lives in the full table (SPEC 9.3); you rarely need it.

## Shapes that work

Fn rename — `fn foo(...)` matches only plain fns; write `pub`/`async`/
generics explicitly to match them (SPEC 14.1). Slots written explicitly
evolve, `...`/same-metavar slots are preserved, bodies are never rewritten
(SPEC 14.7). A `trait T {` header + fn pair rewrites trait methods incl.
`;` decls (never `pub`, SPEC 13.4). Other items rename with a bare pair, one ident edit,
vis/generics/bodies preserved (SPEC 14.8; definition-only, follow-ups
per the 14.8 table — or one `# follow=uses` hunk for workspace uses):

```diff
- struct Old
+ struct New
```

Call rewrite — needs `# allow=many` for multiple sites, `# optional` to
tolerate zero (SPEC 17). Add `# repeat` to fold nested same-rule matches
(`Box::new(Box::new(x))` → `x`, SPEC 17.8; general-expr shapes fold too,
`let`/type/macro hunks reject it). Metavars bind once, repeats must match
structurally (SPEC 15.7, SPEC 17.7); literal `E` needs backticks when `E`
is declared (SPEC 15.4):

```diff rsmpl
@@ -5,5 @@ rule=rename_calls
# allow=many
- foo(E)
+ bar(E)
```

Method-call rewrite — same directives; one bare metavar binds the whole
receiver chain (`E.to_string()`); composite receivers never match — except a
leading `&`, which routes to general-expr matching (`- &E.path()` / `+ E.path()`, SPEC 18.5):

```diff rsmpl
@@ -5,5 @@ rule=method_to_function
# allow=many
- Receiver.old_method(Arg)
+ new_function(Receiver, Arg)
```

Move — cut with `# bind=Helper` + `- I` (`I` infers `item`), paste with
`# use=Helper` + `+ Helper`; paste hunks may precede cut hunks; cross-file
moves are two file sections (SPEC 9.3, SPEC 10.5). Same-anchor pastes land
in document order (first hunk topmost) with one blank line between items
(SPEC 20.2).

Use-path prefix — after a `mod` rename, move imports with one hunk
(`- use crate::old` / `+ use crate::new`, `# allow=many` or `# sweep`); plain, grouped,
glob, `as`, and `pub use` tails preserved, segment-wise; members at the
matched position rewrite too (`crate::{old, x}` → `crate::{new, x}`),
but never by suffix (`crate::{x::old}` is untouched) (SPEC 14.9).

Beyond calls — `let` (`- let x = E;` / `+ let y = E;`), general
expressions (`- E + 1`), types (`- Vec<T>` / `+ Box<[T]>`, `T` inferred),
2–4-line `let` sequences (per-position `P0`/`E0` holes, SPEC 16.4.2).
A bare `P` never matches annotated `x: T` — write `P: T`. Bounded shape
set: full supported-vs-rejected table with exact errors is SPEC 18.6
(mixed `let-else`, concrete-head `P @`, macro types stay ordinary hunks
(block macros opaque, `fn` items strict-header + body); blocks, `if`, `match`, loops, closures,
`async`/`try`/`unsafe` blocks, `yield`, `break`/`continue`, `let` expressions, agreed `let-else`,
and placeholder-head `P @` match structurally); ellipsis positions are SPEC 16.

## Ranges (file reads, not a CLI listing)

```sh
# Read the target file, note the 1-based spans to rewrite, cite
# @@ -start,len with L = end-start+1 (e.g. fn foo 1-3 → @@ -1,3 @@).
# Doc/attr lines count: they are part of the item span (/// + #[x] + fn 1-5 → @@ -1,5 @@).
```

Small file already held? Skip discovery: `@@ -1,1 @@` + `# sweep`
searches the whole file (SPEC 17.3).

## Verify cheaply

```sh
# Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
cat change.rsmpl | gitrs apply --check
gitrs apply --file change.rsmpl --check
gitrs apply --patch "$(cat change.rsmpl)" --check  # inline, no temp file
gitrs apply --file change.rsmpl --format json --check
gitrs apply --file change.rsmpl --format json --diff-max-lines 0 --check
gitrs apply --file change.rsmpl --output full.patch
```

Inverse: `gitrs diff` prints pending changes as an RSmPL patch (v1 renames/slot-sweeps/call-sweeps
compressed, rest ordinary; see README diff section, SPEC D.4; binaries stripped, `.history/` skipped).
Piped patches avoid temp files — no input flags means stdin (SPEC D.1).
`--file` names a patch file (avoids quoting for large patches); `--patch`
passes it inline (archived to `<repo>/.history/{id}.rsmpl`, last 128 kept,
path on stderr + JSON `history`). JSON prints `{version, rules, files, diff,`
`diff_truncated, diff_lines, diff_shown, checked, history}` (SPEC D.2);
`version` is the report schema (currently `3`); `checked` is `{lowering, semantics, build}` where `semantics` is `"verified"|"partial"` (each computed by a check that can fail); each `rules` entry carries `semantics` (`instantiated|postcondition|reparse-only|not-applicable`) plus per-match `spans` (`[[start,end], …]`, original-source 1-based lines, one per emitted edit — `len == matches` except evolution delete+insert pairs) plus per-site `sites` (`[{span, bindings}]`, omitted when nothing binds; cut values carry `binding_meta` digests, `--bindings full` lifts the cut); multi-edit rules add `span_edits` labels, skipped follows add `follow_skipped_details` reason triples;
JSON success and failure are both stdout-only (no stderr `check:`/`error:`
lines; stderr keeps `warning:` lines only); `--output` writes
the full diff before `--check` runs; on an empty lowering it removes a
stale file at that path, else creates nothing (mirror, SPEC D.2.3);
truncated stdout is review-only, never pipe it to `git apply` (SPEC D.2.3);
`--diff-max-lines N` (`0` omits) bounds only the reported diff while
`--check`/`--output`/apply use the full lowering (SPEC D.2.1); verified lowering
is always 0-context, `--context N` renders reported/`--output` for vanilla `git apply` (D.2.2).

## Errors: code → fix (SPEC 25)

| Code | Fix |
|---|---|
| `RSMPL_RANGE_CONFLICT` | range exceeds file — read the file, fix the range |
| `RSMPL_PARSE_CONFLICT` | target file doesn't parse as Rust — read the file; empty patch — provide a `diff --git` section (empty input is rejected, not a no-op); differing `diff --git a/old b/new` paths are a file rename section, applied via old-side content (SPEC 6) |
| `RSMPL_PATTERN_CONFLICT` | pattern doesn't parse — check SPEC 18.6 table; unknown directive (`# foo`, `# edition`, `# format_scope`) is rejected, check SPEC 9.3; unknown header tag (anything but `rule=`) is rejected, check SPEC 8.3; unknown `# root=` (only `item/expr/stmt/type`) is rejected, and `# root=item` needs a single declared metavar, check SPEC 13.4/18.6 C16; `# identifier/# path/# lifetime/# field` is rejected, check SPEC 15.1; `# repeat` needs a call/general-expr shape + bare spelling, check SPEC 17.8; item vis-prefix mismatch (`prefixes differ`) is rejected, check SPEC 14.8; `auto trait` headers rejected, check SPEC 14.8; lone `# fallback` without `# symbol` + non-default `# attrs=` on call/macro shapes rejected, check SPEC 18.6 C15; same name under two kinds rejected, check SPEC 9.4 |
| `RSMPL_MATCH_CONFLICT` | `no match found` (+ best-match-candidate preview in `reason`) — read the file, then fix range/pattern or add `# optional`; if the suggestion says exactly one match exists elsewhere, retry with `# scope=file` + `@@ -1,1 @@`; stale ranged hunk retries once file-scoped with `drift` evidence (terse: uniqueness gate; anchored: context must agree, else original conflict stands, SPEC 10.8); too many matches — add `# allow=many`; if the preview shows mixed `let-else`/macro-type, use an ordinary hunk (SPEC 18.6 R1/R2/R4/R5; fn-body `use` — SPEC 14.9); matcher partial overlap (`partially overlapping matches cannot be merged`, MATCH, rule+file, no suggestion, SPEC 17.6/25.4) vs plan-stage overlap (`overlapping edits cannot be merged`, GIT, file-only + suggestion, SPEC 10.6/25.7) |
| `RSMPL_SYMBOL_CONFLICT` | add `# fallback=syntax` for syntactic matching |
| `RSMPL_CAPTURE_CONFLICT` | `# use=Name` without matching `# bind=Name` anywhere in the patch (paste may precede cut, SPEC 20.3) — fix names |
| `GIT_HUNK_CONFLICT` | ordinary hunk failed — rebase the hunk context (blank file lines are empty context: ` ` or empty line in-body; blank-mismatch hints the prefix) |
| `RSMPL_IO_CONFLICT` | unreadable patch/repo/output file or bad flags (`--format json` → stdout envelope, else clap text) — check paths; absolute/`..`-escaping targets rejected (SPEC 27.6) |

With `--format json`, failures print `{version, error: {code, rule, file, range,
reason, suggestion, repair}}` to stdout only (no stderr `error:` line), exit 2 (SPEC D.2).
`repair` carries machine self-correction on every patch-input failure (SPEC D.2/25.9;
paste `fix_range`/`fix_directive`/`fix_header`/`fix_context` verbatim, follow the
`widen_scope`/`read_file`/`add_directive`/`fix_pattern`/`fix_name` template); `null` only on environmental IO.

## Full reference

Format: SPEC 9.3 (directives), SPEC 13.4 (parse contexts), SPEC 14.1
(functions), SPEC 14.8 (other items), SPEC 15 (metavariables), SPEC 16
(ellipsis), SPEC 17.7 (equality), SPEC 18.6 (reject table), SPEC 19
(symbols), SPEC 25 (errors), SPEC 26.1 (lowering). CLI: SPEC D.1 (sources
+ history), SPEC D.2 (JSON), SPEC D.2.1 (budget), SPEC D.2.2 (lowering),
SPEC D.3 (ranges).
Recommended loop: SPEC D.3 (read → author → check → apply).
