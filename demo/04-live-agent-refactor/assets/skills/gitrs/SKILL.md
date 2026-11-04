---
name: gitrs
description: Patch-first Rust refactoring with gitrs semantic hunks (read, author, check, apply).
---

# gitrs skill

Use `gitrs` for every Rust refactoring or modification — never hand-edit what a semantic hunk can express. Text edits that need no semantics still go through `gitrs` as ordinary hunks, so one tool owns every change. `gitrs apply` re-execs the sibling `applyrs` binary and `gitrs diff` the sibling `diffrs` (same flags either way; calling `applyrs`/`diffrs` directly is equivalent); all three must be built together at one version or the dispatcher fails loud on skew — rebuild first, else probes test a stale engine.

## When to use / when not to

- Use: rename (fn/item/field/variant), call rewrites, moves, signature evolution, `let`/expr/type rewrites — anything with `-`/`+` pattern lines.
- Viewing only: use a targeted file read — the CLI lists nothing.
- Small file already held: skip discovery entirely — `@@ -1,1 @@` plus `# sweep` searches the whole file (SPEC 17.3).

## Workflow: one patch, few calls

1. Grep every name first (one search per name); note each site's file + line, then read only the spans you will cite.
2. Compose the ENTIRE refactor as one patch (every file, every hunk) from the complete examples below — standard shapes (item/fn/method/field rename, call sweep, use line, move, ordinary) need NO probe; substitute your names and cite your spans.
3. Validate with ONE `gitrs apply --file patch.rsmpl --format json --diff-max-lines 0 --check`; read `matches`/`spans`/`files` from that output — never re-run a check to summarize it.
4. On a hunk error, fix from the `repair` block and re-check the whole patch (one call). Budget: two attempts per site, then that site becomes an ordinary hunk — never a third probe of the same shape, never a deliberate negative probe (no-fallback after a symbol conflict, bare method after a nearest-fn miss).
5. Confirm `matches` covers every grepped site, then apply ONCE. A no-match failure on re-apply means it already landed — check `git diff`, don't re-apply.
6. Verify the build (`cargo check` plus whatever tests the task names).

## Minimal patch

```diff rsmpl
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_foo
- fn foo
+ fn bar
```

Single path suffices (`a/X b/X` is the same section); `---`/`+++`/`index` are optional metadata; differing `a/old b/new` paths are a file-rename section applied via old-side content, and `/dev/null` halves create/delete files (SPEC 6). `rule=` may be omitted when a `#` directive line is present (diagnostics show `<unnamed>`; add `rule=name` when a patch has several hunks); elide the `+new,len` side (ignored for RSMPL hunks, SPEC 10.4). Every `@@ -old,len @@` cites ORIGINAL pre-patch lines, never rebased (SPEC 10), and doc/attr lines count in the span. A hunk is RSmPL iff the header carries `rule=` or at least one `#` directive line follows it; directive-free renames use the terse form plus `rule=` (SPEC 7.1). Factored diet (SPEC D.5): later hunks of a shared-rule run go bare — `@@ -old,len @@` with no `rule=`/`#` lines inherits the section's most-recent RSmPL hunk (same file+rule+directives only; never across files/rules). Ordinary hunks may mix in but must not overlap semantic ranges (SPEC 10.6). Batching: one refactor = one patch (every file, every hunk) validated by a single `--check` — original-file ranges never shift each other, so per-sub-task applies only multiply round trips; compose from the complete examples below and probe only genuinely novel shapes.

## Directives (SPEC 9.3)

| Directive | Meaning |
|---|---|
| `# lang=rust` | optional explicitness; hunks default to Rust (any other `lang=` rejected) |
| `# allow=one\|many` / `# sweep` | match cardinality (default `one`); `# sweep` fuses `# allow=many` + `# scope=file` — ranged multi-match keeps `# allow=many` alone |
| `# scope=contained\|anchor\|overlap\|exact\|file` | range relation of the match (default `contained`); `file` searches the whole file with dummy `@@ -1,1 @@`, no discovery (SPEC 17.3) |
| `# repeat` | fold nested same-rule matches bottom-up (call + general-expr shapes only; `...` bodies stay verbatim) |
| `# optional` | zero matches OK (default: error); `# optional=false` restores the error |
| `E` / `T` / `P` / `V Q G W` / `I` / `N` | inferred metavariables — no declaration needed (SPEC 15.8); `N` is the fn-header name hole (binds per-site, replays; `# ident X` declares others); explicit `# expr E` / `# type T` / `# item I` etc. stay legal, and a literal spelled like a declared metavar needs backticks (`` `E` ``, SPEC 15.4); in type/expression patterns a bare capitalized ident is a hole, so backtick concrete names (`` `Widget`<T> ``) — item/fn-signature patterns take them bare (`- struct Widget`) |
| `# bind=N` / `# use=N` | cut (`- I` + `# bind=`) / paste (`+ N` + `# use=`); `I` infers `item`; `# bind=` + `# allow=many` on the cut binds an ordered collection, and an `N`-hole paste template (`+ fn new_N(...) { ... }`) renames each item per item (SPEC 20.3) |
| `# attrs=preserve\|absent\|explicit\|strip` | attribute policy (default `preserve`: ignore + preserve attrs/doc comments; written attr lines are the match delta on fn/item/`let`; uniform attr deltas over hetero items raise via `attr_sweep`, SPEC 14.6/D.4); write `#[...]` on its own diff line (single-line `#[a] let x = E;` never matches) |
| `# root=item\|expr\|stmt\|type` | force parse context when ambiguous (SPEC 13.4) |
| `# symbol Name=path` / `# fallback=syntax` | bare `# symbol` verifies seeds against the crate-local index (`symbol: resolved`); `# fallback=syntax` matches exact text when resolution fails (`symbol: syntax`); with `# follow=uses`, ambiguous/opaque sites skip instead of vetoing; `# fallback=syntax` also rides without `# symbol` (e.g. beside `# follow=uses`) for purely syntactic matching |
| `# follow=uses` | rewrite workspace use sites alongside a fn/item/field/variant/assoc rename (fn/item: unique-definition only, multi-def vetoes; one-hop `pub use` re-exports followed, second-hop/off-target/glob-re-export vetoes; fields anchor `.old` by receiver type — fn params, `let x: C`, `let x = S{…}`, `self` in `impl S` — plus `S`-labels with shorthand `S { old }` → `S { new: old }`, siblings never veto (`shadowed`/`off-target` skips); variants claim `E::Old` paths, bare `Old` needs `use E::*` else `second-hop`; assoc claims container-proven `Self::`/`S::`/`Tr::`/qself tails with per-block `shadowed` skips; trait-method headers claim `impl Tr for S` defs + anchored `recv.old()`/`S::old` calls; `mod` claims `old::`/`crate::old::` path uses plus its file move) (definition-only without it, with `orphans` evidence; raised as `item_intent` (items) / `member_rename` pre-pass (members, file/section-pure partial) / `mod_rename` pre-pass (`mod` + file move, file/section-pure partial) / `assoc_rename` pre-pass (assoc, one section per def file) / `trait_method_rename` pre-pass (trait-method, single trait hunk) when the cross-file probe proves the group) |
| `# returns=none` | minus-side requires no return type (fn only, SPEC 14.2) |
| `# mode=ast\|tokens`, `# macro=invocation\|token_tree\|expanded`, `# format=preserve`, `# metavars=bare\|sigiled` | rarely needed: parse/macro/format/metavar spelling modes (defaults `ast`/`invocation`/`preserve`/`bare`; `format=rustfmt\|canonical` rejected in v0.1 — run `cargo fmt` after apply) |

<!-- generated: directives start -- do not hand-edit; run `python3 tools/gen_directives.py --write` -->
Directive inventory (code is the source: `directives.rs` + `model.rs`): 16 keys, 3 bare flags, 20 kinds.
Keys: lang, allow, optional, scope, bind, use, attrs, mode, macro, format, returns, root, fallback, metavars, symbol, follow.
Bare: optional, repeat, sweep.
Kinds: identifier, ident, path, symbol, expr, stmt, item, type, pat, lifetime, generic_params, where_clause, visibility, qualifier, qualifier_list, attributes, block, field, macro, token_tree.
<!-- generated: directives end -->

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

Struct rename with use-site follow (verify `matches` covers every site you grepped; sites the follow skips get explicit `# sweep` hunks, use lines inside fn bodies get ordinary hunks):

```diff rsmpl
diff --git src/lib.rs
@@ -8,7 @@ rule=rename_widget
# follow=uses
# fallback=syntax
- pub struct Widget
+ pub struct Gadget
```

Fn rename, full form (untouched slots elided; `...` and `}` keep the one-space marker):

```diff rsmpl
@@ -10,7 @@ rule=rename_foo
- pub fn foo<T>(...) where T: Clone {
+ pub fn bar<T>(...) where T: Clone {
    ...
 }
```

Method rename (container header spells generics exactly as written):

```diff rsmpl
@@ -24,17 @@ rule=rename_bite
  impl<T> Widget<T> {
- pub fn bite(...) {
+ pub fn chew(...) {
    ...
 }
```

Field rename (reads follow by receiver anchoring):

```diff rsmpl
@@ -8,7 @@ rule=rename_field
# follow=uses
  struct Widget {
- tooth: T
+ fang: T
```

- Fn rename: `fn foo(...)` matches plain fns only; write `pub`/`async`/`unsafe`/generics/`where` explicitly to match them (SPEC 14.1). Explicit slots evolve, `...`/same-metavar slots are preserved, bodies are never rewritten (SPEC 14.7); rename+body joints (one to three `- E`/`+ E` pairs inside the renamed body) ride the same `# follow=uses` hunk. A braced body is an `...` context line plus the closing `}` context line — both keep the one-space hunk marker (`    ...`, ` }`); a bare unprefixed `}` never enters the hunk and fails as an unbalanced body block. A `trait T {` header + fn pair rewrites trait methods incl. `;` decls (never `pub`, SPEC 13.4); an `impl S {` header + fn pair rewrites inherent methods the same way. Slots run `fn name<generics>(params) -> ret where...` — elide what you don't touch (`(...)`, no return); container headers spell generics exactly as written (`impl<T, F> S<T, F> {`).
- Call rewrite: needs `# allow=many` for multiple sites, `# optional` to tolerate zero (SPEC 17). Add `# repeat` to fold nested matches (`Box::new(Box::new(x))` → `x`, SPEC 17.8; general-expr shapes fold too, `let`/type/macro hunks reject it). Metavars bind once and repeats must match structurally (SPEC 15.7/17.7). A qualified line with exactly two disjoint deltas (`old::foo(E)` → `new::bar(E)`, or shared receiver+method `old_mod::obj.old_m(E)` → `new_mod::obj.new_m(E)`) composes under one rule; three deltas stay ordinary, and hetero fused hunks partition per contiguous run (declining runs stay ordinary, SPEC D.4).
- Method-call rewrite: bind the receiver with one bare metavar (binds the whole receiver chain) (`E.to_string()`); composite receivers never match — except a leading `&`, which routes to general-expr matching (`- &E.path()` / `+ E.path()`, SPEC 18.5).
- Symbol rewrite: `- Old(...)` / `+ New(...)` with bare `# symbol` (crate-index verified, vendored copies never match) or `# fallback=syntax` (exact text), or `# symbol Old=crate::old_api` + `# symbol New=crate::new_api`; `use crate::old_api;` lets bare `old_api(...)` match; general positions take whole paths and single-leading prefixes (`Old::foo`), field types take the same law (`x: Old`, `x: Vec<Old>`), use-prefixes take a symbol head (`use Old::sub`) (SPEC 19).
- Item rename: `- struct Old` / `+ struct New` (also `enum`/`trait`/`type`/`const`/`static`/`mod`/`union`/`impl`), one ident edit — or one ident edit plus one closed-set vis step (`- pub struct Old` / `+ pub(crate) struct New`; same-name plus vis-only is a pure vis evolution, `- struct S` / `+ pub struct S`; uniform N≥2 steps sweep as `- struct N` / `+ pub struct N` with `# allow=many`) (SPEC 14.8). File, inline-mod, and block-level (fn bodies) items all match. A bare `impl Old` pair renames self-ty + trait-side path (definition only — follow up use sites per the SPEC 14.8 table, or one `# follow=uses` hunk). A joint `impl Trait for Type` rename states trait + self-type pairs as two `# follow=uses` hunks sharing one rule (trait first); the composed `impl` header is follow-rewritten, never written.
- Definition RHS evolution: `- type A = Vec<T>;` / `+ type A = Box<[T]>;` (const/static likewise; struct-field/enum-payload/discriminant likewise under their container header; name preserved, SPEC 14.8). Rename+member evolution (def pair above member pair, no header), field-add/del and variant add/del (one member, shared-rule two-hunk emission), and collection moves compose under one rule (SPEC 14.8/20).
- Move: cut `- I` (`# bind=N`; `I` infers `item`), paste `+ N` (`# use=N`); paste may precede cut; cross-file = two sections (SPEC 20); same-anchor pastes land in document order with one blank line between items; `as` paste (`+ fn old as new(...)`) renames the moved fn + self-calls, joint (`+ fn old as new(x: i64) -> i64 { ... }`) adds one sig-slot evolution, container (`+ struct/enum/union old as new { ... }`) renames container + self-referential field types / variant payloads, trait (`+ trait old as new { ... }`) renames trait + self-referential method signatures/bodies, joint-trait (`+ trait old as new { fn m_old as m_new; ... }`) adds one method rename, inherent-`impl` (`+ impl old as new { ... }`) renames `impl` block + self-ty self-paths; shadow/field/macro/attr interiors veto (`RSMPL_PATTERN_CONFLICT`), two evolution slots stay ordinary; move+attr (E1) / trait-move (E2) ride the same cut/paste probe with the delta as ordinary leftover (SPEC 20); file-module move (deleted+created file pair, N≥1 items, stems match) joins its `mod old;`→`mod new;` pair under the first cut's rule (SPEC 20.6).
- Use-path prefix: `- use crate::old` / `+ use crate::new` (`# allow=many` or `# sweep`) moves imports after a `mod` rename; plain/grouped/glob/`as`/`pub use` tails preserved, segment-wise; members at the matched position rewrite too (`crate::{old, x}` → `crate::{new, x}`), never by suffix; regroup merges N≥2 `- use a::b::X;` into `+ use a::b::{X, Y};` and splits the inverse (same order/vis, no attrs; `as`/glob/nested stay ordinary) (SPEC 14.9).
- Beyond calls: `- let x = E;`/`+ let y = E;`, General-expression rewrites (`- E + 1`), types (`` - `Vec`<T> `` / `` + `Box`<[T]> ``, `T` inferred — the backticks are required, a bare `Box` reads as an unbound hole), 2–4-line `let` sequences (per-position `P0`/`E0` holes, SPEC 16.4.2). A bare `P` never matches annotated `x: T` — write `P: T`. Ellipsis `...` is legal only in SPEC 16 positions (fn/arg/body/path slots). Full supported-vs-rejected table with exact errors: SPEC 18.6 (mixed `let-else`, concrete-head `P @`, macro types stay ordinary hunks; block macros opaque, `fn` items strict-header + body; blocks, `if`, `match`, loops, closures, `async`/`try`/`unsafe` blocks, `yield`, `break`/`continue`, `let` expressions, agreed `let-else`, placeholder-head `P @`, array-repeat `[E; N]`, and `&raw` addr match structurally).
- Ordinary hunks: text edits with no `#` lines; headers stay two-sided (`@@ -1,4 +1,5 @@` — new-side elision is RSmPL-only, SPEC 10.4/11; blank insertion needs bare `+`, `- foo` removal a `-- ` body line, SPEC 11); content starts immediately after the `+`/`-` marker — a space there becomes part of the line; they may mix with semantic hunks but must not overlap semantic ranges (SPEC 10.6).
- Macros: plain call patterns never enter macro bodies; `# macro=token_tree` renames verbatim (`M!(...)`→`N!(...)`) or binds one hole per top-level arg; shared-enclosing lines raise as one sweep (SPEC 21).

## CLI: apply (`gitrs apply` = `applyrs`, SPEC D.1/D.2)

```sh
cargo build -p applyrs -p diffrs -p gitrs # gitrs re-execs target/debug siblings: rebuild first, else probes test a stale engine; then read target (1-based spans)
gitrs apply --file change.rsmpl --format json --diff-max-lines 0 --check
gitrs apply --file change.rsmpl --output full.patch   # full diff while preview stays small
gitrs apply --file change.rsmpl                       # apply for real
```

- Inputs: no input flags reads stdin (`cat change.rsmpl | gitrs apply --check`; `--file -` is the explicit alias); `--file PATH` for large patches, `--patch TEXT` inline (conflicts with `--file`). `--repo DIR` (default `.`) selects the worktree — run from the root or pass it; patch targets must stay inside it (absolute/`..` escapes rejected, SPEC 27.6). Exit `0` = applied/clean, `2` = structured error.
- Flags: `--check` lowers + `git apply --check` without landing; `--format text|json` (default `text`); `--diff-max-lines N` (`0` omits) bounds only the reported diff; `--context N` renders reported/`--output` diffs with N context lines (verified lowering is always 0-context, SPEC D.2.2); `--output PATH` (cwd-relative) always writes the full rendered diff before `--check` runs, and removes a stale file on an empty lowering; `--verify build` additionally runs `cargo check` (temp copy under `--check`, `applied: false` on failure; post-apply otherwise, `applied: true`); `--bindings full` lifts the 80-char/8-entry evidence caps for that run; `--history` archives `--check` runs too (real applies always archive to `<repo>/.history/{id}.rsmpl`, last 128 kept).
- Read the JSON report, don't re-read files: `version` is the schema (currently `5`); `rules[]` carries `matches`, `spans` (original-source 1-based, one per emitted edit), per-site `sites` (`{span, bindings, origin}`; omitted when nothing binds), `orphans_total`/`follow_skipped_total`/`sites_total`, and per-rule `semantics` (`instantiated|postcondition|reparse-only|not-applicable`); `files[]` carries exact `{path, added, removed, post_sha}` even when `diff` is truncated; multi-edit rules add `span_edits` labels, cut values carry `binding_meta` digests, skipped follows carry `follow_skipped_details`, and stale ranges carry `drift`; `checked` is `{lowering, semantics: verified|partial, build: pass|skipped}` — `partial` means orphans, follow-skips, truncated evidence, ordinary hunks, or unverified edits remain, so verify `matches` equals the sites you read (fewer means a site, e.g. inside a macro, needs an ordinary hunk). Text mode prints the same verdicts as `check:`/`applied …; semantics …` on stderr; JSON success and failure are stdout-only (stderr keeps `warning:` lines only). A truncated stdout diff is review-only — never pipe it to `git apply` (SPEC D.2.1/D.2.3).
- Lint without a checkout: `--repo /tmp/empty --check` — `no match found` is the pass signal (SPEC D.2.4); re-check against the real repo after.

## CLI: diff (`gitrs diff` = `diffrs`, SPEC D.4)

- `gitrs diff` prints pending worktree changes as RSmPL (stdout); `--staged` diffs index vs HEAD, `[REV]` (e.g. `HEAD~1`) diffs since a revision, `--repo DIR` selects the worktree. v1 compresses fn/item renames, slot/call/method sweeps, and whole-item moves; the rest stays ordinary — output always parses and round-trips through `gitrs apply --check` against the pre-image. Binaries and submodule (gitlink) sections are stripped and `.history/` skipped, each with a stderr `warning:`; empty changes print `(no changes)` on stderr with empty stdout (never pipe that into `applyrs` — empty input is rejected).
- Raise instead of collecting: `--patch TEXT` / `--file PATH` convert native patch text (e.g. `git diff` output, a saved PR patch) against `--repo`'s pre-image without mutating the repo (`--staged`/`[REV]` still select the base). `--check` verifies the raise in-process (T361 byte-compare of scratch-applied post-tree; JSON gains `check: {verified, files_compared, first_divergence}`, text prints the verdict on stderr; failure exits 2 naming file + line). `--format json` returns `{version, files, patch, hunks, pre_pass_declined}` (`hunks[]` carries `detector`, `proven`, `totality`, `declined` trials for ordinary sections); text mode prints the patch on stdout and `declined:` provenance on stderr (`--declined summary|full`, default `summary`).

## Errors: code → fix (SPEC 25)

| Code | Fix |
|---|---|
| `RSMPL_RANGE_CONFLICT` | range exceeds file — read the file, fix the range |
| `RSMPL_PARSE_CONFLICT` | target file doesn't parse as Rust — read the file; empty patch — provide a `diff --git` section (empty input is rejected, not a no-op); differing `diff --git a/old b/new` paths are a file rename section, applied via old-side content (SPEC 6); count mismatch (`fix_header`) — if counts look right, check 1-char marker indent (an unprefixed line never enters the body); when the error names `suspect line N`, prefix line N with its marker and recount — `expected_header` is null there, never paste a header past the dropped line (SPEC 25.2) |
| `RSMPL_PATTERN_CONFLICT` | pattern doesn't parse — check SPEC 18.6 table; unknown directive (`# foo`, `# edition`, `# format_scope`) is rejected, check SPEC 9.3; unknown header tag (anything but `rule=`) is rejected, check SPEC 8.3; unknown `# root=` (only `item/expr/stmt/type`) is rejected, and `# root=item` needs a single declared metavar, check SPEC 13.4/18.6 C16; `# identifier/# path/# lifetime/# field` is rejected (`# ident` is the fn-header name-hole kind), check SPEC 15.1; `# repeat` needs a call/general-expr shape + bare spelling, check SPEC 17.8; item vis-prefix mismatch (`prefixes differ`) is rejected, check SPEC 14.8; definition name/type/`mut` mismatches fail fast naming the rename/bare spelling, check SPEC 14.8; `auto trait` headers rejected, check SPEC 14.8; lone `# fallback` without `# symbol` + non-default `# attrs=` on call/macro shapes rejected (C15); `# symbol` outside its legal shapes rejected (C14); same name under two kinds rejected, check SPEC 9.4; general template rewriting bound binder idents without binder identity rejected (C21 — multi-leaf destructuring renames need a single-binder spelling or an ordinary hunk; Stmt single-leaf destructuring completes like `let`; sequences exempt except a rename-once `let` rename (plain/destructuring, with authored init-substitution of later in-window uses, per-occurrence exclusion with hole-bound surfacing, outer-scope `new` declining follow with `capture-risk` orphans), which completes like `let`), check SPEC 18.6 |
| `RSMPL_MATCH_CONFLICT` | `no match found` (+ best-match-candidate preview in `reason`) — read the file, then fix range/pattern or add `# optional`; if the suggestion says exactly one match exists elsewhere, retry with `# scope=file` + `@@ -1,1 @@`; stale ranged hunk retries once file-scoped with `drift` evidence (terse: uniqueness gate; anchored: context must agree, else original conflict stands, SPEC 10.8); too many matches — add `# allow=many`; if the preview shows mixed `let-else`/macro-type, use an ordinary hunk (SPEC 18.6 R1/R2/R4/R5; fn-body `use` — SPEC 14.9); matcher partial overlap (`partially overlapping matches cannot be merged`, MATCH, rule+file, no suggestion, SPEC 17.6/25.4) vs plan-stage overlap (`overlapping edits cannot be merged`, GIT, file-only + suggestion, SPEC 10.6/25.7) |
| `RSMPL_SYMBOL_CONFLICT` | add `# fallback=syntax` for syntactic matching |
| `RSMPL_CAPTURE_CONFLICT` | `# use=Name` without matching `# bind=Name` anywhere in the patch (paste may precede cut, SPEC 20.3) — fix names |
| `GIT_HUNK_CONFLICT` | ordinary hunk failed — rebase the hunk context (the file line differs from the hunk text: paste `actual` from the repair or re-read the file; blank file lines are empty context: ` ` or empty line in-body) |
| `RSMPL_BUILD_CONFLICT` | post-lowering `--verify build` failure — read `repair.file` at `repair.range` and fix the build breakage (SPEC 25.10) |
| `RSMPL_IO_CONFLICT` | unreadable patch/repo/output file or bad flags (`--format json` → stdout envelope, else clap text) — check paths; absolute/`..`-escaping targets rejected (SPEC 27.6) |

<!-- generated: errors start -- do not hand-edit; run `python3 tools/gen_errors.py --write` -->
Error-code inventory (code is the source: `errors.rs`): 9 codes.
Codes: RSMPL_RANGE_CONFLICT, RSMPL_PARSE_CONFLICT, RSMPL_PATTERN_CONFLICT, RSMPL_MATCH_CONFLICT, RSMPL_SYMBOL_CONFLICT, RSMPL_CAPTURE_CONFLICT, GIT_HUNK_CONFLICT, RSMPL_IO_CONFLICT, RSMPL_BUILD_CONFLICT.
<!-- generated: errors end -->

JSON failures (`--format json`) print `{version, error: {code, rule, file, range, reason, suggestion, repair}}` to stdout only, exit 2. `repair` carries machine self-correction on every patch-input failure (SPEC D.2/25.9; paste `fix_range`/`fix_directive`/`fix_header`/`fix_context` verbatim, follow the `widen_scope`/`read_file`/`add_directive`/`fix_pattern`/`fix_name` template); `null` only on environmental IO.

Full contract: SPEC.md. One-page reference: AGENTS.md. Recommended loop: SPEC D.3 (read → author → check → apply).

